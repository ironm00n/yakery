//! `$XDG_CONFIG_HOME/agent-mail/config.json`: the login and where its password lives.

use std::{
    env, fmt,
    fs::{self, OpenOptions, Permissions},
    io::{ErrorKind, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{auth::store::Held, identity::Login, secret::Secret};

pub struct Config {
    pub login: Login,
    pub password: PasswordFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Keys {
    login: Option<String>,
    password_file: Option<String>,
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = xdg_dir("XDG_CONFIG_HOME", ".config")?.join("config.json");
        match fs::read_to_string(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Err(incomplete(&path).into()),
            read => Config::parse(&path, &read.with_context(|| path.display().to_string())?),
        }
    }

    fn parse(path: &Path, text: &str) -> Result<Config> {
        let keys: Keys = serde_json::from_str(text)
            .map_err(|e| ConfigError(format!("{}: {e}", incomplete(path))))?;
        let (Some(login), Some(password_file)) = (keys.login, keys.password_file) else {
            return Err(incomplete(path).into());
        };
        Ok(Config {
            login: login
                .parse()
                .map_err(|e| ConfigError(format!("{}: {e:#}", path.display())))?,
            password: PasswordFile(expand_home(&password_file)?),
        })
    }
}

/// A config or password file that is missing or malformed; waiting will not fix it.
#[derive(Debug)]
pub struct ConfigError(String);

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        ConfigError(message.into())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn incomplete(path: &Path) -> ConfigError {
    ConfigError(format!(
        "{} must hold \"login\" and \"password-file\"",
        path.display()
    ))
}

/// `$<var>/agent-mail`, or `~/<fallback>/agent-mail` when the variable is unset or relative.
pub fn xdg_dir(var: &str, fallback: &str) -> Result<PathBuf> {
    let base = match env::var_os(var).map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        _ => home()?.join(fallback),
    };
    Ok(base.join("agent-mail"))
}

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn expand_home(path: &str) -> Result<PathBuf> {
    Ok(match path.strip_prefix("~/") {
        Some(rest) => home()?.join(rest),
        None => PathBuf::from(path),
    })
}

/// The password file, and beside it `<file>.new`, which holds a rotated password until the rotation is confirmed.
pub struct PasswordFile(PathBuf);

impl PasswordFile {
    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        PasswordFile(path)
    }

    fn staged_path(&self) -> PathBuf {
        let mut path = self.0.clone().into_os_string();
        path.push(".new");
        path.into()
    }

    pub fn read(&self) -> Result<Secret> {
        read_password(&self.0)
    }

    pub fn read_staged(&self) -> Result<Option<Secret>> {
        let path = self.staged_path();
        match fs::metadata(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            _ => read_password(&path).map(Some),
        }
    }

    pub fn stage(&self, password: &Secret, _: &Held) -> Result<()> {
        let path = self.staged_path();
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| path.display().to_string())?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        file.write_all(password.expose().as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    /// Moves `<file>.new` over the file, but only if it holds `expected`, the password that just signed in. Another
    /// session may have promoted it first: if `.new` is gone and the file holds `expected`, that is success.
    pub fn promote(&self, expected: &Secret, _: &Held) -> Result<()> {
        let staged = self.staged_path();
        match self.read_staged()? {
            Some(password) if password == *expected => fs::rename(&staged, &self.0)
                .with_context(|| format!("promoting {}", staged.display())),
            Some(_) => bail!(
                "{} holds another password than the one that signed in; leaving it",
                staged.display()
            ),
            None if self.read().is_ok_and(|p| p == *expected) => Ok(()),
            None => bail!(
                "{} is gone, and {} does not hold the password that signed in",
                staged.display(),
                self.0.display()
            ),
        }
    }

    pub fn fingerprint(&self) -> Result<Fingerprint> {
        Fingerprint::of(&self.0)
    }

    pub fn staged_fingerprint(&self) -> Result<Fingerprint> {
        Fingerprint::of(&self.staged_path())
    }

    pub fn describe(&self) -> String {
        format!("{} and {}", self.0.display(), self.staged_path().display())
    }

    pub fn discard(&self, _: &Held) -> Result<()> {
        fs::remove_file(self.staged_path())
            .with_context(|| format!("removing {}", self.staged_path().display()))
    }
}

/// Which file a password came from, and in what state, without anything derived from the password itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    path: PathBuf,
    dev: u64,
    ino: u64,
    mtime: i64,
    mtime_nsec: i64,
    size: u64,
}

impl Fingerprint {
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn of(path: &Path) -> Result<Fingerprint> {
        let meta =
            fs::metadata(path).map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
        Ok(Fingerprint {
            path: path.to_owned(),
            dev: meta.dev(),
            ino: meta.ino(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
            size: meta.size(),
        })
    }
}

fn read_password(path: &Path) -> Result<Secret> {
    let text =
        fs::read_to_string(path).map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    let password = text.trim_end_matches(['\n', '\r']);
    if password.is_empty() {
        return Err(ConfigError(format!("{} is empty", path.display())).into());
    }
    Ok(Secret::new(password))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_config_names_the_path_and_both_keys() {
        let path = Path::new("/c/agent-mail/config.json");
        for text in [
            "{}",
            r#"{"login": "agent@ironmoon.dev"}"#,
            "not json",
            r#"{"login": "a@b", "password_file": "x"}"#,
        ] {
            let message = format!("{:#}", Config::parse(path, text).err().unwrap());
            assert!(
                message.starts_with(
                    "/c/agent-mail/config.json must hold \"login\" and \"password-file\""
                ),
                "{message}"
            );
        }
    }

    #[test]
    fn complete_config_parses() {
        let config = Config::parse(
            Path::new("config.json"),
            r#"{"login": "ada@agent.ironmoon.dev", "password-file": "/run/ada"}"#,
        )
        .unwrap();
        assert_eq!(config.login.to_string(), "ada@agent.ironmoon.dev");
        assert_eq!(config.password.0, PathBuf::from("/run/ada"));
    }
}

#[cfg(test)]
mod password_file_tests {
    use super::*;

    fn scratch(name: &str) -> (PathBuf, PasswordFile) {
        let dir =
            std::env::temp_dir().join(format!("agent-mail-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pw"), "old").unwrap();
        let file = PasswordFile::at(dir.join("pw"));
        (dir, file)
    }

    #[test]
    fn a_promote_another_session_finished_is_success() {
        let (dir, file) = scratch("promote");
        let new = Secret::new("new-password");
        file.stage(&new, &Held::nothing()).unwrap();
        file.promote(&new, &Held::nothing()).unwrap();
        file.promote(&new, &Held::nothing()).unwrap();
        assert!(
            file.promote(&Secret::new("other"), &Held::nothing())
                .is_err()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn promote_refuses_a_staged_password_other_than_the_one_that_signed_in() {
        let (dir, file) = scratch("promote-other");
        let (a, b) = (Secret::new("new-a"), Secret::new("new-b"));
        file.stage(&a, &Held::nothing()).unwrap();
        file.stage(&b, &Held::nothing()).unwrap();
        assert!(file.promote(&a, &Held::nothing()).is_err());
        assert_eq!(fs::read_to_string(dir.join("pw")).unwrap(), "old");
        assert_eq!(fs::read_to_string(dir.join("pw.new")).unwrap(), "new-b");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_fingerprint_changes_when_the_file_does() {
        let (dir, file) = scratch("fingerprint");
        let before = file.fingerprint().unwrap();
        assert_eq!(before, file.fingerprint().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("pw"), "changed").unwrap();
        assert_ne!(before, file.fingerprint().unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
}
