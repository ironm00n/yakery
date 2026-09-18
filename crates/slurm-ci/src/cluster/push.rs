//! The push phase (§7), run by `__build` as namespace root after the untrusted
//! child has exited, with the overlay still mounted. Pushes only what this job
//! built, after the tamper checks the 2026-09-01 audit added: nothing in this
//! flow legitimately copies a lower store path up, and `nix copy` will happily
//! sign a rewritten NAR under the db's stale hash.

use std::fs::{self, File};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nix::mount::{mount, MsFlags};

use crate::cluster::sandbox::ToolEnv;

pub enum PushError {
    /// Red, never a warning.
    Tamper(String),
    /// Built fine, not cached: green with a warning.
    Push(anyhow::Error),
}

impl From<anyhow::Error> for PushError {
    fn from(e: anyhow::Error) -> Self {
        PushError::Push(e)
    }
}

/// Returns how many paths were pushed.
pub fn push_phase(
    tools: &ToolEnv,
    upper: &File,
    lower_store: &File,
    key: &File,
) -> Result<usize, PushError> {
    // The upper mirrors the lower's root, which is the bootstrap's `nix/`.
    let upper_store = fd_path(upper).join("store");
    let Ok(entries) = fs::read_dir(&upper_store) else {
        eprintln!("push: nothing new in the store");
        return Ok(0);
    };
    let lower = fd_path(lower_store);
    let mut tampered = Vec::new();
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if fs::symlink_metadata(lower.join(&name)).is_ok() {
            tampered.push(name);
        } else if looks_like_store_path(&name) {
            candidates.push(format!("/nix/store/{name}"));
        }
    }
    if !tampered.is_empty() {
        return Err(PushError::Tamper(format!(
            "lower store paths copied up: {}",
            tampered.join(" ")
        )));
    }

    let mut built = Vec::new();
    for path in &candidates {
        // Not every upper entry is a registered path (build temp dirs, a
        // failed build's leftovers): those are skipped, not pushed.
        let out = match tools
            .exec("nix")
            .args(["path-info", "--sigs", path])
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                eprintln!("push: skipping {path}: {e:#}");
                continue;
            }
        };
        if out.split_whitespace().skip(1).count() == 0 {
            built.push(path.clone());
        }
    }
    if built.is_empty() {
        eprintln!("push: no locally built paths");
        return Ok(0);
    }
    eprintln!("push: verifying {} built paths", built.len());
    if let Err(e) = tools
        .exec("nix")
        .args(["store", "verify", "--no-trust"])
        .args(&built)
        .output()
    {
        return Err(PushError::Tamper(format!("nix store verify failed: {e:#}")));
    }

    // `/run` is the outer namespace's private tmpfs; the key exists there only
    // once the untrusted child is gone.
    fs::create_dir_all("/run/push").context("mkdir /run/push")?;
    let key_path = PathBuf::from("/run/push/key");
    let mut key_bytes = Vec::new();
    std::io::Read::read_to_end(&mut &*key, &mut key_bytes).context("read signing key")?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&key_path)
        .context("create /run/push/key")?
        .write_all(&key_bytes)
        .context("write /run/push/key")?;
    remount_rw("/cache")?;

    tools
        .exec("nix")
        .args(["store", "sign", "--key-file", "/run/push/key"])
        .args(&built)
        .output()
        .context("nix store sign")?;
    tools
        .exec("nix")
        .args([
            "copy",
            "--no-recursive",
            "--to",
            "file:///cache?compression=zstd",
        ])
        .args(&built)
        .output()
        .context("nix copy")?;
    eprintln!("push: {} paths pushed", built.len());
    Ok(built.len())
}

/// Rebake (§6, TCB): copy the built env's closure into the candidate lower,
/// point its `env` symlink at it, and seed the cache so the next job can
/// substitute it.
pub fn rebake_publish(
    tools: &ToolEnv,
    env_path: &str,
    candidate_mount: &Path,
    candidate_host: &Path,
) -> Result<()> {
    let store = format!("local?root={}", candidate_mount.display());
    tools
        .exec("nix")
        .args(["copy", "--to", &store, env_path])
        .output()
        .context("nix copy to candidate")?;
    std::os::unix::fs::symlink(env_path, candidate_mount.join("env"))
        .context("candidate env symlink")?;
    tools
        .exec("nix")
        .args(["copy", "--to", "file:///cache?compression=zstd", env_path])
        .output()
        .context("seed cache")?;
    eprintln!(
        "rebake: {} populated with {env_path}",
        candidate_host.display()
    );
    Ok(())
}

/// `<32 base32 chars>-<name>`, not a derivation or a temp entry.
fn looks_like_store_path(name: &str) -> bool {
    let Some((hash, rest)) = name.split_once('-') else {
        return false;
    };
    hash.len() == 32
        && hash
            .bytes()
            .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b))
        && !rest.is_empty()
        && !name.ends_with(".drv")
        && !name.ends_with(".lock")
        && !name.ends_with(".chroot")
}

/// Rw view of a bind we mounted read-only ourselves (not propagated, so not
/// locked): the push phase's write access to the cache.
fn remount_rw(target: &str) -> Result<()> {
    mount(
        None::<&str>,
        target,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REMOUNT,
        None::<&str>,
    )
    .with_context(|| format!("remount {target} rw"))
}

/// A directory held by fd survives the old root's detach; the magic link is
/// how the push phase sees the raw upper and lower after `pivot_root`.
fn fd_path(f: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", f.as_raw_fd()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_path_names() {
        assert!(looks_like_store_path(
            "pg32lah9bfxyh8492bvdwwdpz2ibrvia-slurm-ci-0.1.0"
        ));
        assert!(!looks_like_store_path(
            "pg32lah9bfxyh8492bvdwwdpz2ibrvia-foo.drv"
        ));
        assert!(!looks_like_store_path(
            "pg32lah9bfxyh8492bvdwwdpz2ibrvia-foo.lock"
        ));
        assert!(!looks_like_store_path(".links"));
        assert!(!looks_like_store_path("tmp-12345"));
        assert!(!looks_like_store_path("PG32LAH9BFXYH8492BVDWWDPZ2IBRVIA-x"));
    }
}
