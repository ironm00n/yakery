//! Access tokens for Stalwart: cached, refreshed, or signed in for afresh.

mod form;
pub mod store;
mod zitadel;

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    config::{Config, ConfigError, Fingerprint, PasswordFile, xdg_dir},
    identity::Login,
    policy::{SCOPE, ZITADEL_API_SCOPE},
    secret::Secret,
};
use store::{Access, Held, Store, Tokens, locks_dir, now};
pub use zitadel::PasswordRefused;
use zitadel::{Grant, Http, Zitadel};

const ROTATED_PASSWORD_LEN: usize = 40;

/// Errors that waiting out will not fix: retrying would only post a refused password again.
pub fn is_permanent(e: &anyhow::Error) -> bool {
    e.is::<PasswordRefused>() || e.is::<ConfigError>()
}

pub struct Auth<Z = Http> {
    zitadel: Z,
    store: Store,
    pub login: Login,
    password: PasswordFile,
}

impl Auth {
    pub fn new(config: Config) -> Result<Auth> {
        let store = Store::at(
            xdg_dir("XDG_STATE_HOME", ".local/state")?,
            locks_dir()?,
            &config.login,
        );
        Ok(Auth {
            zitadel: Http::new()?,
            store,
            login: config.login,
            password: config.password,
        })
    }
}

fn tokens(grant: Grant) -> Tokens {
    Tokens {
        access: Some(grant.access),
        refresh: grant.refresh,
    }
}

impl<Z: Zitadel> Auth<Z> {
    /// Serialises claims on mail between the waits of this host.
    pub async fn claims_lock(&self) -> Result<Held> {
        self.store.claims_lock().await
    }

    pub async fn access(&self) -> Result<Access> {
        let held = self.store.auth_lock().await?;
        let tokens_file = self.store.tokens(&held);
        let Tokens { access, refresh } = tokens_file.read()?;
        if let Some(access) = access.filter(|a| a.fresh(now())) {
            return Ok(access);
        }
        let refreshed = match refresh {
            Some(refresh) => self.zitadel.refresh(&refresh).await?.map(|grant| Grant {
                refresh: grant.refresh.or(Some(refresh)),
                ..grant
            }),
            None => None,
        };
        let grant = match refreshed {
            Some(grant) => grant,
            None => self.sign_in(SCOPE, &held).await?.0,
        };
        let access = grant.access.clone();
        tokens_file.write(tokens(grant))?;
        Ok(access)
    }

    /// Drops an access token Stalwart refused, unless another session has already replaced it.
    pub async fn reject(&self, token: &Secret) -> Result<()> {
        let held = self.store.auth_lock().await?;
        let tokens_file = self.store.tokens(&held);
        let mut stored = tokens_file.read()?;
        if stored.access.as_ref().is_some_and(|a| a.token == *token) {
            stored.access = None;
            tokens_file.write(stored)?;
        }
        Ok(())
    }

    /// Posts a password, unless ZITADEL refused this login with this file in this very state before. Only a refusal
    /// at the password step is recorded.
    async fn post(
        &self,
        file: Fingerprint,
        password: &Secret,
        scope: &str,
        held: &Held,
    ) -> Result<Grant> {
        if self.store.was_refused(&file, held)? {
            return Err(anyhow!(PasswordRefused).context(format!(
                "the password in {} was refused for {} before; not posting it again until the file changes",
                file.path().display(),
                self.login
            )));
        }
        match self.zitadel.device_flow(&self.login, password, scope).await {
            Err(e) if e.is::<PasswordRefused>() => {
                self.store.record_refusal(file, held)?;
                Err(e)
            }
            posted => posted,
        }
    }

    /// Falls back to a rotated password left in `<password-file>.new`, which a rotation that died between
    /// changing the password and renaming the file leaves behind.
    async fn sign_in(&self, scope: &str, held: &Held) -> Result<(Grant, Secret)> {
        let file = self.password.fingerprint()?;
        let current = self.password.read()?;
        let refused = match self.post(file, &current, scope, held).await {
            Err(e) if e.is::<PasswordRefused>() => e,
            signed_in => return Ok((signed_in?, current)),
        };
        let Some(staged) = self.password.read_staged()? else {
            return Err(refused);
        };
        let grant = self
            .post(self.password.staged_fingerprint()?, &staged, scope, held)
            .await?;
        self.password.promote(&staged, held)?;
        Ok((grant, staged))
    }

    /// Runs under the auth lock throughout, so two rotations cannot interleave and no sign-in reads a half-done one.
    pub async fn rotate_password(&self) -> Result<()> {
        let held = self.store.auth_lock().await?;
        let (admin, old) = self
            .sign_in(&format!("{SCOPE} {ZITADEL_API_SCOPE}"), &held)
            .await?;
        let new = Secret::password(ROTATED_PASSWORD_LEN)?;
        self.password.stage(&new, &held)?;
        let changed = self
            .zitadel
            .change_password(&admin.access.token, &old, &new)
            .await;
        if let Err(put) = changed {
            return self.settle_failed_change(put, &old, &new, &held).await;
        }
        self.password.promote(&new, &held)?;
        let grant = self
            .post(self.password.fingerprint()?, &new, SCOPE, &held)
            .await
            .context("the password changed, but signing in with it failed")?;
        self.store.tokens(&held).write(tokens(grant))
    }

    /// The PUT failed, but ZITADEL may have applied it and only the reply was lost: whichever password signs in
    /// decides which file survives.
    async fn settle_failed_change(
        &self,
        put: anyhow::Error,
        old: &Secret,
        new: &Secret,
        held: &Held,
    ) -> Result<()> {
        let refused = |r: &Result<Grant>| matches!(r, Err(e) if e.is::<PasswordRefused>());
        let with_new = self
            .post(self.password.staged_fingerprint()?, new, SCOPE, held)
            .await;
        if let Ok(grant) = with_new {
            self.password.promote(new, held)?;
            crate::show::warn(&format!(
                "changing the password reported {put:#}, but the new password works"
            ));
            return self.store.tokens(held).write(tokens(grant));
        }
        if refused(&with_new) {
            let with_old = self
                .post(self.password.fingerprint()?, old, SCOPE, held)
                .await;
            if let Ok(grant) = with_old {
                self.password.discard(held)?;
                self.store.tokens(held).write(tokens(grant))?;
                return Err(put.context("the password is unchanged"));
            }
            if refused(&with_old) {
                bail!(
                    "changing the password failed ({put:#}) and neither password signs in; kept both {}",
                    self.password.describe()
                );
            }
        }
        bail!(
            "changing the password failed ({put:#}) and checking which password works failed too; kept both {}",
            self.password.describe()
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, path::PathBuf};

    use super::{store::tests::scratch, *};

    #[derive(Clone, Copy, Default, PartialEq)]
    enum Change {
        #[default]
        Applies,
        Refused,
        AppliedButReplyLost,
        /// The account's password is neither the old nor the new one afterwards.
        ChangedElsewhere,
    }

    #[derive(Default)]
    struct Fake {
        password: RefCell<String>,
        change: Change,
        refresh_valid: Option<&'static str>,
        calls: RefCell<Vec<String>>,
    }

    fn grant(token: &str, refresh: Option<&str>) -> Grant {
        Grant {
            access: Access {
                token: Secret::new(token),
                expires_at: now() + 3600,
            },
            refresh: refresh.map(Secret::new),
        }
    }

    impl Zitadel for Fake {
        async fn device_flow(&self, _: &Login, password: &Secret, scope: &str) -> Result<Grant> {
            self.calls
                .borrow_mut()
                .push(format!("sign in {} {scope}", password.expose()));
            if password.expose() != *self.password.borrow() {
                return Err(PasswordRefused.into());
            }
            Ok(grant(
                &format!("access-{}", password.expose()),
                Some("refresh-1"),
            ))
        }

        async fn refresh(&self, refresh: &Secret) -> Result<Option<Grant>> {
            self.calls
                .borrow_mut()
                .push(format!("refresh {}", refresh.expose()));
            Ok((Some(refresh.expose()) == self.refresh_valid)
                .then(|| grant("access-refreshed", None)))
        }

        async fn change_password(&self, token: &Secret, old: &Secret, new: &Secret) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("change {} {}", token.expose(), old.expose()));
            match self.change {
                Change::Applies => *self.password.borrow_mut() = new.expose().into(),
                Change::Refused => bail!("HTTP 400"),
                Change::AppliedButReplyLost => {
                    *self.password.borrow_mut() = new.expose().into();
                    bail!("connection reset");
                }
                Change::ChangedElsewhere => {
                    *self.password.borrow_mut() = "someone else's".into();
                    bail!("connection reset");
                }
            }
            Ok(())
        }
    }

    struct Fixture {
        dir: PathBuf,
        auth: Auth<Fake>,
    }

    impl Fixture {
        fn new(name: &str, file: &str, fake: Fake) -> Fixture {
            let dir = scratch(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("password"), file).unwrap();
            let login: Login = "ada@agent.example.org".parse().unwrap();
            let auth = Auth {
                zitadel: fake,
                store: Store::at(dir.join("state"), dir.join("locks"), &login),
                login,
                password: PasswordFile::at(dir.join("password")),
            };
            Fixture { dir, auth }
        }

        fn file(&self, name: &str) -> Option<String> {
            fs::read_to_string(self.dir.join(name)).ok()
        }

        fn calls(&self) -> Vec<String> {
            self.auth.zitadel.calls.borrow().clone()
        }

        async fn stored(&self) -> Tokens {
            let held = self.auth.store.auth_lock().await.unwrap();
            self.auth.store.tokens(&held).read().unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn fake(password: &str) -> Fake {
        Fake {
            password: RefCell::new(password.into()),
            ..Fake::default()
        }
    }

    fn changing(password: &str, change: Change) -> Fake {
        Fake {
            change,
            ..fake(password)
        }
    }

    #[tokio::test]
    async fn access_signs_in_once_then_reuses_the_cached_token() {
        let f = Fixture::new("cache", "old\n", fake("old"));
        assert_eq!(
            f.auth.access().await.unwrap().token,
            Secret::new("access-old")
        );
        assert_eq!(
            f.auth.access().await.unwrap().token,
            Secret::new("access-old")
        );
        assert_eq!(f.calls(), [format!("sign in old {SCOPE}")]);
    }

    #[tokio::test]
    async fn a_rejected_token_is_refreshed_and_the_refresh_token_kept() {
        let f = Fixture::new(
            "refresh",
            "old",
            Fake {
                refresh_valid: Some("refresh-1"),
                ..fake("old")
            },
        );
        let first = f.auth.access().await.unwrap();
        f.auth.reject(&first.token).await.unwrap();
        assert_eq!(
            f.auth.access().await.unwrap().token,
            Secret::new("access-refreshed")
        );
        assert_eq!(f.stored().await.refresh, Some(Secret::new("refresh-1")));
        assert_eq!(f.calls()[1..], ["refresh refresh-1".to_string()]);
    }

    #[tokio::test]
    async fn a_refused_refresh_token_falls_back_to_signing_in() {
        let f = Fixture::new("refused", "old", fake("old"));
        let first = f.auth.access().await.unwrap();
        f.auth.reject(&first.token).await.unwrap();
        f.auth.access().await.unwrap();
        assert_eq!(
            f.calls()[1..],
            [
                "refresh refresh-1".to_string(),
                format!("sign in old {SCOPE}")
            ]
        );
    }

    #[tokio::test]
    async fn reject_leaves_a_token_another_session_replaced() {
        let f = Fixture::new("replaced", "old", fake("old"));
        f.auth.access().await.unwrap();
        f.auth.reject(&Secret::new("someone-elses")).await.unwrap();
        assert!(f.stored().await.access.is_some());
    }

    #[tokio::test]
    async fn rotate_changes_the_password_and_stores_tokens_for_the_new_one() {
        let f = Fixture::new("rotate", "old\n", fake("old"));
        f.auth.rotate_password().await.unwrap();
        let new = f.file("password").unwrap();
        assert_eq!(new.len(), ROTATED_PASSWORD_LEN);
        assert_eq!(*f.auth.zitadel.password.borrow(), new);
        assert_eq!(f.file("password.new"), None);
        assert_eq!(
            f.calls(),
            [
                format!("sign in old {SCOPE} {ZITADEL_API_SCOPE}"),
                "change access-old old".into(),
                format!("sign in {new} {SCOPE}"),
            ]
        );
        assert_eq!(
            f.stored().await.access.unwrap().token,
            Secret::new(format!("access-{new}"))
        );
    }

    #[tokio::test]
    async fn a_refused_change_keeps_the_old_password() {
        let f = Fixture::new("refuse", "old\n", changing("old", Change::Refused));
        let err = f.auth.rotate_password().await.err().unwrap();
        assert!(format!("{err:#}").contains("unchanged"), "{err:#}");
        assert_eq!(f.file("password").as_deref(), Some("old\n"));
        assert_eq!(f.file("password.new"), None);
        assert_eq!(
            f.stored().await.access.unwrap().token,
            Secret::new("access-old")
        );
    }

    #[tokio::test]
    async fn a_change_whose_reply_was_lost_is_kept() {
        let f = Fixture::new(
            "lost",
            "old\n",
            changing("old", Change::AppliedButReplyLost),
        );
        f.auth.rotate_password().await.unwrap();
        let new = f.file("password").unwrap();
        assert_eq!(*f.auth.zitadel.password.borrow(), new);
        assert_eq!(f.file("password.new"), None);
        assert_eq!(
            f.stored().await.access.unwrap().token,
            Secret::new(format!("access-{new}"))
        );
    }

    #[tokio::test]
    async fn when_neither_password_works_both_files_stay() {
        let f = Fixture::new(
            "neither",
            "old\n",
            changing("old", Change::ChangedElsewhere),
        );
        let err = format!("{:#}", f.auth.rotate_password().await.err().unwrap());
        assert!(
            err.contains("neither password signs in") && err.contains("password.new"),
            "{err}"
        );
        assert_eq!(f.file("password").as_deref(), Some("old\n"));
        assert_eq!(
            f.file("password.new").map(|p| p.len()),
            Some(ROTATED_PASSWORD_LEN)
        );
    }

    #[tokio::test]
    async fn a_refused_password_retries_with_a_staged_one_and_promotes_it() {
        let f = Fixture::new("crash", "old", fake("staged"));
        fs::write(f.dir.join("password.new"), "staged").unwrap();
        assert_eq!(
            f.auth.access().await.unwrap().token,
            Secret::new("access-staged")
        );
        assert_eq!(f.file("password").as_deref(), Some("staged"));
        assert_eq!(f.file("password.new"), None);
    }

    #[tokio::test]
    async fn a_refused_password_is_not_posted_again_by_any_process_until_the_file_changes() {
        let f = Fixture::new("wrong", "old", fake("other"));
        let first = f.auth.access().await.err().unwrap();
        assert!(first.is::<PasswordRefused>() && is_permanent(&first));
        let login: Login = "ada@agent.example.org".parse().unwrap();
        let next_process = Auth {
            zitadel: fake("other"),
            store: Store::at(f.dir.join("state"), f.dir.join("locks"), &login),
            login,
            password: PasswordFile::at(f.dir.join("password")),
        };
        let second = next_process.access().await.err().unwrap();
        assert!(is_permanent(&second));
        assert!(next_process.zitadel.calls.borrow().is_empty());
        assert_eq!(f.calls().len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(f.dir.join("password"), "other").unwrap();
        assert!(next_process.access().await.is_ok());
    }
}
