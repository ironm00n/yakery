use std::{
    env,
    ffi::{CStr, OsStr},
    fs::{File, TryLockError},
    io::ErrorKind,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
};

use private::{Dir, Mode, Replace, Secret};

use crate::{
    address::{SessionId, Topic},
    listeners::Listeners,
    policy::Login,
    prelude::*,
};

pub fn home() -> Result<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).context("HOME is not set")
}

pub fn passwd_home() -> Result<PathBuf> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let (mut buf, mut found) = (vec![0 as libc::c_char; 16 * 1024], std::ptr::null_mut());
    // SAFETY: every pointer refers to storage that outlives the call, and `buf.len()` is its size.
    #[allow(unsafe_code)]
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
    #[allow(unsafe_code)]
    let dir = unsafe { CStr::from_ptr(entry.assume_init().pw_dir) };
    Ok(PathBuf::from(OsStr::from_bytes(dir.to_bytes())))
}

pub fn xdg(var: &str, fallback: &str) -> Result<PathBuf> {
    match env::var_os(var).map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => Ok(dir),
        _ => Ok(home()?.join(fallback)),
    }
}

pub struct Config {
    pub login: Login,
    pub password: PasswordFile,
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = xdg("XDG_CONFIG_HOME", ".config")?.join("agent-mail/config.json");
        match fs::read_to_string(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Config::parse(&path, "{}"),
            read => Config::parse(&path, &read.with_context(|| path.display().to_string())?),
        }
    }

    pub fn parse(path: &Path, text: &str) -> Result<Config> {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        struct Keys {
            login: Option<String>,
            password_file: Option<String>,
        }
        let path = path.display();
        let incomplete = format!("{path} must hold \"login\" and \"password-file\"");
        let keys: Keys = serde_json::from_str(text)
            .map_err(|e| Failure::Config.with(format!("{incomplete}: {e}")))?;
        let (Some(login), Some(file)) = (keys.login, keys.password_file) else {
            return Err(Failure::Config.with(incomplete));
        };
        let login = login.parse().map_err(|e| Failure::Config.with(format!("{path}: {e:#}")))?;
        let file = match file.strip_prefix("~/") {
            Some(rest) => home()?.join(rest),
            None => PathBuf::from(file),
        };
        Ok(Config { login, password: PasswordFile(file) })
    }
}

/// Which file a password came from, and in what state, without anything derived from the password itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub path: PathBuf,
    dev: u64,
    ino: u64,
    mtime: i64,
    mtime_nsec: i64,
    size: u64,
}

pub struct PasswordFile(pub PathBuf);

impl PasswordFile {
    fn path(&self, staged: bool) -> PathBuf {
        let mut path = self.0.clone().into_os_string();
        if staged {
            path.push(".new");
        }
        path.into()
    }

    pub fn read(&self, staged: bool) -> Result<Secret> {
        let path = self.path(staged);
        let text = fs::read_to_string(&path)
            .map_err(|e| Failure::Config.with(format!("{}: {e}", path.display())))?;
        match text.trim_end_matches(['\n', '\r']) {
            "" => Err(Failure::Config.with(format!("{} is empty", path.display()))),
            password => Ok(Secret::new(password)),
        }
    }

    pub fn staged(&self) -> Result<Option<Secret>> {
        match fs::metadata(self.path(true)) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            _ => self.read(true).map(Some),
        }
    }

    pub fn stage(&self, password: &Secret, _: &Held) -> Result<()> {
        let staged = self.path(true);
        let parent =
            staged.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let name = staged.file_name().and_then(|n| n.to_str()).context("no file name")?;
        let dir = Dir::open(parent, &[])?;
        dir.drop_stale_partials()?;
        dir.write(name, password.expose().as_bytes(), Mode::Private, Replace::Yes).map(drop)
    }

    pub fn promote(&self, expected: &Secret, _: &Held) -> Result<()> {
        let (staged, file) = (self.path(true), self.0.display());
        match self.staged()? {
            Some(p) if p == *expected => fs::rename(&staged, &self.0)
                .with_context(|| format!("promoting {}", staged.display())),
            Some(_) => bail!(
                "{} holds another password than the one that signed in; leaving it",
                staged.display()
            ),
            None if self.read(false).is_ok_and(|p| p == *expected) => Ok(()),
            None => bail!(
                "{} is gone, and {file} does not hold the password that signed in",
                staged.display()
            ),
        }
    }

    pub fn discard(&self, _: &Held) -> Result<()> {
        let staged = self.path(true);
        fs::remove_file(&staged).with_context(|| format!("removing {}", staged.display()))
    }

    pub fn fingerprint(&self, staged: bool) -> Result<Fingerprint> {
        let path = self.path(staged);
        let m = fs::metadata(&path)
            .map_err(|e| Failure::Config.with(format!("{}: {e}", path.display())))?;
        let (dev, ino, mtime, mtime_nsec, size) =
            (m.dev(), m.ino(), m.mtime(), m.mtime_nsec(), m.size());
        Ok(Fingerprint { path, dev, ino, mtime, mtime_nsec, size })
    }

    pub fn describe(&self) -> String {
        format!("{} and {}", self.0.display(), self.path(true).display())
    }
}

pub struct Held {
    _file: File,
}

/// What each Claude Code session watches, as its sends and subscribes recorded it, beside the locks, so
/// `wait --session` finds the same record whatever its environment.
pub struct Records(Dir);

#[derive(Default, Serialize, Deserialize)]
pub struct Watched {
    pub topics: BTreeSet<Topic>,
    pub threads: BTreeSet<String>,
}

impl Records {
    pub fn of(&self, session: &SessionId) -> Result<Watched> {
        self.0.load(&format!("{session}.json"))
    }

    pub fn record(
        &self,
        session: &SessionId,
        topics: Vec<Topic>,
        threads: Vec<String>,
    ) -> Result<()> {
        let lock = self.0.file("lock")?;
        lock.lock().context("locking the session records")?;
        let name = format!("{session}.json");
        let mut watched: Watched = self.0.load(&name)?;
        watched.topics.extend(topics);
        watched.threads.extend(threads);
        self.0.save(&name, &watched)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Access {
    pub token: Secret,
    pub expires_at: i64,
}

impl Access {
    pub fn fresh(&self, now: i64) -> bool {
        now + 60 < self.expires_at
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Tokens {
    pub access: Option<Access>,
    pub refresh: Option<Secret>,
}

#[derive(Default, Serialize, Deserialize)]
struct OnDisk {
    login: Option<String>,
    #[serde(flatten)]
    tokens: Tokens,
}

#[derive(PartialEq, Serialize, Deserialize)]
struct Refusal {
    login: String,
    file: Fingerprint,
}

/// The tokens every session of the user shares, under `$XDG_STATE_HOME`, and the locks and refusals under the
/// passwd home, so that every session of the user on this host takes the same ones.
pub struct Store {
    pub state: PathBuf,
    pub locks: PathBuf,
    pub login: Login,
}

impl Store {
    pub fn new(login: &Login) -> Result<Store> {
        let (state, locks) =
            (xdg("XDG_STATE_HOME", ".local/state")?, passwd_home()?.join(".local/state"));
        Ok(Store { state, locks, login: login.clone() })
    }

    fn dir(&self, locks: bool) -> Result<Dir> {
        match locks {
            true => Dir::open(&self.locks, &["agent-mail", "locks"]),
            false => Dir::open(&self.state, &["agent-mail"]),
        }
    }

    fn key(&self) -> String {
        self.login.to_string().to_lowercase()
    }

    pub async fn lock(&self, name: &str) -> Result<File> {
        let dir = self.dir(true)?;
        let path = dir.path(name).display().to_string();
        let file = dir.file(name)?;
        for _ in 0..600 {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => {
                    tokio::time::sleep(Duration::from_millis(200)).await
                }
                Err(TryLockError::Error(e)) => return Err(e).context(format!("locking {path}")),
            }
        }
        bail!("another agent-mail has held {path} for over 120 s")
    }

    pub fn records(&self) -> Result<Records> {
        Ok(Records(Dir::open(&self.locks, &["agent-mail", "sessions"])?))
    }

    pub fn listeners(&self) -> Result<Listeners> {
        Ok(Listeners(Dir::open(&self.locks, &["agent-mail", "sessions"])?))
    }

    pub async fn held(&self) -> Result<Held> {
        Ok(Held { _file: self.lock("auth.lock").await? })
    }

    pub fn tokens(&self, _: &Held) -> Result<Tokens> {
        let stored: OnDisk = self.dir(false)?.load("tokens.json")?;
        let ours =
            stored.login.and_then(|l| l.parse::<Login>().ok()).is_some_and(|l| l == self.login);
        Ok(if ours { stored.tokens } else { Tokens::default() })
    }

    pub fn save(&self, tokens: Tokens, _: &Held) -> Result<()> {
        self.dir(false)?.save("tokens.json", &OnDisk { login: Some(self.key()), tokens })
    }

    pub fn refused(&self, file: &Fingerprint, record: bool, _: &Held) -> Result<bool> {
        let dir = self.dir(true)?;
        let mut all: Vec<Refusal> = dir.load("refused.json")?;
        let refusal = Refusal { login: self.key(), file: file.clone() };
        let known = all.contains(&refusal);
        if record && !known {
            all.push(refusal);
            dir.save("refused.json", &all)?;
        }
        Ok(known)
    }
}
