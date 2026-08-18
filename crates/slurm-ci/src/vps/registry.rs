//! The run registry: one JSON file per in-flight attempt, written before
//! anything else can fail, deleted once the JSONL line exists. Restoring a
//! stale registry would re-attach to jobs that no longer exist, so it is
//! deliberately not backed up (§9).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::ids::{RunId, Token};
use crate::report::JsonlRecord;
use crate::spec::JobSpec;
use crate::verbs::JobId;
use crate::verdict::Verdict;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub run: RunId,
    pub attempt: u32,
    pub repo: String,
    pub git_ref: String,
    pub commit: String,
    pub spec: JobSpec,
    /// Retired on the first valid `HELLO`.
    pub token: Token,
    pub claimed: bool,
    pub job: Option<JobId>,
    pub node: Option<String>,
    pub submitted_at: u64,
    pub cancelled: bool,
    /// Set once decided; a record with a verdict but still on disk is a
    /// finalize (log/eff/JSONL) that did not complete.
    pub verdict: Option<(Verdict, String)>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct StateDir {
    root: PathBuf,
}

impl StateDir {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for sub in ["runs", "tls"] {
            fs::create_dir_all(root.join(sub))
                .with_context(|| format!("create {}", root.join(sub).display()))?;
        }
        Ok(StateDir { root })
    }

    pub fn tls_dir(&self) -> PathBuf {
        self.root.join("tls")
    }

    fn record_path(&self, run: &RunId) -> PathBuf {
        self.root.join("runs").join(format!("{run}.json"))
    }

    pub fn save(&self, rec: &RunRecord) -> Result<()> {
        let path = self.record_path(&rec.run);
        let tmp = path.with_extension("json.tmp");
        write_private(&tmp, &serde_json::to_vec_pretty(rec)?)?;
        fs::rename(&tmp, &path).with_context(|| format!("rename {}", path.display()))
    }

    pub fn remove(&self, run: &RunId) -> Result<()> {
        match fs::remove_file(self.record_path(run)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn load_all(&self) -> Result<Vec<RunRecord>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.root.join("runs"))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|b| Ok(serde_json::from_slice(&b)?))
            {
                Ok(rec) => out.push(rec),
                Err(e) => eprintln!("registry: skipping {}: {e:#}", path.display()),
            }
        }
        out.sort_by_key(|r: &RunRecord| r.submitted_at);
        Ok(out)
    }

    pub fn append_jsonl(&self, rec: &JsonlRecord) -> Result<()> {
        let path = self.root.join("efficiency.jsonl");
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let mut line = serde_json::to_vec(rec)?;
        line.push(b'\n');
        f.write_all(&line)
            .with_context(|| format!("append {}", path.display()))
    }
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::TmpDir;
    use crate::units::{MiB, Minutes};

    fn record() -> RunRecord {
        RunRecord {
            run: RunId::random().unwrap(),
            attempt: 1,
            repo: "o/r".into(),
            git_ref: "refs/heads/master".into(),
            commit: "0".repeat(40),
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
            token: Token::random().unwrap(),
            claimed: false,
            job: None,
            node: None,
            submitted_at: now(),
            cancelled: false,
            verdict: None,
        }
    }

    #[test]
    fn save_load_remove() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(dir.path()).unwrap();
        let a = record();
        let mut b = record();
        b.submitted_at += 1;
        b.verdict = Some((Verdict::Green, "callback".into()));
        state.save(&b).unwrap();
        state.save(&a).unwrap();
        assert_eq!(state.load_all().unwrap(), vec![a.clone(), b.clone()]);
        state.remove(&a.run).unwrap();
        state.remove(&a.run).unwrap();
        assert_eq!(state.load_all().unwrap(), vec![b]);
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join("runs")).unwrap();
        assert!(mode.is_dir());
        for e in fs::read_dir(dir.path().join("runs")).unwrap() {
            assert_eq!(
                e.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
