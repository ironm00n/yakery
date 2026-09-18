//! `~/ci/run-<run>/`: everything one attempt needs, written by `dispatch`
//! (0700), consumed by `build`, reaped by the next `submit` (§4).
//!
//! ```text
//! job.json    the Submit, verbatim (public — it is also on the sbatch line)
//! cbtoken     the callback token, 0600; unlinked by build at job start
//! jobid       written after sbatch; what the verbs check identity against
//! job.out     Slurm's output file, the authoritative log
//! build.json  BuildReport, written at teardown
//! done        marker: eff record served, dir eligible for reaping
//! ```

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::ids::{RunId, Token};
use crate::verbs::{JobId, Submit};

pub const PREFIX: &str = "run-";
/// Finished runs kept for autopsy.
pub const KEEP_DONE: usize = 20;
/// A dir with no `jobid` this old is a submit that died mid-way.
pub const UNSUBMITTED_GRACE: Duration = Duration::from_secs(3600);

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct JobFile {
    pub submit: Submit,
    /// Reaper grace for node-local leftovers, derived at submit (§5).
    pub reap_grace_secs: u64,
}

pub struct RunDir {
    path: PathBuf,
}

impl RunDir {
    pub fn path_for(root: &Path, run: &RunId) -> PathBuf {
        root.join(format!("{PREFIX}{run}"))
    }

    /// Create fresh; refuses to reuse, so a run id is submitted at most once.
    pub fn create(root: &Path, job: &JobFile, token: &Token) -> Result<Self> {
        fs::create_dir_all(root)?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        let path = Self::path_for(root, &job.submit.run);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .with_context(|| format!("create {}", path.display()))?;
        let dir = RunDir { path };
        fs::write(dir.path.join("job.json"), serde_json::to_vec_pretty(job)?)?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dir.path.join("cbtoken"))?;
        writeln!(f, "{token}")?;
        Ok(dir)
    }

    pub fn open(root: &Path, run: &RunId) -> Result<Self> {
        let path = Self::path_for(root, run);
        if !path.is_dir() {
            bail!("no run dir for {run}");
        }
        Ok(RunDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn job(&self) -> Result<JobFile> {
        let bytes = fs::read(self.path.join("job.json")).context("read job.json")?;
        serde_json::from_slice(&bytes).context("parse job.json")
    }

    pub fn output_file(&self) -> PathBuf {
        self.path.join("job.out")
    }

    /// Read and unlink: on disk only during PENDING (§4).
    pub fn take_token(&self) -> Result<Token> {
        let p = self.path.join("cbtoken");
        let s = fs::read_to_string(&p).context("read cbtoken")?;
        fs::remove_file(&p)?;
        Token::parse(s.trim())
    }

    pub fn record_job(&self, job: &JobId) -> Result<()> {
        fs::write(self.path.join("jobid"), format!("{job}\n")).context("write jobid")
    }

    pub fn job_id(&self) -> Result<Option<JobId>> {
        match fs::read_to_string(self.path.join("jobid")) {
            Ok(s) => JobId::parse(s.trim()).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Verbs act only on our own jobs (§4): the job id must be the one this
    /// run dir recorded at submit.
    pub fn verify(root: &Path, run: &RunId, job: &JobId) -> Result<Self> {
        let dir = Self::open(root, run)?;
        match dir.job_id()? {
            Some(recorded) if &recorded == job => Ok(dir),
            Some(recorded) => bail!("job {job} is not run {run}'s job ({recorded})"),
            None => bail!("run {run} has no recorded job"),
        }
    }

    pub fn mark_done(&self) -> Result<()> {
        fs::write(self.path.join("done"), b"").context("write done")
    }

    fn is_done(&self) -> bool {
        self.path.join("done").exists()
    }

    fn age(&self) -> Duration {
        let newest = fs::read_dir(&self.path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().ok()?.modified().ok())
            .max()
            .or_else(|| fs::metadata(&self.path).ok()?.modified().ok());
        newest
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .unwrap_or(Duration::ZERO)
    }

    pub fn remove(self) -> Result<()> {
        fs::remove_dir_all(&self.path).with_context(|| format!("remove {}", self.path.display()))
    }
}

/// Reap on every submit so cancel-before-start, node crash, or VPS death
/// cannot accumulate orphans. `orphan_grace` covers queue deadline + walltime
/// + kill grace: a submitted run with no `done` older than that is dead.
pub fn prune(root: &Path, orphan_grace: Duration) -> Result<usize> {
    let mut done: Vec<(Duration, RunDir)> = Vec::new();
    let mut removed = 0;
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|n| n.strip_prefix(PREFIX)) else {
            continue;
        };
        if RunId::parse(id).is_err() || !entry.path().is_dir() {
            continue;
        }
        let dir = RunDir { path: entry.path() };
        let age = dir.age();
        if dir.is_done() {
            done.push((age, dir));
        } else if dir.job_id()?.is_none() {
            if age > UNSUBMITTED_GRACE {
                dir.remove()?;
                removed += 1;
            }
        } else if age > orphan_grace {
            dir.remove()?;
            removed += 1;
        }
    }
    done.sort_by_key(|(age, _)| *age);
    for (_, dir) in done.into_iter().skip(KEEP_DONE) {
        dir.remove()?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{JobSpec, TmpDir};
    use crate::units::{MiB, Minutes};

    fn job(run: RunId) -> JobFile {
        JobFile {
            submit: Submit {
                run,
                repo: "o/r".into(),
                git_host: "h".into(),
                git_ref: "refs/heads/m".into(),
                commit: "0".repeat(40),
                callback_host: "c".into(),
                fingerprint: "0".repeat(64),
                spec: JobSpec {
                    installables: vec!["a".into()],
                    cores: 1,
                    jobs: 1,
                    mem: MiB(1),
                    time: Minutes(1),
                    partition: "sharing".into(),
                    constraint: None,
                    exclusive: false,
                    tmpdir: TmpDir::Ram,
                },
                exclude: None,
            },
            reap_grace_secs: 10,
        }
    }

    fn backdate(path: &Path, secs: u64) {
        let t = SystemTime::now() - Duration::from_secs(secs);
        let ft = std::fs::FileTimes::new().set_modified(t);
        for e in fs::read_dir(path).unwrap().flatten() {
            fs::File::options()
                .write(true)
                .open(e.path())
                .unwrap()
                .set_times(ft)
                .unwrap();
        }
        fs::File::open(path).unwrap().set_times(ft).unwrap();
    }

    #[test]
    fn lifecycle_and_identity() {
        let root = tempfile::tempdir().unwrap();
        let run = RunId::random().unwrap();
        let token = Token::random().unwrap();
        let dir = RunDir::create(root.path(), &job(run.clone()), &token).unwrap();
        assert!(RunDir::create(root.path(), &job(run.clone()), &token).is_err());
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(dir.path().join("cbtoken"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(dir.job().unwrap().submit.run, run);
        assert_eq!(dir.job_id().unwrap(), None);

        let jid = JobId::parse("77").unwrap();
        assert!(RunDir::verify(root.path(), &run, &jid).is_err());
        dir.record_job(&jid).unwrap();
        assert!(RunDir::verify(root.path(), &run, &jid).is_ok());
        assert!(RunDir::verify(root.path(), &run, &JobId::parse("78").unwrap()).is_err());
        assert!(RunDir::verify(root.path(), &RunId::random().unwrap(), &jid).is_err());

        assert!(dir.take_token().unwrap().matches(&token));
        assert!(dir.take_token().is_err());
    }

    #[test]
    fn prune_policy() {
        let root = tempfile::tempdir().unwrap();
        let mk = |secs: u64, jobid: bool, done: bool| {
            let run = RunId::random().unwrap();
            let d =
                RunDir::create(root.path(), &job(run.clone()), &Token::random().unwrap()).unwrap();
            if jobid {
                d.record_job(&JobId::parse("1").unwrap()).unwrap();
            }
            if done {
                d.mark_done().unwrap();
            }
            backdate(d.path(), secs);
            run
        };
        let fresh_unsubmitted = mk(10, false, false);
        let stale_unsubmitted = mk(4000, false, false);
        let live = mk(4000, true, false);
        let orphan = mk(100_000, true, false);
        let done_recent = mk(50, true, true);
        let mut done_old: Vec<RunId> = (0..KEEP_DONE + 2)
            .map(|i| mk(1000 + i as u64, true, true))
            .collect();

        let removed = prune(root.path(), Duration::from_secs(50_000)).unwrap();
        assert_eq!(removed, 1 + 1 + 3);
        let exists = |r: &RunId| RunDir::path_for(root.path(), r).is_dir();
        assert!(exists(&fresh_unsubmitted));
        assert!(!exists(&stale_unsubmitted));
        assert!(exists(&live));
        assert!(!exists(&orphan));
        assert!(exists(&done_recent));
        let oldest = done_old.split_off(KEEP_DONE - 1);
        assert!(done_old.iter().all(exists));
        assert!(oldest.iter().all(|r| !exists(r)));
    }
}
