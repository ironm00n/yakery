//! Records that cross hosts as JSON: what `build` measured at teardown, what
//! `eff` merges with `sacct`, and the JSONL line the VPS keeps (§9).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::ids::RunId;
use crate::sacct::SacctRecord;
use crate::verbs::JobId;
use crate::verdict::Verdict;

/// Written by `build` into the run dir at teardown. Node-local disk is the one
/// resource Slurm does not account and the one this design consumes hardest.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct BuildReport {
    pub node: String,
    pub bootstrap_version: String,
    pub exit: i32,
    pub fetch_secs: u64,
    pub build_secs: u64,
    pub push_secs: u64,
    pub upper_bytes: u64,
    pub src_bytes: u64,
    pub xtmp_bytes: u64,
    pub pushed_paths: usize,
}

impl BuildReport {
    pub const FILE: &'static str = "build.json";

    pub fn write(&self, run_dir: &Path) -> Result<()> {
        let path = run_dir.join(Self::FILE);
        fs::write(&path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("write {}", path.display()))
    }

    pub fn read(run_dir: &Path) -> Result<Option<Self>> {
        match fs::read(run_dir.join(Self::FILE)) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("parse build.json")?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// The `eff` verb's reply: Slurm's accounting plus the build's own numbers.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EffRecord {
    pub sacct: SacctRecord,
    pub build: Option<BuildReport>,
}

/// One JSONL line per attempt on the VPS. Timestamps in `sacct` are Slurm's,
/// so VPS/cluster clock skew cannot distort the record.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct JsonlRecord {
    pub run: RunId,
    pub attempt: u32,
    pub repo: String,
    pub git_ref: String,
    pub commit: String,
    pub job: Option<JobId>,
    pub verdict: Verdict,
    pub verdict_via: String,
    pub submitted_at: u64,
    pub finished_at: u64,
    pub eff: Option<EffRecord>,
    /// §9: on this cluster `--mem` is not obviously enforced, so an overshoot
    /// is the one finding that harms someone other than us.
    pub mem_overshoot: bool,
}

/// `MaxRSS` vs `ReqMem`, both in Slurm's own units (`183917204K`, `126000M`).
pub fn mem_overshoot(rec: &SacctRecord) -> bool {
    match (
        rec.max_rss.as_deref().and_then(slurm_bytes),
        rec.req_mem.as_deref().and_then(slurm_bytes),
    ) {
        (Some(rss), Some(req)) => rss > req,
        _ => false,
    }
}

fn slurm_bytes(s: &str) -> Option<u64> {
    let s = s.trim_end_matches(['n', 'c']);
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = num.parse().ok()?;
    let mult = match unit {
        "" => 1,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        "T" => 1 << 40,
        _ => return None,
    };
    Some(n * mult)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overshoot_from_job_9442675() {
        let rec = SacctRecord {
            max_rss: Some("183917204K".into()),
            req_mem: Some("126000M".into()),
            ..Default::default()
        };
        assert!(mem_overshoot(&rec));
        let fine = SacctRecord {
            max_rss: Some("1000K".into()),
            req_mem: Some("16000Mn".into()),
            ..Default::default()
        };
        assert!(!mem_overshoot(&fine));
        assert!(!mem_overshoot(&SacctRecord::default()));
    }

    #[test]
    fn build_report_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(BuildReport::read(dir.path()).unwrap(), None);
        let r = BuildReport {
            node: "d0020".into(),
            exit: 76,
            pushed_paths: 3,
            ..Default::default()
        };
        r.write(dir.path()).unwrap();
        assert_eq!(BuildReport::read(dir.path()).unwrap(), Some(r));
    }
}
