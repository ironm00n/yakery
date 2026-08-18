//! Namespace and mount plumbing for the §5 stack: stock kernel, no helper
//! binaries. Every function here runs in a re-exec'd single-threaded child —
//! `unshare(CLONE_NEWUSER)` refuses multithreaded callers.

use std::ffi::CString;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::sys::statvfs::statvfs;
use nix::unistd::{chdir, getgid, getuid, pivot_root};

/// New user + mount namespace with the caller mapped to `uid:gid` inside.
/// Called as the real user this maps them to root-in-ns; called again from a
/// child of that namespace it maps ns-root to an ordinary uid (§5 step 4).
pub fn enter_userns(uid: u32, gid: u32) -> Result<()> {
    let (outer_uid, outer_gid) = (getuid().as_raw(), getgid().as_raw());
    unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS)
        .context("unshare(NEWUSER|NEWNS)")?;
    fs::write("/proc/self/uid_map", format!("{uid} {outer_uid} 1\n")).context("write uid_map")?;
    fs::write("/proc/self/setgroups", "deny\n").context("write setgroups")?;
    fs::write("/proc/self/gid_map", format!("{gid} {outer_gid} 1\n")).context("write gid_map")?;
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("make / rprivate")?;
    Ok(())
}

/// Takes effect for the *next* fork, not the caller.
pub fn unshare_pid() -> Result<()> {
    unshare(CloneFlags::CLONE_NEWPID).context("unshare(NEWPID)")
}

/// Once the child pid namespace's init has exited, every further `fork()`
/// would land in a dead namespace (ENOMEM). Point children back at our own.
pub fn restore_pid_for_children() -> Result<()> {
    let own = fs::File::open("/proc/self/ns/pid").context("open /proc/self/ns/pid")?;
    nix::sched::setns(own, CloneFlags::CLONE_NEWPID).context("setns(own pid ns)")
}

/// A fresh procfs for whoever mounted it; parent and child each need their own.
pub fn mount_proc(target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    mount(
        Some("proc"),
        target,
        Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        None::<&str>,
    )
    .with_context(|| format!("mount proc at {}", target.display()))
}

/// A minimal root on a private tmpfs, populated then `pivot_root`ed into.
pub struct Rootfs {
    root: PathBuf,
}

impl Rootfs {
    pub fn new(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        mount(
            Some("tmpfs"),
            root,
            Some("tmpfs"),
            MsFlags::MS_NOSUID,
            Some("mode=755"),
        )
        .with_context(|| format!("mount rootfs tmpfs at {}", root.display()))?;
        Ok(Rootfs {
            root: root.to_owned(),
        })
    }

    fn at(&self, target: &str) -> PathBuf {
        self.root.join(target.trim_start_matches('/'))
    }

    pub fn bind(&self, source: &Path, target: &str, ro: bool) -> Result<()> {
        let dest = self.at(target);
        if source.is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            fs::write(&dest, b"")?;
        }
        let rec = MsFlags::MS_BIND | MsFlags::MS_REC;
        mount(Some(source), &dest, None::<&str>, rec, None::<&str>)
            .with_context(|| format!("bind {} -> {target}", source.display()))?;
        if ro {
            // A userns may add restrictions but never clear a locked one, so
            // the remount must carry the source mount's own nosuid/nodev/
            // noexec/atime flags or it is refused with EPERM.
            let keep = locked_flags(source)?;
            mount(
                None::<&str>,
                &dest,
                None::<&str>,
                MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | keep,
                None::<&str>,
            )
            .with_context(|| format!("remount {target} read-only"))?;
        }
        Ok(())
    }

    pub fn tmpfs(&self, target: &str, opts: &str) -> Result<()> {
        let dest = self.at(target);
        fs::create_dir_all(&dest)?;
        mount(
            Some("tmpfs"),
            &dest,
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some(opts),
        )
        .with_context(|| format!("mount tmpfs at {target}"))
    }

    /// `userxattr,volatile`: the only overlay configuration that works on this
    /// kernel with an xfs upper (§6).
    pub fn overlay(&self, target: &str, lower: &Path, upper: &Path, work: &Path) -> Result<()> {
        let dest = self.at(target);
        fs::create_dir_all(&dest)?;
        fs::create_dir_all(upper)?;
        fs::create_dir_all(work)?;
        let opts = format!(
            "lowerdir={},upperdir={},workdir={},userxattr,volatile",
            lower.display(),
            upper.display(),
            work.display()
        );
        mount(
            Some("overlay"),
            &dest,
            Some("overlay"),
            MsFlags::empty(),
            Some(opts.as_str()),
        )
        .with_context(|| format!("mount overlay at {target} ({opts})"))
    }

    /// Mounted before `pivot_root`, while the host's procfs is still visible
    /// — the kernel refuses a new proc mount in a userns otherwise.
    pub fn proc(&self) -> Result<()> {
        mount_proc(&self.at("/proc"))
    }

    pub fn sys_ro(&self) -> Result<()> {
        self.bind(Path::new("/sys"), "/sys", true)
    }

    pub fn devices(&self) -> Result<()> {
        for name in ["null", "zero", "full", "random", "urandom", "tty"] {
            self.bind(
                &Path::new("/dev").join(name),
                &format!("/dev/{name}"),
                false,
            )?;
        }
        self.tmpfs("/dev/shm", "mode=1777")?;
        if Path::new("/dev/pts").is_dir() {
            self.bind(Path::new("/dev/pts"), "/dev/pts", false)?;
            self.symlink("pts/ptmx", "/dev/ptmx")?;
        }
        self.symlink("/proc/self/fd", "/dev/fd")?;
        self.symlink("/proc/self/fd/0", "/dev/stdin")?;
        self.symlink("/proc/self/fd/1", "/dev/stdout")?;
        self.symlink("/proc/self/fd/2", "/dev/stderr")?;
        Ok(())
    }

    pub fn write(&self, target: &str, contents: &str) -> Result<()> {
        let dest = self.at(target);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(&dest, contents).with_context(|| format!("write {target}"))
    }

    pub fn mkdir(&self, target: &str, mode: u32) -> Result<()> {
        let dest = self.at(target);
        fs::create_dir_all(&dest)?;
        fs::set_permissions(&dest, fs::Permissions::from_mode(mode))?;
        Ok(())
    }

    pub fn symlink(&self, to: &str, target: &str) -> Result<()> {
        let dest = self.at(target);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        symlink(to, &dest).with_context(|| format!("symlink {target} -> {to}"))
    }

    /// Synthesised `/etc`: enough for nss, TLS, and a mapped uid to have a name.
    pub fn etc(&self, uid: u32, gid: u32, resolv_conf: &str) -> Result<()> {
        self.write(
            "/etc/passwd",
            &format!("root:x:0:0:root:/root:/bin/sh\nci:x:{uid}:{gid}:ci:/tmp/home:/bin/sh\n"),
        )?;
        self.write("/etc/group", &format!("root:x:0:\nci:x:{gid}:\n"))?;
        self.write(
            "/etc/nsswitch.conf",
            "passwd: files\ngroup: files\nhosts: files dns\n",
        )?;
        self.write("/etc/hosts", "127.0.0.1 localhost\n::1 localhost\n")?;
        self.write("/etc/resolv.conf", resolv_conf)?;
        Ok(())
    }

    /// `pivot_root(".", ".")` then detach the old root (pivot_root(2)). Not
    /// chroot: a chrooted process cannot create user namespaces.
    pub fn pivot(self) -> Result<()> {
        chdir(&self.root).context("chdir new root")?;
        pivot_root(".", ".").context("pivot_root")?;
        umount2(".", MntFlags::MNT_DETACH).context("detach old root")?;
        chdir("/").context("chdir /")?;
        Ok(())
    }
}

/// statvfs(2) `f_flag` bits → mount flags; the named constants are partly
/// gated off on musl, so these are the numbers from the man page.
fn locked_flags(path: &Path) -> Result<MsFlags> {
    let st = statvfs(path).with_context(|| format!("statvfs {}", path.display()))?;
    let bits = st.flags().bits() as u64;
    let mut out = MsFlags::empty();
    for (st_bit, ms_flag) in [
        (0x0002, MsFlags::MS_NOSUID),
        (0x0004, MsFlags::MS_NODEV),
        (0x0008, MsFlags::MS_NOEXEC),
        (0x0400, MsFlags::MS_NOATIME),
        (0x0800, MsFlags::MS_NODIRATIME),
        (0x1000, MsFlags::MS_RELATIME),
    ] {
        if bits & st_bit != 0 {
            out |= ms_flag;
        }
    }
    Ok(out)
}

/// The host's resolver config, unless it is a loopback stub that would be dead
/// inside the namespace; then systemd-resolved's upstream file (§5).
pub fn usable_resolv_conf() -> Result<String> {
    for path in ["/etc/resolv.conf", "/run/systemd/resolve/resolv.conf"] {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let servers: Vec<&str> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("nameserver"))
            .map(str::trim)
            .collect();
        if !servers.is_empty() && !servers.iter().all(|s| s.starts_with("127.") || *s == "::1") {
            return Ok(text);
        }
    }
    bail!("no non-loopback nameserver in /etc/resolv.conf or systemd-resolved's upstream file")
}

/// Capability probe for the upper (§5): `user.*` xattrs must stick.
pub fn supports_user_xattr(dir: &Path) -> Result<bool> {
    let probe = dir.join(".xattr-probe");
    fs::write(&probe, b"")?;
    let path = CString::new(probe.as_os_str().as_encoded_bytes())?;
    let name = CString::new("user.slurm-ci")?;
    let rc =
        unsafe { nix::libc::setxattr(path.as_ptr(), name.as_ptr(), b"1".as_ptr().cast(), 1, 0) };
    let _ = fs::remove_file(&probe);
    Ok(rc == 0)
}

/// `chmod -R u+rwX` then remove: overlay workdirs contain mode-000 dirs (§6).
pub fn force_remove(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fn open_up(p: &Path) {
        if let Ok(meta) = fs::symlink_metadata(p) {
            if meta.is_dir() {
                let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
                for e in fs::read_dir(p).into_iter().flatten().flatten() {
                    open_up(&e.path());
                }
            }
        }
    }
    open_up(path);
    fs::remove_dir_all(path).with_context(|| format!("remove {}", path.display()))
}

/// Allocated bytes under `path`, symlinks not followed.
pub fn dir_size(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    let own = meta.blocks() * 512;
    if !meta.is_dir() {
        return own;
    }
    own + fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| dir_size(&e.path()))
        .sum::<u64>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_size_and_force_remove() {
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("work");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("f"), vec![0u8; 8192]).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        assert!(dir_size(dir.path()) >= 8192 || dir_size(dir.path()) == 0);
        force_remove(&locked).unwrap();
        assert!(!locked.exists());
        force_remove(&locked).unwrap();
    }

    #[test]
    fn xattr_probe_runs() {
        let dir = tempfile::tempdir().unwrap();
        let _ = supports_user_xattr(dir.path()).unwrap();
    }
}
