//! The VPS's view of the cluster: one SSH connection per verb, hardened option
//! set, secrets on stdin. Every call is a handshake on a shared login node, so
//! callers back off (§3).

use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::ids::{RunId, Token};
use crate::proc::{env_or, env_var, Exec};
use crate::report::EffRecord;
use crate::sacct::SacctRecord;
use crate::verbs::{JobId, Submit, Verb, REFUSED};

pub struct Cluster {
    user: String,
    host: String,
    key: String,
    known_hosts: String,
    ssh: String,
}

impl Cluster {
    pub fn from_env() -> Result<Self> {
        Ok(Cluster {
            user: env_var("CI_CLUSTER_USER")?,
            host: env_var("CI_CLUSTER_HOST")?,
            key: env_var("CI_SSH_KEY")?,
            known_hosts: env_var("CI_KNOWN_HOSTS")?,
            ssh: env_or("CI_SSH_BIN", "ssh"),
        })
    }

    fn call(&self, verb: &Verb, stdin: Option<Vec<u8>>) -> Result<String> {
        let mut exec = Exec::new(&self.ssh).args([
            "-i",
            &self.key,
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            &format!("UserKnownHostsFile={}", self.known_hosts),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=20",
            "-o",
            "ServerAliveInterval=15",
            &format!("{}@{}", self.user, self.host),
            &verb.command_line(),
        ]);
        if let Some(data) = stdin {
            exec = exec.stdin(data);
        }
        exec.output()
    }

    /// Three tries with growing gaps: a lost reply after a successful `sbatch`
    /// is recovered at job start via `HELLO`, not by resubmitting (§2).
    pub fn submit(&self, submit: &Submit, token: &Token) -> Result<JobId> {
        let verb = Verb::Submit(Box::new(submit.clone()));
        let mut last = None;
        for attempt in 1..=3u64 {
            match self.call(&verb, Some(format!("{token}\n").into_bytes())) {
                Ok(out) => return JobId::parse(out.trim()).context("submit reply"),
                Err(e) if e.to_string().contains(REFUSED) => return Err(e),
                Err(e) => {
                    eprintln!("submit attempt {attempt}: {e:#}");
                    last = Some(e);
                    sleep(Duration::from_secs(attempt * 10));
                }
            }
        }
        Err(last.expect("three failures").context("submit failed"))
    }

    pub fn status(&self, job: &JobId, run: &RunId) -> Result<Option<SacctRecord>> {
        let out = self.call(
            &Verb::Status {
                job: job.clone(),
                run: run.clone(),
            },
            None,
        )?;
        parse_optional_json(&out)
    }

    /// The tail of the Slurm output file (dispatch bounds it).
    pub fn log(&self, job: &JobId, run: &RunId) -> Result<Vec<u8>> {
        self.call(
            &Verb::Log {
                job: job.clone(),
                run: run.clone(),
            },
            None,
        )
        .map(String::into_bytes)
    }

    pub fn cancel(&self, job: &JobId, run: &RunId) -> Result<()> {
        self.call(
            &Verb::Cancel {
                job: job.clone(),
                run: run.clone(),
            },
            None,
        )
        .map(drop)
    }

    /// slurmdbd lands a beat after completion: tolerate empty replies for a
    /// couple of minutes before giving up on the record.
    pub fn eff(&self, job: &JobId, run: &RunId) -> Result<Option<EffRecord>> {
        let verb = Verb::Eff {
            job: job.clone(),
            run: run.clone(),
        };
        for _ in 0..6 {
            if let Some(rec) = parse_optional_json::<EffRecord>(&self.call(&verb, None)?)? {
                return Ok(Some(rec));
            }
            sleep(Duration::from_secs(20));
        }
        Ok(None)
    }

    pub fn rebake(&self) -> Result<JobId> {
        let out = self.call(&Verb::Rebake, None)?;
        JobId::parse(out.trim()).context("rebake reply")
    }
}

fn parse_optional_json<T: serde::de::DeserializeOwned>(out: &str) -> Result<Option<T>> {
    let line = out.trim();
    if line.is_empty() {
        return Ok(None);
    }
    if !line.starts_with('{') {
        bail!("unexpected dispatch reply {line:?}");
    }
    serde_json::from_str(line)
        .map(Some)
        .context("parse dispatch reply")
}
