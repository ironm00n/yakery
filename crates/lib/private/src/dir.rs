use std::{
    fs::{DirBuilder, File},
    io::{Read, Write},
    os::{fd::OwnedFd, unix::fs::DirBuilderExt},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use rustix::{
    fs::{
        AtFlags, CWD, FileType, Mode as RawMode, OFlags, RenameFlags, fchmod, fstat, fsync, linkat,
        mkdirat, openat, renameat, renameat_with, statat, unlinkat,
    },
    io::Errno,
    process::getuid,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{now, random};

/// Dir's own names for files it is still writing; no caller may use one, so the sweep removes only those.
const PARTIAL: &str = ".partial-";
pub const PARTIAL_EXPIRY: Duration = Duration::from_secs(600);

/// Every guarantee holds per path component, so a name is exactly one.
fn entry(name: &str) -> Result<&str> {
    let one = !name.is_empty() && !name.contains('/') && name != "." && name != "..";
    ensure!(one, "{:?} is not a single file name", name);
    Ok(name)
}

fn caller(name: &str) -> Result<&str> {
    ensure!(
        !entry(name)?.starts_with(PARTIAL),
        "{name}: names starting with {PARTIAL} are Dir's own"
    );
    Ok(name)
}

fn errno(e: &anyhow::Error) -> Option<Errno> {
    e.downcast_ref::<Errno>().copied()
}

/// Mode set, bytes written and synced, and closed: NFS keeps a file unlinked while open under a .nfs name.
fn synced(mut file: File, bytes: &[u8], mode: Mode) -> rustix::io::Result<(u64, u64)> {
    fchmod(&file, mode.bits())?;
    let io = |e: std::io::Error| Errno::from_io_error(&e).unwrap_or(Errno::IO);
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(io)?;
    let stat = fstat(&file)?;
    Ok((stat.st_dev, stat.st_ino))
}

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Private,
    Readable,
}

impl Mode {
    fn bits(self) -> RawMode {
        RawMode::from_raw_mode(match self {
            Mode::Private => 0o600,
            Mode::Readable => 0o644,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replace {
    Yes,
    No,
}

/// A directory reached through a handle. `base` is the caller's and is created as a path; each of `names` below it
/// is ours: created private, or else a real directory of this user's that no one else may write to, opened from its
/// parent's handle, so moving or linking one on the way cannot redirect a write.
pub struct Dir(OwnedFd, PathBuf);

impl Dir {
    pub fn open(base: &Path, names: &[&str]) -> Result<Dir> {
        let at = || base.display().to_string();
        DirBuilder::new().recursive(true).mode(0o700).create(base).with_context(at)?;
        let flags = OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC;
        let mut dir =
            Dir(openat(CWD, base, flags, RawMode::empty()).with_context(at)?, base.into());
        for name in names {
            let path = || dir.path(name).display().to_string();
            let made = match mkdirat(&dir.0, entry(name)?, RawMode::RWXU) {
                Ok(()) => true,
                Err(Errno::EXIST) => false,
                Err(e) => return Err(anyhow!(e).context(path())),
            };
            let next =
                dir.subdir(name)?.with_context(|| format!("{}: gone as it was made", path()))?;
            if made {
                fchmod(&next.0, RawMode::RWXU).with_context(path)?;
            }
            dir = next;
        }
        Ok(dir)
    }

    /// The directory `name` if it is there, opened from this one as `open` opens each of its names.
    pub fn subdir(&self, name: &str) -> Result<Option<Dir>> {
        let path = self.path(entry(name)?);
        let shown = path.display().to_string();
        let flags = OFlags::DIRECTORY | OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match openat(&self.0, name, flags, RawMode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(Errno::LOOP | Errno::NOTDIR) => bail!("{shown}: not a directory, or a symlink"),
            Err(e) => return Err(anyhow!(e).context(shown)),
        };
        let stat = fstat(&fd)?;
        let uid = stat.st_uid;
        ensure!(uid == getuid().as_raw(), "{shown}: owned by uid {uid}, not this user");
        let mode = stat.st_mode & 0o777;
        ensure!(mode & 0o022 == 0, "{shown}: others may write to it (mode {mode:o})");
        Ok(Some(Dir(fd, path)))
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.1.join(name)
    }

    /// A regular file only: non-blocking, so a FIFO put in a file's place cannot hang the open.
    fn at(&self, name: &str, flags: OFlags) -> Result<File> {
        let shown = || self.path(name).display().to_string();
        let flags = flags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let fd = openat(&self.0, entry(name)?, flags, Mode::Private.bits())
            .map_err(anyhow::Error::from)
            .with_context(shown)?;
        let kind = FileType::from_raw_mode(fstat(&fd).with_context(shown)?.st_mode);
        ensure!(kind == FileType::RegularFile, "{}: not a regular file", shown());
        Ok(File::from(fd))
    }

    /// `name` open to read and write, created private if missing, never through a link.
    pub fn file(&self, name: &str) -> Result<File> {
        self.at(caller(name)?, OFlags::CREATE | OFlags::RDWR)
    }

    /// `name` open to read, never through a link.
    pub fn open_file(&self, name: &str) -> Result<File> {
        self.at(name, OFlags::RDONLY)
    }

    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let mut file = match self.at(name, OFlags::RDONLY) {
            Err(e) if errno(&e) == Some(Errno::NOENT) => return Ok(None),
            opened => opened?,
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).with_context(|| self.path(name).display().to_string())?;
        Ok(Some(bytes))
    }

    /// Whole or not at all: through a partial file, synced with the directory, then renamed to `name`, over what
    /// is there only if `replace`. False if `name` was there and not replaced.
    pub fn write(&self, name: &str, bytes: &[u8], mode: Mode, replace: Replace) -> Result<bool> {
        let noreplace = |from: &str, to: &str| {
            renameat_with(&self.0, from, &self.0, to, RenameFlags::NOREPLACE)
        };
        let link = |from: &str, to: &str| linkat(&self.0, from, &self.0, to, AtFlags::empty());
        self.write_with(name, bytes, mode, replace, noreplace, link)
    }

    /// NFS refuses renameat2's flags with EINVAL; a hard link, which fails on an existing name, stands in for
    /// RENAME_NOREPLACE there, and since NFS answers a LINK replayed after a lost reply with EEXIST, a name that is
    /// already this inode counts as linked. Tests pass their own renameat2 and linkat.
    pub(crate) fn write_with(
        &self,
        name: &str,
        bytes: &[u8],
        mode: Mode,
        replace: Replace,
        noreplace: impl FnOnce(&str, &str) -> rustix::io::Result<()>,
        link: impl FnOnce(&str, &str) -> rustix::io::Result<()>,
    ) -> Result<bool> {
        caller(name)?;
        let partial = format!("{PARTIAL}{}", random::chars(random::ALPHANUMERIC, 16)?);
        let file = self.at(&partial, OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY)?;
        let placed = synced(file, bytes, mode).and_then(|ours| match replace {
            Replace::Yes => renameat(&self.0, &partial, &self.0, name),
            Replace::No => match noreplace(&partial, name) {
                Err(Errno::INVAL) => {
                    let linked = match link(&partial, name) {
                        Err(Errno::EXIST) => statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)
                            .ok()
                            .filter(|stat| (stat.st_dev, stat.st_ino) == ours)
                            .map_or(Err(Errno::EXIST), |_| Ok(())),
                        linked => linked,
                    };
                    linked.map(|()| {
                        let _ = unlinkat(&self.0, &partial, AtFlags::empty());
                    })
                }
                renamed => renamed,
            },
        });
        if placed.is_err() {
            let _ = unlinkat(&self.0, &partial, AtFlags::empty());
        }
        match placed.and_then(|()| fsync(&self.0)) {
            Ok(()) => Ok(true),
            Err(Errno::EXIST) if replace == Replace::No => Ok(false),
            Err(e) => Err(anyhow!(e).context(self.path(name).display().to_string())),
        }
    }

    /// `bytes` at the end of `name`, created private and synced with the directory if missing. Whole or not at all
    /// for one writer at a time, since a failed write is cut back off; O_APPEND does not order writers on NFS.
    pub fn append(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let flags = OFlags::WRONLY | OFlags::APPEND;
        let mut file = match self.at(caller(name)?, flags | OFlags::CREATE | OFlags::EXCL) {
            Ok(file) => {
                fchmod(&file, Mode::Private.bits())?;
                fsync(&self.0)?;
                file
            }
            Err(e) if errno(&e) == Some(Errno::EXIST) => self.at(name, flags)?,
            Err(e) => return Err(e),
        };
        let start = file.metadata()?.len();
        let appended = file.write_all(bytes).and_then(|()| file.sync_data());
        appended
            .or_else(|e| file.set_len(start).and(Err(e)))
            .with_context(|| self.path(name).display().to_string())
    }

    /// `name`, made here private, or `None` if a directory has the name already.
    pub fn create_dir(&self, name: &str) -> Result<Option<Dir>> {
        caller(name)?;
        let path = || self.path(name).display().to_string();
        match mkdirat(&self.0, name, RawMode::RWXU) {
            Ok(()) => {}
            Err(Errno::EXIST) => return self.subdir(name).map(|_| None),
            Err(e) => return Err(anyhow!(e).context(path())),
        }
        fsync(&self.0).with_context(path)?;
        let made =
            self.subdir(name)?.with_context(|| format!("{}: gone as it was made", path()))?;
        fchmod(&made.0, RawMode::RWXU).with_context(path)?;
        Ok(Some(made))
    }

    pub fn load<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        match self.read(name)? {
            None => Ok(T::default()),
            Some(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| self.path(name).display().to_string()),
        }
    }

    pub fn save(&self, name: &str, value: &impl Serialize) -> Result<()> {
        self.write(name, &serde_json::to_vec(value)?, Mode::Private, Replace::Yes).map(drop)
    }

    pub fn existing(&self, name: &str) -> Result<Option<PathBuf>> {
        match statat(&self.0, entry(name)?, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile => {
                Ok(Some(self.path(name)))
            }
            Ok(_) => bail!("{name} is there but is not a file"),
            Err(Errno::NOENT) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn drop_stale_partials(&self) -> Result<()> {
        let expired = now().saturating_sub(PARTIAL_EXPIRY.as_secs() as i64);
        for entry in rustix::fs::Dir::read_from(&self.0)? {
            let entry = entry?;
            let name = entry.file_name();
            if !name.to_bytes().starts_with(PARTIAL.as_bytes()) {
                continue;
            }
            match statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat)
                    if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
                        && stat.st_mtime < expired =>
                {
                    match unlinkat(&self.0, name, AtFlags::empty()) {
                        Ok(()) | Err(Errno::NOENT) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                Ok(_) | Err(Errno::NOENT) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}
