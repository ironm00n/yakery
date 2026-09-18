//! What a job asks for. `RepoConfig` is the repo's `.slurm-ci.toml`; `JobSpec`
//! is its canonical, validated form, and `caps` is the dispatch-side contract
//! with the account whose fair-share is spent (§2, §8). Both sides of the SSH
//! verb run the same checks — one binary — so a bad spec fails on the VPS
//! first and cannot pass dispatch by accident.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::units::{MiB, Minutes};

pub const CONFIG_PATH: &str = ".slurm-ci.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    /// Fragments of the fetched tree's own flake: `packages.x86_64-linux.default`.
    pub installables: Vec<String>,
    #[serde(default = "default_cores")]
    pub cores: u32,
    /// nix `max-jobs`; `cores` is per job.
    #[serde(default = "default_jobs")]
    pub jobs: u32,
    #[serde(default = "default_mem")]
    pub mem: String,
    #[serde(default = "default_time")]
    pub time: String,
    #[serde(default = "default_partition")]
    pub partition: String,
    pub constraint: Option<String>,
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default)]
    pub tmpdir: TmpDir,
}

fn default_cores() -> u32 {
    32
}
fn default_jobs() -> u32 {
    4
}
fn default_mem() -> String {
    "64G".into()
}
fn default_time() -> String {
    "45m".into()
}
fn default_partition() -> String {
    "sharing".into()
}

/// Where `TMPDIR` points inside the job (§8 hero mode): tmpfs bills `--mem`,
/// the node-local disk does not.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TmpDir {
    #[default]
    Ram,
    Disk,
}

impl TmpDir {
    pub fn as_str(self) -> &'static str {
        match self {
            TmpDir::Ram => "ram",
            TmpDir::Disk => "disk",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "ram" => Ok(TmpDir::Ram),
            "disk" => Ok(TmpDir::Disk),
            _ => bail!("bad tmpdir {s:?}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub installables: Vec<String>,
    pub cores: u32,
    pub jobs: u32,
    pub mem: MiB,
    pub time: Minutes,
    pub partition: String,
    pub constraint: Option<String>,
    pub exclusive: bool,
    pub tmpdir: TmpDir,
}

impl JobSpec {
    pub fn from_toml(text: &str) -> Result<Self> {
        let cfg: RepoConfig =
            toml::from_str(text).with_context(|| format!("parse {CONFIG_PATH}"))?;
        let spec = JobSpec {
            installables: cfg.installables,
            cores: cfg.cores,
            jobs: cfg.jobs,
            mem: MiB::parse(&cfg.mem)?,
            time: Minutes::parse(&cfg.time)?,
            partition: cfg.partition,
            constraint: cfg.constraint,
            exclusive: cfg.exclusive,
            tmpdir: cfg.tmpdir,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Syntax and caps. Called by the VPS before submit and by dispatch before
    /// `sbatch`; the second call is the one that counts.
    pub fn validate(&self) -> Result<&'static PartitionCap> {
        if self.installables.is_empty() {
            bail!("no installables");
        }
        for inst in &self.installables {
            check_installable(inst)?;
        }
        if let Some(c) = &self.constraint {
            check_constraint(c)?;
        }
        if self.cores == 0 || self.jobs == 0 {
            bail!("cores and jobs must be positive");
        }
        let cap = caps::partition(&self.partition)?;
        if self.cores > cap.max_cores {
            bail!(
                "cores {} exceeds {} cap {}",
                self.cores,
                cap.name,
                cap.max_cores
            );
        }
        if self.mem > cap.max_mem {
            bail!("mem {} exceeds {} cap {}", self.mem, cap.name, cap.max_mem);
        }
        if self.time > cap.max_time {
            bail!(
                "time {} exceeds {} cap {}",
                self.time,
                cap.name,
                cap.max_time
            );
        }
        Ok(cap)
    }
}

pub struct PartitionCap {
    pub name: &'static str,
    pub max_cores: u32,
    pub max_mem: MiB,
    pub max_time: Minutes,
}

pub mod caps {
    use super::*;

    pub const PARTITIONS: &[PartitionCap] = &[
        PartitionCap {
            name: "sharing",
            max_cores: 128,
            max_mem: MiB(240 * 1024),
            max_time: Minutes(60),
        },
        PartitionCap {
            name: "short",
            max_cores: 128,
            max_mem: MiB(240 * 1024),
            max_time: Minutes(24 * 60),
        },
    ];

    /// Ceiling on our simultaneously queued/running jobs, counted by comment tag.
    pub const MAX_INFLIGHT: usize = 8;

    /// `sbatch --deadline` (§8): a job still PENDING after this is cancelled by
    /// Slurm, which bounds the run that never learns its job id.
    pub const QUEUE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

    /// `sinfo -h -N -o '%f'` census, 2026-08-21 (§10). Case-sensitive, and one
    /// bad token rejects the whole expression cluster-side.
    pub const CONSTRAINT_TOKENS: &[&str] = &[
        "a100@80g",
        "a30",
        "bansil",
        "broadwell",
        "cascadelake",
        "dgx",
        "haswell",
        "ib",
        "ivybridge",
        "largemem",
        "lotterhos",
        "prod",
        "prod8",
        "rocm",
        "sapphirerapids",
        "skylake_avx512",
        "xen2",
        "zen",
        "zen2",
        "zhang",
    ];

    pub fn partition(name: &str) -> Result<&'static PartitionCap> {
        PARTITIONS
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("partition {name:?} not in allowlist"))
    }

    pub fn partition_names() -> Vec<&'static str> {
        PARTITIONS.iter().map(|p| p.name).collect()
    }

    pub fn longest_walltime() -> Minutes {
        PARTITIONS
            .iter()
            .map(|p| p.max_time)
            .max()
            .expect("non-empty caps")
    }
}

/// A dotted attr path into the fetched tree's flake. Anything URL-shaped would
/// evaluate a flake that never went through review (§4).
pub fn check_installable(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !ok {
        bail!("bad installable {s:?}: expected a dotted attr path");
    }
    Ok(())
}

pub fn check_constraint(s: &str) -> Result<()> {
    if s.is_empty() {
        bail!("empty constraint");
    }
    for tok in s.split(['|', '&']) {
        if !caps::CONSTRAINT_TOKENS.contains(&tok) {
            bail!("constraint token {tok:?} not in allowlist");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_caps() {
        let spec =
            JobSpec::from_toml("installables = [\"packages.x86_64-linux.default\"]\n").unwrap();
        assert_eq!(spec.cores, 32);
        assert_eq!(spec.mem, MiB(65536));
        assert_eq!(spec.time, Minutes(45));
        assert_eq!(spec.partition, "sharing");
        assert_eq!(spec.tmpdir, TmpDir::Ram);
        assert!(!spec.exclusive);
        assert_eq!(spec.validate().unwrap().name, "sharing");
    }

    #[test]
    fn caps_reject() {
        let over = "installables = [\"a\"]\ntime = \"2h\"\n";
        assert!(JobSpec::from_toml(over)
            .unwrap_err()
            .to_string()
            .contains("exceeds sharing cap"));
        let short = "installables = [\"a\"]\ntime = \"2h\"\npartition = \"short\"\n";
        assert!(JobSpec::from_toml(short).is_ok());
        let part = "installables = [\"a\"]\npartition = \"gpu\"\n";
        assert!(JobSpec::from_toml(part)
            .unwrap_err()
            .to_string()
            .contains("allowlist"));
        let mem = "installables = [\"a\"]\nmem = \"1T\"\n";
        assert!(JobSpec::from_toml(mem).is_err());
        let unknown = "installables = [\"a\"]\nnodes = 2\n";
        assert!(JobSpec::from_toml(unknown).is_err());
        assert!(JobSpec::from_toml("installables = []\n").is_err());
    }

    #[test]
    fn installables() {
        assert!(check_installable("packages.x86_64-linux.default").is_ok());
        assert!(check_installable("checks.x86_64-linux.my-test_1").is_ok());
        for bad in [
            "",
            "-x",
            ".#foo",
            "github:a/b#c",
            "a b",
            "a/b",
            "path:/etc",
            "a\n",
        ] {
            assert!(check_installable(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn constraints() {
        assert!(check_constraint("cascadelake").is_ok());
        assert!(check_constraint("zen2|skylake_avx512").is_ok());
        assert!(check_constraint("ib&broadwell").is_ok());
        assert!(check_constraint("zen3").is_err());
        assert!(check_constraint("Cascadelake").is_err());
        assert!(check_constraint("ib&").is_err());
        assert!(check_constraint("").is_err());
    }
}
