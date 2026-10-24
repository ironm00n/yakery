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
        AtFlags, CWD, FileType, Mode as RawMode, OFlags, RenameFlags, fchmod, fstat, fsync,
        mkdirat, openat, renameat_with, statat, unlinkat,
    },
    io::Errno,
    process::getuid,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{now, random};

/// Written names never start with a dot, so no name a caller writes can take a partial file's.
const PARTIAL: &str = ".partial-";
pub const PARTIAL_EXPIRY: Duration = Duration::from_secs(600);

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
        let flags = OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC;
        let at = || base.display().to_string();
        DirBuilder::new().recursive(true).mode(0o700).create(base).with_context(at)?;
        let mut dir =
            Dir(openat(CWD, base, flags, RawMode::empty()).with_context(at)?, base.into());
        for name in names {
            dir.1.push(name);
            let path = dir.1.display().to_string();
            match mkdirat(&dir.0, *name, RawMode::RWXU) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(e) => return Err(anyhow!(e).context(path)),
            }
            let fd =
                openat(&dir.0, *name, flags | OFlags::NOFOLLOW, RawMode::empty()).map_err(|e| {
                    match e {
                        Errno::LOOP | Errno::NOTDIR => {
                            anyhow!("{path}: not a directory, or a symlink")
                        }
                        e => anyhow!(e).context(path.clone()),
                    }
                })?;
            let stat = fstat(&fd)?;
            let uid = stat.st_uid;
            ensure!(uid == getuid().as_raw(), "{path}: owned by uid {uid}, not this user");
            let mode = stat.st_mode & 0o777;
            ensure!(mode & 0o022 == 0, "{path}: others may write to it (mode {mode:o})");
            dir.0 = fd;
        }
        Ok(dir)
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
        self.at(name, OFlags::CREATE | OFlags::RDWR)
            .with_context(|| self.path(name).display().to_string())
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
        let partial = format!("{PARTIAL}{}", random::chars(random::ALPHANUMERIC, 16)?);
        let mut file = self.at(&partial, OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY)?;
        let flags = match replace {
            Replace::Yes => RenameFlags::empty(),
            Replace::No => RenameFlags::NOREPLACE,
        };
        let written = fchmod(&file, mode.bits()).map_err(std::io::Error::from);
        let renamed =
            match written.and_then(|()| file.write_all(bytes)).and_then(|()| file.sync_all()) {
                Ok(()) => renameat_with(&self.0, &partial, &self.0, name, flags),
                Err(e) => Err(Errno::from_io_error(&e).unwrap_or(Errno::IO)),
            };
        if renamed.is_err() {
            let _ = unlinkat(&self.0, &partial, AtFlags::empty());
        }
        match renamed.and_then(|()| fsync(&self.0)) {
            Ok(()) => Ok(true),
            Err(Errno::EXIST) if replace == Replace::No => Ok(false),
            Err(e) => Err(anyhow!(e).context(self.path(name).display().to_string())),
        }
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
                Ok(stat) if stat.st_mtime < expired => unlinkat(&self.0, name, AtFlags::empty())?,
                Ok(_) | Err(Errno::NOENT) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}
