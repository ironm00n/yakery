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

fn caller(name: &str) -> Result<&str> {
    ensure!(!name.starts_with(PARTIAL), "{name}: names starting with {PARTIAL} are Dir's own");
    Ok(name)
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
            match mkdirat(&dir.0, *name, RawMode::RWXU) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(e) => return Err(anyhow!(e).context(path())),
            }
            let next =
                dir.subdir(name)?.with_context(|| format!("{}: gone as it was made", path()))?;
            dir = next;
        }
        Ok(dir)
    }

    /// The directory `name` if it is there, opened from this one as `open` opens each of its names.
    pub fn subdir(&self, name: &str) -> Result<Option<Dir>> {
        let path = self.path(name);
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

    fn at(&self, name: &str, flags: OFlags) -> rustix::io::Result<File> {
        let flags = flags | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        openat(&self.0, name, flags, Mode::Private.bits()).map(File::from)
    }

    /// `name` open to read and write, created private if missing, never through a link.
    pub fn file(&self, name: &str) -> Result<File> {
        self.at(caller(name)?, OFlags::CREATE | OFlags::RDWR)
            .with_context(|| self.path(name).display().to_string())
    }

    /// `name` open to read, never through a link.
    pub fn open_file(&self, name: &str) -> Result<File> {
        self.at(name, OFlags::RDONLY).with_context(|| self.path(name).display().to_string())
    }

    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = || self.path(name).display().to_string();
        let mut file = match self.at(name, OFlags::RDONLY) {
            Err(Errno::NOENT) => return Ok(None),
            opened => opened.with_context(path)?,
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).with_context(path)?;
        Ok(Some(bytes))
    }

    /// Whole or not at all: through a partial file, synced with the directory, renamed over `name` only if
    /// `replace`. False if `name` was there and not replaced.
    pub fn write(&self, name: &str, bytes: &[u8], mode: Mode, replace: Replace) -> Result<bool> {
        self.write_with(name, bytes, mode, replace, |from, to| {
            renameat_with(&self.0, from, &self.0, to, RenameFlags::NOREPLACE)
        })
    }

    /// NFS refuses renameat2's flags with EINVAL; a hard link, which fails on an existing name, stands in for
    /// RENAME_NOREPLACE there. Tests pass their own `noreplace`.
    pub(crate) fn write_with(
        &self,
        name: &str,
        bytes: &[u8],
        mode: Mode,
        replace: Replace,
        noreplace: impl FnOnce(&str, &str) -> rustix::io::Result<()>,
    ) -> Result<bool> {
        caller(name)?;
        let partial = format!("{PARTIAL}{}", random::chars(random::ALPHANUMERIC, 16)?);
        let mut file = self.at(&partial, OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY)?;
        let written = fchmod(&file, mode.bits()).map_err(std::io::Error::from);
        let placed =
            match written.and_then(|()| file.write_all(bytes)).and_then(|()| file.sync_all()) {
                Err(e) => Err(Errno::from_io_error(&e).unwrap_or(Errno::IO)),
                Ok(()) if replace == Replace::Yes => renameat(&self.0, &partial, &self.0, name),
                Ok(()) => match noreplace(&partial, name) {
                    Err(Errno::INVAL) => linkat(&self.0, &partial, &self.0, name, AtFlags::empty())
                        .map(|()| {
                            let _ = unlinkat(&self.0, &partial, AtFlags::empty());
                        }),
                    renamed => renamed,
                },
            };
        if placed.is_err() {
            let _ = unlinkat(&self.0, &partial, AtFlags::empty());
        }
        match placed.and_then(|()| fsync(&self.0)) {
            Ok(()) => Ok(true),
            Err(Errno::EXIST) if replace == Replace::No => Ok(false),
            Err(e) => Err(anyhow!(e).context(self.path(name).display().to_string())),
        }
    }

    /// `bytes` at the end of `name`, which is created private if missing; synced.
    pub fn append(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let path = || self.path(name).display().to_string();
        let mut file = self
            .at(caller(name)?, OFlags::CREATE | OFlags::WRONLY | OFlags::APPEND)
            .with_context(path)?;
        file.write_all(bytes).and_then(|()| file.sync_data()).with_context(path)
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
        self.subdir(name)?.with_context(|| format!("{}: gone as it was made", path())).map(Some)
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
        match statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW) {
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
