use std::{
    env,
    fs::{DirBuilder, File},
    io::{ErrorKind, Write},
    os::{fd::OwnedFd, unix::ffi::OsStrExt, unix::fs::DirBuilderExt},
    time::{SystemTime, UNIX_EPOCH},
};

use rustix::{
    fs::{
        AtFlags, CWD, FileType, Mode, OFlags, RenameFlags, fstat, fsync, mkdirat, openat,
        renameat_with, statat, unlinkat,
    },
    io::Errno,
    process::getuid,
};

use crate::prelude::*;

const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
pub const PASSWORD_CLASSES: [&[u8]; 4] = [
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    b"abcdefghijklmnopqrstuvwxyz",
    b"0123456789",
    b"!#%&*+-./:;=?@^_~",
];
/// Saved names never start with a dot, so no attachment can take a partial file's name.
const PARTIAL: &str = ".partial-";
pub const PARTIAL_EXPIRY: Duration = Duration::from_secs(600);

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn below(n: usize) -> Result<usize> {
    loop {
        let mut byte = [0];
        getrandom::fill(&mut byte)?;
        if usize::from(byte[0]) < 256 - 256 % n {
            return Ok(usize::from(byte[0]) % n);
        }
    }
}

fn pick(set: &[u8]) -> Result<u8> {
    Ok(set[below(set.len())?])
}

pub fn token(len: usize) -> Result<String> {
    (0..len).map(|_| pick(ALPHANUMERIC).map(char::from)).collect()
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn password(len: usize) -> Result<Secret> {
        let mut chars = PASSWORD_CLASSES.iter().map(|c| pick(c)).collect::<Result<Vec<u8>>>()?;
        while chars.len() < len {
            chars.push(pick(&PASSWORD_CLASSES.concat())?);
        }
        for i in (1..chars.len()).rev() {
            chars.swap(i, below(i + 1)?);
        }
        Ok(Secret(chars.into_iter().map(char::from).collect()))
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

pub fn home(trusted: bool) -> Result<PathBuf> {
    if !trusted {
        return env::var_os("HOME").map(PathBuf::from).context("HOME is not set");
    }
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let (mut buf, mut found) = (vec![0 as libc::c_char; 16 * 1024], std::ptr::null_mut());
    // SAFETY: every pointer refers to storage that outlives the call, and `buf.len()` is its size.
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    ensure!(status == 0 && !found.is_null(), "no passwd entry for this user");
    // SAFETY: getpwuid_r succeeded, so `entry` is filled in and `pw_dir` points into `buf`.
    let dir = unsafe { std::ffi::CStr::from_ptr(entry.assume_init().pw_dir) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes())))
}

pub fn xdg(var: &str, fallback: &str) -> Result<PathBuf> {
    match env::var_os(var).map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => Ok(dir),
        _ => Ok(home(false)?.join(fallback)),
    }
}

/// A directory reached through a handle. `base` is the user's and is created as a path; each of `names` below it is
/// ours: created private, or else a real directory of the user's that no one else may write to, opened from its
/// parent's handle, so moving or linking one on the way cannot redirect a write.
pub struct Dir(OwnedFd, PathBuf);

impl Dir {
    pub fn open(base: &Path, names: &[&str]) -> Result<Dir> {
        let flags = OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC;
        let at = || base.display().to_string();
        DirBuilder::new().recursive(true).mode(0o700).create(base).with_context(at)?;
        let mut dir = Dir(openat(CWD, base, flags, Mode::empty()).with_context(at)?, base.into());
        for name in names {
            dir.1.push(name);
            let path = dir.1.display().to_string();
            match mkdirat(&dir.0, *name, Mode::RWXU) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(e) => return Err(anyhow!(e).context(path)),
            }
            let fd = openat(&dir.0, *name, flags | OFlags::NOFOLLOW, Mode::empty()).map_err(
                |e| match e {
                    Errno::LOOP | Errno::NOTDIR => anyhow!("{path}: not a directory, or a symlink"),
                    e => anyhow!(e).context(path.clone()),
                },
            )?;
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

    pub fn file(&self, name: &str, flags: OFlags) -> rustix::io::Result<File> {
        openat(&self.0, name, flags | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::RUSR | Mode::WUSR)
            .map(File::from)
    }

    pub fn write(&self, name: &str, bytes: &[u8], replace: bool) -> Result<bool> {
        let partial = format!("{PARTIAL}{}", token(16)?);
        let mut file = self.file(&partial, OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY)?;
        let flags = if replace { RenameFlags::empty() } else { RenameFlags::NOREPLACE };
        let renamed = match file.write_all(bytes).and_then(|()| file.sync_all()) {
            Ok(()) => renameat_with(&self.0, &partial, &self.0, name, flags),
            Err(e) => Err(Errno::from_io_error(&e).unwrap_or(Errno::IO)),
        };
        if renamed.is_err() {
            let _ = unlinkat(&self.0, &partial, AtFlags::empty());
        }
        match renamed.and_then(|()| fsync(&self.0)) {
            Ok(()) => Ok(true),
            Err(Errno::EXIST) if !replace => Ok(false),
            Err(e) => Err(anyhow!(e).context(self.path(name).display().to_string())),
        }
    }

    pub fn load<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        let path = self.path(name);
        match fs::read(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(T::default()),
            read => serde_json::from_slice(&read?).with_context(|| path.display().to_string()),
        }
    }

    pub fn save(&self, name: &str, value: &impl Serialize) -> Result<()> {
        self.write(name, &serde_json::to_vec(value)?, true).map(drop)
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
