//! The account the sync signs in to Stalwart as.

use anyhow::{Context, Result};
use private::{Dir, Secret};
use reqwest::{RequestBuilder, Response, StatusCode};

use crate::policy::Policy;

const APP_PASSWORD: &str = "app-password";
/// sysexits.h's: systemd starts the sync again, with the new app password.
const EX_TEMPFAIL: i32 = 75;

/// The account the sync signs in to Stalwart as, with the app password systemd passes in.
pub struct Account {
    user: String,
    password: Secret,
}

impl Account {
    pub fn read(policy: &Policy, credentials: &Dir) -> Result<Account> {
        let bytes = credentials.read(APP_PASSWORD)?.context("no app-password credential")?;
        let password = Secret::new(String::from_utf8(bytes).context("the app password")?.trim());
        Ok(Account { user: policy.account.clone(), password })
    }

    pub fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        request.basic_auth(&self.user, Some(self.password.expose()))
    }

    /// A 401 means the app password changed, so the process exits for systemd to start it with the new one.
    pub async fn send(&self, request: RequestBuilder, what: &str) -> Result<Response> {
        let reply = self.auth(request).send().await.with_context(|| format!("stalwart {what}"))?;
        if reply.status() == StatusCode::UNAUTHORIZED {
            yakery_log::warn!("{} was refused; exiting to read the app password again", self.user);
            std::process::exit(EX_TEMPFAIL);
        }
        Ok(reply)
    }
}
