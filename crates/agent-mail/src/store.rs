use std::{
    env,
    ffi::{CStr, OsStr},
    fs::{File, TryLockError},
    io::ErrorKind,
    ops::Deref,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt},
    },
};

use oidc::Access;
use private::{Dir, Mode, Replace, Secret, now};
use tokio::time::Instant;

use crate::{
    address::{SessionId, Topic},
    deadline,
    listeners::Listeners,
    policy::Login,
    prelude::*,
    show::utc,
    web::TIMEOUT,
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

/// A lock this process holds. Its file holds its lease: until when the holder says it is busy, which is as long as
/// a waiter waits for it. Letting the lock go empties the file, so no lease outlasts its holding.
pub struct Lock {
    file: File,
    path: String,
}

impl Drop for Lock {
    fn drop(&mut self) {
        if let Err(e) = self.file.set_len(0) {
            warn!("clearing the lease on {}: {e}", self.path);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Lease {
    pid: u32,
    until: i64,
}

impl Lock {
    /// `name` in `dir`, if no other process holds it, leased for one request's time.
    pub fn try_take(dir: &Dir, name: &str) -> Result<Option<Lock>> {
        let (file, path) = (dir.file(name)?, dir.path(name).display().to_string());
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e).context(format!("locking {path}")),
        }
        let lock = Lock { file, path };
        lock.lease(Instant::now() + TIMEOUT)?;
        Ok(Some(lock))
    }

    /// Waits while another process holds `name` in `dir` under a lease, and fails naming it once two polls find the
    /// same lease ended, or none: one poll may come between a holder taking the lock and leasing it. Under
    /// [`deadline::by`], it gives up at that deadline too.
    pub async fn take(dir: &Dir, name: &str) -> Result<Lock> {
        let path = dir.path(name).display().to_string();
        let waiting = async {
            let mut seen = None;
            loop {
                if let Some(lock) = Lock::try_take(dir, name)? {
                    return anyhow::Ok(lock);
                }
                let said = dir.read(name)?.unwrap_or_default();
                let lease = serde_json::from_slice::<Lease>(&said).ok();
                if seen.as_ref() == Some(&said) && lease.as_ref().is_none_or(|l| l.until < now()) {
                    bail!(match lease {
                        Some(l) => format!(
                            "{path} is held by pid {}, past its lease until {}",
                            l.pid,
                            utc(l.until)
                        ),
                        None => format!(
                            "{path} is held by a process that wrote no lease, so which one is not known"
                        ),
                    });
                }
                if seen.is_none() {
                    note!("another agent-mail holds {path}; waiting for it");
                }
                seen = Some(said);
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        };
        deadline::bounded(waiting)
            .await
            .with_context(|| format!("gave up waiting for {path} at the deadline"))?
    }

    fn lease(&self, until: Instant) -> Result<()> {
        let left = until.saturating_duration_since(Instant::now()).as_secs_f64().ceil() as i64;
        let lease = serde_json::to_vec(&Lease { pid: std::process::id(), until: now() + left })?;
        let file = &self.file;
        let written = file.write_all_at(&lease, 0).and_then(|()| file.set_len(lease.len() as u64));
        written.with_context(|| format!("leasing {}", self.path))
    }

    /// `work` with the lock leased until `deadline`, given up there.
    pub async fn until<T>(
        &self,
        deadline: Instant,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        self.lease(deadline)?;
        let done = tokio::time::timeout_at(deadline, work).await;
        done.map_err(|_| anyhow!("gave up at the end of the lease on {}", self.path))?
    }

    pub async fn within<T>(
        &self,
        bound: Duration,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        self.until(Instant::now() + bound, work).await
    }
}

/// The auth lock, held.
pub struct Held(Lock);

impl Deref for Held {
    type Target = Lock;

    fn deref(&self) -> &Lock {
        &self.0
    }
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

    pub async fn record(
        &self,
        session: &SessionId,
        topics: Vec<Topic>,
        threads: Vec<String>,
    ) -> Result<()> {
        self.change(session, |watched| {
            watched.topics.extend(topics);
            watched.threads.extend(threads);
        })
        .await
    }

    /// Takes exactly the topics and threads named out of `session`'s record, and returns those it did not hold.
    pub async fn forget(
        &self,
        session: &SessionId,
        topics: Vec<Topic>,
        threads: Vec<String>,
    ) -> Result<Watched> {
        let mut absent = Watched::default();
        self.change(session, |watched| {
            absent.topics = topics.into_iter().filter(|t| !watched.topics.remove(t)).collect();
            absent.threads = threads.into_iter().filter(|t| !watched.threads.remove(t)).collect();
        })
        .await?;
        Ok(absent)
    }

    async fn change(&self, session: &SessionId, edit: impl FnOnce(&mut Watched)) -> Result<()> {
        let _held = Lock::take(&self.0, "lock").await?;
        let name = format!("{session}.json");
        let mut watched: Watched = self.0.load(&name)?;
        edit(&mut watched);
        self.0.save(&name, &watched)
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

    pub async fn lock(&self, name: &str) -> Result<Lock> {
        Lock::take(&self.dir(true)?, name).await
    }

    pub fn records(&self) -> Result<Records> {
        Ok(Records(Dir::open(&self.locks, &["agent-mail", "sessions"])?))
    }

    pub fn listeners(&self) -> Result<Listeners> {
        Ok(Listeners(Dir::open(&self.locks, &["agent-mail", "sessions"])?))
    }

    pub async fn held(&self) -> Result<Held> {
        Ok(Held(self.lock("auth.lock").await?))
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

    pub fn refused(&self, file: &Fingerprint, _: &Held) -> Result<bool> {
        let all: Vec<Refusal> = self.dir(true)?.load("refused.json")?;
        Ok(all.contains(&Refusal { login: self.key(), file: file.clone() }))
    }

    /// Records the refusal of `file` as it is now, in place of one of its earlier states.
    pub fn refuse(&self, file: &Fingerprint, _: &Held) -> Result<()> {
        let (dir, refusal) = (self.dir(true)?, Refusal { login: self.key(), file: file.clone() });
        let mut all: Vec<Refusal> = dir.load("refused.json")?;
        all.retain(|r| (&r.login, &r.file.path) != (&refusal.login, &refusal.file.path));
        all.push(refusal);
        dir.save("refused.json", &all)
    }
}
