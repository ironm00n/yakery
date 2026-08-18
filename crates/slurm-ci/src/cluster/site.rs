//! Everything Explorer-specific on the cluster side (§10). Porting = a new
//! version of this file.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::proc::{env_var, Exec};
use crate::spec::caps;
use crate::units::Minutes;

pub const PROJECT_DIR: &str = "/projects/dbp";
pub const CACHE_DIR: &str = "/projects/dbp/nix-cache";
pub const CACHE_KEY_NAME: &str = "dbp-ci-1";
/// `rename()`-flipped symlink to `bootstrap-store.vN` (§6).
pub const BOOTSTRAP_LINK: &str = "/projects/dbp/bootstrap-store";
pub const BOOTSTRAP_PREFIX: &str = "bootstrap-store.v";
/// Node-local scratch; the sanity gate refuses it if it is not local (§5).
pub const NODE_TMP: &str = "/tmp";
pub const NODE_TMP_FLOOR_BYTES: u64 = 50 << 30;
/// `KillWait=600` on Explorer, plus scheduler slack.
pub const KILL_GRACE: Duration = Duration::from_secs(600 + 600);

/// `~/ci`: the cluster user's private CI state. Nothing store-shaped lives in
/// `/home` (inode cap), only small files.
pub struct CiDir {
    root: PathBuf,
}

impl CiDir {
    pub fn from_env() -> Result<Self> {
        Ok(CiDir {
            root: PathBuf::from(env_var("HOME")?).join("ci"),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn runs_root(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn cache_key(&self) -> PathBuf {
        self.root.join("cache-priv.pem")
    }

    pub fn cache_pub(&self) -> PathBuf {
        self.root.join("cache-pub.pem")
    }

    /// The rebake's pinned flake ref, one line; TCB, set by hand (§6).
    pub fn bootstrap_pin(&self) -> PathBuf {
        self.root.join("bootstrap.pin")
    }

    pub fn bin(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(env_var("HOME")?).join("bin").join("slurm-ci"))
    }
}

/// How long anything of ours may legitimately live: the longest walltime any
/// allowed partition grants, `sinfo`-derived so a partition change cannot
/// silently break the ESTALE argument (§6), floored by our own caps.
pub fn max_walltime() -> Minutes {
    let ours = caps::longest_walltime();
    let partitions = caps::partition_names().join(",");
    let sinfo = Exec::new("sinfo")
        .args(["-h", "-p", &partitions, "-o", "%l"])
        .output();
    match sinfo {
        Ok(out) => out
            .lines()
            .filter_map(|l| Minutes::parse_sinfo(l).ok().flatten())
            .chain(std::iter::once(ours))
            .max()
            .unwrap_or(ours),
        Err(e) => {
            eprintln!("sinfo failed ({e:#}); using cap-derived walltime {ours}");
            ours
        }
    }
}

pub fn reap_grace() -> Duration {
    Duration::from_secs(max_walltime().0 * 60) + KILL_GRACE
}

/// Resolve the bootstrap symlink at job start, never at submit (§6).
pub fn resolve_bootstrap() -> Result<(PathBuf, String)> {
    let target =
        std::fs::read_link(BOOTSTRAP_LINK).with_context(|| format!("readlink {BOOTSTRAP_LINK}"))?;
    let dir = if target.is_absolute() {
        target
    } else {
        Path::new(PROJECT_DIR).join(target)
    };
    let version = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(BOOTSTRAP_PREFIX))
        .with_context(|| {
            format!(
                "{BOOTSTRAP_LINK} points at {}, not a {BOOTSTRAP_PREFIX}N dir",
                dir.display()
            )
        })?
        .to_owned();
    Ok((dir, version))
}
