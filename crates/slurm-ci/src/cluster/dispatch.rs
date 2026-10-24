//! `slurm-ci dispatch`: the SSH forced-command entry on the login node.
//! `$SSH_ORIGINAL_COMMAND` is untrusted argv, never a shell; the callback
//! token arrives on stdin. Every verb re-validates and acts only on jobs this
//! account's run dirs recorded (§4). Nothing here may stay resident.

use std::io::{Read, Write};

use anyhow::{bail, Context, Result};

use crate::cluster::rundir::{self, JobFile, RunDir};
use crate::cluster::site::{self, CiDir};
use crate::ids::Token;
use crate::proc::{env_var, Exec};
use crate::report::{BuildReport, EffRecord};
use crate::sacct::{SacctRecord, FIELDS};
use crate::spec::caps;
use crate::verbs::{JobId, Submit, Verb, REFUSED};

pub const COMMENT_PREFIX: &str = "slurm-ci:";
/// The last 4 MiB of `job.out` is what the fallback path ships to Forgejo.
const LOG_TAIL: usize = 4 << 20;

pub fn main() -> Result<i32> {
    match dispatch() {
        Ok(()) => Ok(0),
        Err(e) => {
            eprintln!("{REFUSED} {e:#}");
            Ok(2)
        }
    }
}

fn dispatch() -> Result<()> {
    let orig = std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default();
    let argv: Vec<&str> = orig.split_whitespace().collect();
    let ci = CiDir::from_env()?;
    match Verb::parse(&argv)? {
        Verb::Submit(s) => {
            let token = read_stdin_token()?;
            let job = submit(&ci, &s, &token)?;
            println!("{job}");
        }
        Verb::Status { job, run } => {
            RunDir::verify(&ci.runs_root(), &run, &job)?;
            if let Some(rec) = sacct(&job)? {
                println!("{}", serde_json::to_string(&rec)?);
            }
        }
        Verb::Log { job, run } => {
            let dir = RunDir::verify(&ci.runs_root(), &run, &job)?;
            let data = std::fs::read(dir.output_file()).unwrap_or_default();
            let tail = &data[data.len().saturating_sub(LOG_TAIL)..];
            std::io::stdout().write_all(tail)?;
        }
        Verb::Cancel { job, run } => {
            RunDir::verify(&ci.runs_root(), &run, &job)?;
            Exec::new("scancel").arg(job.as_str()).output()?;
        }
        Verb::Eff { job, run } => {
            let dir = RunDir::verify(&ci.runs_root(), &run, &job)?;
            if let Some(sacct) = sacct(&job)? {
                let rec = EffRecord {
                    build: BuildReport::read(dir.path())?,
                    sacct,
                };
                println!("{}", serde_json::to_string(&rec)?);
                dir.mark_done()?;
            }
        }
        Verb::Rebake => {
            let job = submit_rebake(&ci)?;
            println!("{job}");
        }
    }
    Ok(())
}

fn submit(ci: &CiDir, s: &Submit, token: &Token) -> Result<JobId> {
    let account =
        env_var("CI_SLURM_ACCOUNT").context("account must be pinned in the forced-command line")?;
    let cap = s.spec.validate()?;
    let reap_grace = site::reap_grace();
    rundir::prune(&ci.runs_root(), caps::QUEUE_DEADLINE + reap_grace).context("prune run dirs")?;
    let inflight = count_inflight()?;
    if inflight >= caps::MAX_INFLIGHT {
        bail!(
            "{inflight} slurm-ci jobs already queued or running (cap {})",
            caps::MAX_INFLIGHT
        );
    }

    let job = JobFile {
        submit: s.clone(),
        reap_grace_secs: reap_grace.as_secs(),
    };
    let dir = RunDir::create(&ci.runs_root(), &job, token)?;
    let bin = ci.bin()?;
    let mut args: Vec<String> = vec![
        "--parsable".into(),
        "--job-name=slurm-ci".into(),
        format!("--comment={COMMENT_PREFIX}{}", s.run),
        format!("--account={account}"),
        format!("--partition={}", cap.name),
        "--nodes=1".into(),
        format!("--cpus-per-task={}", s.spec.cores),
        format!("--mem={}", s.spec.mem.slurm()),
        format!("--time={}", s.spec.time.0),
        format!(
            "--deadline=now+{}minutes",
            caps::QUEUE_DEADLINE.as_secs() / 60
        ),
        format!("--output={}", dir.output_file().display()),
    ];
    if let Some(c) = &s.spec.constraint {
        args.push(format!("--constraint={c}"));
    }
    if s.spec.exclusive {
        args.push("--exclusive".into());
    }
    if let Some(n) = &s.exclude {
        args.push(format!("--exclude={n}"));
    }
    args.push(format!("--wrap={} build {}", bin.display(), s.run));

    let out = match Exec::new("sbatch").args(&args).output() {
        Ok(out) => out,
        Err(e) => {
            let _ = dir.remove();
            return Err(e);
        }
    };
    let job_id =
        JobId::parse(out.trim().split(';').next().unwrap_or("")).context("sbatch reply")?;
    dir.record_job(&job_id)?;
    Ok(job_id)
}

/// Our queued/running jobs, by comment tag. One `squeue` per submit.
fn count_inflight() -> Result<usize> {
    let out = Exec::new("squeue")
        .args(["--me", "-h", "-o", "%k"])
        .output()?;
    Ok(out
        .lines()
        .filter(|l| l.trim().starts_with(COMMENT_PREFIX))
        .count())
}

fn sacct(job: &JobId) -> Result<Option<SacctRecord>> {
    let out = Exec::new("sacct")
        .args(["-j", job.as_str(), "-n", "-P", "-o", FIELDS])
        .output()?;
    SacctRecord::parse(job.as_str(), &out)
}

/// Rebake is TCB (§6): zero arguments, content pinned cluster-side,
/// `--dependency=singleton` so two can never race the symlink flip.
fn submit_rebake(ci: &CiDir) -> Result<JobId> {
    let account =
        env_var("CI_SLURM_ACCOUNT").context("account must be pinned in the forced-command line")?;
    let bin = ci.bin()?;
    let out_dir = ci.root().join("rebake");
    std::fs::create_dir_all(&out_dir)?;
    let out = Exec::new("sbatch")
        .args([
            "--parsable",
            "--job-name=slurm-ci-rebake",
            "--dependency=singleton",
            &format!("--account={account}"),
            "--partition=short",
            "--nodes=1",
            "--cpus-per-task=8",
            "--mem=32G",
            "--time=180",
            &format!("--output={}/%j.out", out_dir.display()),
            &format!("--wrap={} rebake", bin.display()),
        ])
        .output()?;
    JobId::parse(out.trim().split(';').next().unwrap_or("")).context("sbatch reply")
}

fn read_stdin_token() -> Result<Token> {
    let mut s = String::new();
    std::io::stdin()
        .take(1024)
        .read_to_string(&mut s)
        .context("read token from stdin")?;
    Token::parse(s.trim()).context("callback token on stdin")
}
