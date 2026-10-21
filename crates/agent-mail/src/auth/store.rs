//! `$XDG_STATE_HOME/agent-mail/tokens.json`, shared by every session of the user, and the user's locks.
//!
//! The locks live under the home directory of the user's passwd entry rather than anything from the environment,
//! so every session of one user on one host takes the same lock files.

use std::{
    fs::{self, DirBuilder, File, OpenOptions, Permissions},
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    config::Fingerprint,
    home::home,
    random::{ALPHANUMERIC, Draws},
    secret::Secret,
};

const EXPIRY_MARGIN_SECS: i64 = 60;
/// Long enough for another session's device flow, which polls for up to 30 s after its form walk.
const LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const LOCK_POLL: Duration = Duration::from_millis(200);
/// Serialises everything that reads or writes the tokens, the refusals or the password file.
const AUTH_LOCK: &str = "auth.lock";
const CLAIMS_LOCK: &str = "claims.lock";

#[derive(Clone, Serialize, Deserialize)]
pub struct Access {
    pub token: Secret,
    pub expires_at: i64,
}

impl Access {
    pub fn fresh(&self, now: i64) -> bool {
        now + EXPIRY_MARGIN_SECS < self.expires_at
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Tokens {
    pub access: Option<Access>,
    pub refresh: Option<Secret>,
}

/// Tokens are kept with the login they belong to; another login's tokens read as none.
#[derive(Serialize, Deserialize)]
struct OnDisk {
    login: Option<String>,
    #[serde(flatten)]
    tokens: Tokens,
}

/// A password file ZITADEL refused for a login, in the state it was in then.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Refusal {
    login: String,
    file: Fingerprint,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn locks_dir() -> Result<PathBuf> {
    Ok(home()?.join(".local/state/agent-mail/locks"))
}

fn private_dir(dir: &Path) -> Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| dir.display().to_string())?;
    fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    Ok(())
}

pub struct Store {
    dir: PathBuf,
    locks: PathBuf,
    login: String,
}

/// An exclusive lock on a lock file, released when dropped.
pub struct Held {
    _file: Option<File>,
}

impl Held {
    #[cfg(test)]
    pub fn nothing() -> Held {
        Held { _file: None }
    }
}

/// The tokens, readable and writable only while the auth lock is held: ZITADEL rotates the refresh token on each
/// use, so read, refresh and write must not interleave across sessions.
pub struct TokensFile<'a> {
    store: &'a Store,
    _held: &'a Held,
}

impl Store {
    pub fn at(dir: PathBuf, locks: PathBuf, login: impl ToString) -> Self {
        Store {
            dir,
            locks,
            login: login.to_string().to_lowercase(),
        }
    }

    pub async fn auth_lock(&self) -> Result<Held> {
        self.hold(AUTH_LOCK).await
    }

    pub async fn claims_lock(&self) -> Result<Held> {
        self.hold(CLAIMS_LOCK).await
    }

    pub fn tokens<'a>(&'a self, held: &'a Held) -> TokensFile<'a> {
        TokensFile {
            store: self,
            _held: held,
        }
    }

    async fn hold(&self, name: &str) -> Result<Held> {
        private_dir(&self.locks)?;
        let path = self.locks.join(name);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| path.display().to_string())?;
        let deadline = tokio::time::Instant::now() + LOCK_TIMEOUT;
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(Held { _file: Some(lock) }),
                Err(std::fs::TryLockError::WouldBlock)
                    if tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(LOCK_POLL).await;
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    bail!(
                        "another agent-mail has held {} for over {} s",
                        path.display(),
                        LOCK_TIMEOUT.as_secs()
                    )
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(e).with_context(|| format!("locking {}", path.display()));
                }
            }
        }
    }

    /// Beside the locks rather than in the state directory, so every process of the user sees the same refusals.
    fn refusals_path(&self) -> PathBuf {
        self.locks.join("refused.json")
    }

    fn refusals(&self) -> Result<Vec<Refusal>> {
        let path = self.refusals_path();
        match fs::read(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(vec![]),
            read => serde_json::from_slice(&read.with_context(|| path.display().to_string())?)
                .with_context(|| path.display().to_string()),
        }
    }

    /// Whether ZITADEL refused this login with the password file in exactly this state before, so a stale
    /// password is not posted again and again.
    pub fn was_refused(&self, file: &Fingerprint, _: &Held) -> Result<bool> {
        let refusal = Refusal {
            login: self.login.clone(),
            file: file.clone(),
        };
        Ok(self.refusals()?.contains(&refusal))
    }

    pub fn record_refusal(&self, file: Fingerprint, _: &Held) -> Result<()> {
        let refusal = Refusal {
            login: self.login.clone(),
            file,
        };
        let mut refusals = self.refusals()?;
        if !refusals.contains(&refusal) {
            refusals.push(refusal);
        }
        write_private(
            &self.locks,
            &self.refusals_path(),
            &serde_json::to_vec(&refusals)?,
        )
    }
}

/// Writes through a uniquely named temporary file and a rename, so a reader never sees a partial file.
fn write_private(dir: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    private_dir(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    let staged = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        Draws::default().string(ALPHANUMERIC, 8)?
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .with_context(|| staged.display().to_string())?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&staged, path).with_context(|| path.display().to_string())
}

impl TokensFile<'_> {
    fn path(&self) -> PathBuf {
        self.store.dir.join("tokens.json")
    }

    pub fn read(&self) -> Result<Tokens> {
        let path = self.path();
        match fs::read(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Tokens::default()),
            read => {
                let bytes = read.with_context(|| path.display().to_string())?;
                let stored: OnDisk =
                    serde_json::from_slice(&bytes).with_context(|| path.display().to_string())?;
                let ours = stored
                    .login
                    .is_some_and(|l| l.eq_ignore_ascii_case(&self.store.login));
                Ok(if ours {
                    stored.tokens
                } else {
                    Tokens::default()
                })
            }
        }
    }

    pub fn write(&self, tokens: Tokens) -> Result<()> {
        let stored = OnDisk {
            login: Some(self.store.login.clone()),
            tokens,
        };
        write_private(&self.store.dir, &self.path(), &serde_json::to_vec(&stored)?)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-mail-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    pub fn store(dir: &Path, login: &str) -> Store {
        Store::at(dir.join("state"), dir.join("locks"), login)
    }

    #[test]
    fn access_is_fresh_until_the_margin() {
        let access = Access {
            token: Secret::new("t"),
            expires_at: 1_000,
        };
        assert!(access.fresh(900));
        assert!(access.fresh(939));
        assert!(!access.fresh(940));
        assert!(!access.fresh(2_000));
    }

    fn tokens(access: &str) -> Tokens {
        Tokens {
            access: Some(Access {
                token: Secret::new(access),
                expires_at: 5,
            }),
            refresh: Some(Secret::new("r")),
        }
    }

    #[tokio::test]
    async fn tokens_round_trip_with_private_modes() {
        let dir = scratch("store");
        let store = store(&dir, "agent@example.org");
        let held = store.auth_lock().await.unwrap();
        let file = store.tokens(&held);
        assert!(file.read().unwrap().access.is_none());
        file.write(tokens("a")).unwrap();
        let back = file.read().unwrap();
        assert_eq!(back.access.unwrap().token, Secret::new("a"));
        assert_eq!(back.refresh, Some(Secret::new("r")));
        let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.join("state")), 0o700);
        assert_eq!(mode(dir.join("locks")), 0o700);
        assert_eq!(mode(dir.join("state/tokens.json")), 0o600);
        assert!(
            fs::read_dir(dir.join("state")).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn another_logins_tokens_read_as_none() {
        let dir = scratch("logins");
        let agents = store(&dir, "agent@example.org");
        let held = agents.auth_lock().await.unwrap();
        agents.tokens(&held).write(tokens("agents")).unwrap();
        drop(held);
        let other = store(&dir, "ada@agent.example.org");
        let held = other.auth_lock().await.unwrap();
        let read = other.tokens(&held).read().unwrap();
        assert!(read.access.is_none() && read.refresh.is_none());
        other.tokens(&held).write(tokens("adas")).unwrap();
        drop(held);
        let again = store(&dir, "Agent@Example.org");
        let held = again.auth_lock().await.unwrap();
        assert!(again.tokens(&held).read().unwrap().access.is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn files_without_a_login_read_as_none() {
        let dir = scratch("legacy");
        fs::create_dir_all(dir.join("state")).unwrap();
        fs::write(
            dir.join("state/tokens.json"),
            r#"{"access":{"token":"t","expires_at":9},"refresh":"r"}"#,
        )
        .unwrap();
        let store = store(&dir, "agent@example.org");
        let held = store.auth_lock().await.unwrap();
        assert!(store.tokens(&held).read().unwrap().access.is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_locks_live_under_the_passwd_home() {
        let dir = locks_dir().unwrap();
        assert!(
            dir.is_absolute() && dir.ends_with(".local/state/agent-mail/locks"),
            "{}",
            dir.display()
        );
    }
}
