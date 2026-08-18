//! `slurm-ci rebake`: the maintenance job that materialises the next
//! bootstrap store (§6). TCB: zero arguments, the flake ref is pinned in
//! `~/ci/bootstrap.pin`, and the flip is `symlink` + `rename()` so no job ever
//! observes a missing lower.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{bail, Context, Result};

use crate::cluster::build::{install_term_forwarding, run_relayed, self_exe, Sink, Workspace};
use crate::cluster::ns;
use crate::cluster::site::{self, CiDir, BOOTSTRAP_LINK, BOOTSTRAP_PREFIX, CACHE_DIR, PROJECT_DIR};

pub fn main() -> Result<i32> {
    let ci = CiDir::from_env()?;
    let pin = fs::read_to_string(ci.bootstrap_pin())
        .with_context(|| format!("read {}", ci.bootstrap_pin().display()))?
        .trim()
        .to_owned();
    if pin.is_empty() {
        bail!("bootstrap.pin is empty");
    }
    let (lower, version) = site::resolve_bootstrap()?;
    let next: u32 = version
        .parse::<u32>()
        .with_context(|| format!("bootstrap version {version:?}"))?
        + 1;
    let candidate = Path::new(PROJECT_DIR).join(format!("{BOOTSTRAP_PREFIX}{next}"));
    fs::create_dir(&candidate).with_context(|| format!("create {}", candidate.display()))?;
    install_term_forwarding()?;
    let sink = Sink { client: None };
    sink.line(format!("rebake: v{version} -> v{next} from {pin}"));

    let job = std::env::var("SLURM_JOB_ID").unwrap_or_else(|_| "manual".into());
    let ws = Workspace::create(&format!("rebake-{job}"))?;
    let mut build = self_exe();
    build
        .arg("__ns")
        .arg("__build")
        .arg("--root")
        .arg(ws.dir("root"))
        .arg("--bootstrap")
        .arg(&lower)
        .arg("--upper")
        .arg(ws.dir("upper"))
        .arg("--work")
        .arg(ws.dir("work"))
        .arg("--src")
        .arg(ws.dir("src"))
        .arg("--xtmp")
        .arg(ws.dir("xtmp"))
        .arg("--cache")
        .arg(CACHE_DIR)
        .arg("--key")
        .arg(ci.cache_key())
        .arg("--pubkey")
        .arg(fs::read_to_string(ci.cache_pub())?.trim())
        .arg("--cores")
        .arg("8")
        .arg("--jobs")
        .arg("2")
        .arg("--tmpdir")
        .arg("disk")
        .arg("--rebake-candidate")
        .arg(&candidate)
        .arg("--")
        .arg(&pin)
        .env_clear();
    let code = run_relayed(build, &sink)?;
    ws.remove();
    if code != 0 {
        sink.line(format!(
            "rebake build failed ({code}); discarding candidate"
        ));
        let _ = ns::force_remove(&candidate);
        return Ok(code);
    }

    let tmp = Path::new(PROJECT_DIR).join(".bootstrap-store.tmp");
    let _ = fs::remove_file(&tmp);
    symlink(format!("{BOOTSTRAP_PREFIX}{next}"), &tmp)?;
    fs::rename(&tmp, BOOTSTRAP_LINK).context("flip bootstrap symlink")?;
    sink.line(format!("rebake: {BOOTSTRAP_LINK} -> v{next}"));

    let cutoff = SystemTime::now() - site::reap_grace();
    for entry in fs::read_dir(PROJECT_DIR)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(v) = name.strip_prefix(BOOTSTRAP_PREFIX) else {
            continue;
        };
        if v == next.to_string() {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|m| m < cutoff)
            .unwrap_or(false);
        if old_enough {
            sink.line(format!("rebake: reaping {name}"));
            if let Err(e) = ns::force_remove(&entry.path()) {
                sink.line(format!("rebake: {e:#}"));
            }
        }
    }
    Ok(0)
}
