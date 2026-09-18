//! The SSH verb surface (§4). The verb list is the capability grant: fixed
//! argv, per-field validation, secrets on stdin. `Verb::encode` and
//! `Verb::parse` are inverses, tested as such, so the VPS and the login node
//! cannot drift.

use std::fmt;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::ids::RunId;
use crate::spec::{JobSpec, TmpDir};
use crate::units::{MiB, Minutes};

/// Bumped when the argv shape changes; `submit` refuses a client it does not
/// speak, so a half-deployed upgrade fails at submit rather than in a namespace.
pub const PROTOCOL: &str = "v2";

/// Prefix of every dispatch-side rejection on stderr: the client must not
/// retry what dispatch refused on purpose.
pub const REFUSED: &str = "dispatch refused:";

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(String);

impl JobId {
    pub fn parse(s: &str) -> Result<Self> {
        if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) {
            Ok(JobId(s.to_owned()))
        } else {
            bail!("bad job id {s:?}");
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Submit {
    pub run: RunId,
    pub repo: String,
    pub git_host: String,
    pub git_ref: String,
    pub commit: String,
    pub callback_host: String,
    /// SHA-256 of the listener's DER cert, hex.
    pub fingerprint: String,
    pub spec: JobSpec,
    /// Gate retry: `--exclude` the node that refused the first attempt.
    pub exclude: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verb {
    Submit(Box<Submit>),
    Status { job: JobId, run: RunId },
    Log { job: JobId, run: RunId },
    Cancel { job: JobId, run: RunId },
    Eff { job: JobId, run: RunId },
    Rebake,
}

impl Verb {
    pub fn encode(&self) -> Vec<String> {
        let by_id = |name: &str, job: &JobId, run: &RunId| {
            vec![name.to_owned(), job.to_string(), run.to_string()]
        };
        match self {
            Verb::Submit(s) => {
                let mut v = vec![
                    "submit".to_owned(),
                    PROTOCOL.to_owned(),
                    s.run.to_string(),
                    s.repo.clone(),
                    s.git_host.clone(),
                    s.git_ref.clone(),
                    s.commit.clone(),
                    s.callback_host.clone(),
                    s.fingerprint.clone(),
                    s.spec.cores.to_string(),
                    s.spec.jobs.to_string(),
                    s.spec.mem.0.to_string(),
                    s.spec.time.0.to_string(),
                    s.spec.partition.clone(),
                    s.spec.constraint.clone().unwrap_or_else(|| "-".into()),
                    u8::from(s.spec.exclusive).to_string(),
                    s.spec.tmpdir.as_str().to_owned(),
                    s.exclude.clone().unwrap_or_else(|| "-".into()),
                    "--".to_owned(),
                ];
                v.extend(s.spec.installables.iter().cloned());
                v
            }
            Verb::Status { job, run } => by_id("status", job, run),
            Verb::Log { job, run } => by_id("log", job, run),
            Verb::Cancel { job, run } => by_id("cancel", job, run),
            Verb::Eff { job, run } => by_id("eff", job, run),
            Verb::Rebake => vec!["rebake".to_owned()],
        }
    }

    pub fn command_line(&self) -> String {
        self.encode().join(" ")
    }

    pub fn parse(argv: &[&str]) -> Result<Verb> {
        let by_id = |args: &[&str]| -> Result<(JobId, RunId)> {
            match args {
                [job, run] => Ok((JobId::parse(job)?, RunId::parse(run)?)),
                _ => bail!("expected <jobid> <run>"),
            }
        };
        match argv.split_first() {
            Some((&"submit", rest)) => Self::parse_submit(rest).map(|s| Verb::Submit(Box::new(s))),
            Some((&"status", rest)) => by_id(rest).map(|(job, run)| Verb::Status { job, run }),
            Some((&"log", rest)) => by_id(rest).map(|(job, run)| Verb::Log { job, run }),
            Some((&"cancel", rest)) => by_id(rest).map(|(job, run)| Verb::Cancel { job, run }),
            Some((&"eff", rest)) => by_id(rest).map(|(job, run)| Verb::Eff { job, run }),
            Some((&"rebake", [])) => Ok(Verb::Rebake),
            _ => bail!("rejected command {argv:?}"),
        }
    }

    fn parse_submit(rest: &[&str]) -> Result<Submit> {
        let sep = rest
            .iter()
            .position(|&t| t == "--")
            .context("submit: missing `--`")?;
        let (fixed, installables) = (&rest[..sep], &rest[sep + 1..]);
        let [protocol, run, repo, git_host, git_ref, commit, callback_host, fingerprint, cores, jobs, mem, time, partition, constraint, exclusive, tmpdir, exclude] =
            fixed
        else {
            bail!("submit: expected 17 fixed args, got {}", fixed.len());
        };
        if *protocol != PROTOCOL {
            bail!("submit: protocol {protocol:?}, this dispatch speaks {PROTOCOL}");
        }
        let opt = |s: &str| (s != "-").then(|| s.to_owned());
        let spec = JobSpec {
            installables: installables.iter().map(|s| (*s).to_owned()).collect(),
            cores: cores.parse().context("cores")?,
            jobs: jobs.parse().context("jobs")?,
            mem: MiB(mem.parse().context("mem")?),
            time: Minutes(time.parse().context("time")?),
            partition: (*partition).to_owned(),
            constraint: opt(constraint),
            exclusive: match *exclusive {
                "0" => false,
                "1" => true,
                _ => bail!("bad exclusive flag"),
            },
            tmpdir: TmpDir::parse(tmpdir)?,
        };
        let submit = Submit {
            run: RunId::parse(run)?,
            repo: (*repo).to_owned(),
            git_host: (*git_host).to_owned(),
            git_ref: (*git_ref).to_owned(),
            commit: (*commit).to_owned(),
            callback_host: (*callback_host).to_owned(),
            fingerprint: (*fingerprint).to_owned(),
            spec,
            exclude: opt(exclude),
        };
        submit.validate()?;
        Ok(submit)
    }
}

impl Submit {
    pub fn validate(&self) -> Result<()> {
        check_repo(&self.repo)?;
        check_host(&self.git_host)?;
        check_ref(&self.git_ref)?;
        check_commit(&self.commit)?;
        check_host(&self.callback_host)?;
        if self.fingerprint.len() != 64 || !self.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("bad fingerprint");
        }
        if let Some(n) = &self.exclude {
            check_node(n)?;
        }
        self.spec.validate()?;
        Ok(())
    }
}

fn all(s: &str, what: &str, pred: impl Fn(char) -> bool) -> Result<()> {
    if s.is_empty() || !s.chars().all(pred) {
        bail!("bad {what} {s:?}");
    }
    Ok(())
}

pub fn check_repo(s: &str) -> Result<()> {
    let ok = matches!(s.split('/').collect::<Vec<_>>().as_slice(), [owner, name]
        if !owner.is_empty() && !name.is_empty() && !owner.starts_with('.') && !name.starts_with('.'))
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c));
    if !ok {
        bail!("bad repo {s:?}: expected owner/name");
    }
    Ok(())
}

pub fn check_host(s: &str) -> Result<()> {
    let (host, port) = s
        .rsplit_once(':')
        .filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or((s, ""));
    if !port.is_empty() {
        port.parse::<u16>()
            .with_context(|| format!("bad port in {s:?}"))?;
    }
    all(host, "host", |c| {
        c.is_ascii_alphanumeric() || ".-".contains(c)
    })?;
    if host.starts_with('-') || host.starts_with('.') {
        bail!("bad host {s:?}");
    }
    Ok(())
}

pub fn check_ref(s: &str) -> Result<()> {
    all(s, "ref", |c| {
        c.is_ascii_alphanumeric() || "._/-".contains(c)
    })?;
    if s.contains("..") || s.starts_with('-') || s.starts_with('/') {
        bail!("bad ref {s:?}");
    }
    Ok(())
}

pub fn check_commit(s: &str) -> Result<()> {
    if s.len() != 40 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("bad commit {s:?}: expected a full sha1");
    }
    Ok(())
}

pub fn check_node(s: &str) -> Result<()> {
    all(s, "node", |c| {
        c.is_ascii_alphanumeric() || "._-".contains(c)
    })?;
    if s.starts_with('-') {
        bail!("bad node {s:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample_submit() -> Submit {
        Submit {
            run: RunId::parse(&"ab".repeat(16)).unwrap(),
            repo: "ironmoon/yakery".into(),
            git_host: "git.ironmoon.dev".into(),
            git_ref: "refs/heads/master".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            callback_host: "157.151.229.227".into(),
            fingerprint: "cd".repeat(32),
            spec: JobSpec {
                installables: vec![
                    "packages.x86_64-linux.default".into(),
                    "checks.x86_64-linux.vm".into(),
                ],
                cores: 32,
                jobs: 4,
                mem: MiB(65536),
                time: Minutes(45),
                partition: "sharing".into(),
                constraint: Some("zen2|skylake_avx512".into()),
                exclusive: false,
                tmpdir: TmpDir::Disk,
            },
            exclude: Some("d0020".into()),
        }
    }

    fn round_trip(v: Verb) {
        let line = v.command_line();
        let argv: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(Verb::parse(&argv).unwrap(), v, "{line}");
    }

    #[test]
    fn verbs_round_trip() {
        round_trip(Verb::Submit(Box::new(sample_submit())));
        let mut plain = sample_submit();
        plain.spec.constraint = None;
        plain.exclude = None;
        round_trip(Verb::Submit(Box::new(plain)));
        let job = JobId::parse("9442675").unwrap();
        let run = RunId::parse(&"ab".repeat(16)).unwrap();
        round_trip(Verb::Status {
            job: job.clone(),
            run: run.clone(),
        });
        round_trip(Verb::Log {
            job: job.clone(),
            run: run.clone(),
        });
        round_trip(Verb::Cancel {
            job: job.clone(),
            run: run.clone(),
        });
        round_trip(Verb::Eff { job, run });
        round_trip(Verb::Rebake);
    }

    #[test]
    fn rejects() {
        let ok = Verb::Submit(Box::new(sample_submit())).command_line();
        let argv = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        for bad in [
            ok.replace("submit v2", "submit v1"),
            ok.replace("ironmoon/yakery", "ironmoon"),
            ok.replace("ironmoon/yakery", "../etc"),
            ok.replace("refs/heads/master", "refs/../x"),
            ok.replace("0123456789abcdef0123456789abcdef01234567", "0123456"),
            ok.replace("zen2|skylake_avx512", "zen3"),
            ok.replace(" 45 ", " 61 "),
            ok.replace("-- packages", "-- --help"),
            ok.replace("-- packages", "-- github:a/b#c"),
            ok.replace("sharing", "gpu"),
            "status 123".into(),
            "status abc ab".into(),
            "rebake now".into(),
            "sh -c id".into(),
            String::new(),
        ] {
            let v = argv(&bad);
            let refs: Vec<&str> = v.iter().map(String::as_str).collect();
            assert!(Verb::parse(&refs).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn hosts() {
        assert!(check_host("git.ironmoon.dev").is_ok());
        assert!(check_host("157.151.229.227").is_ok());
        assert!(check_host("host:8443").is_ok());
        assert!(check_host("host:99999").is_err());
        assert!(check_host("-x").is_err());
        assert!(check_host("a b").is_err());
    }
}
