//! ZITADEL's OAuth endpoints, its login forms, and its password API.

use std::{collections::HashMap, fmt, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Method;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;

use super::{
    form::Form,
    store::{Access, now},
};
use crate::{
    identity::Login,
    origin::{Body, Origin, Reply, Web},
    policy::CLIENT_ID,
    secret::Secret,
};

const FORM_STEPS: usize = 8;
const TOKEN_POLLS: usize = 10;
const TOKEN_POLL_INTERVAL: Duration = Duration::from_secs(3);

pub struct Grant {
    pub access: Access,
    pub refresh: Option<Secret>,
}

/// The login form asked for the password again after it was posted.
#[derive(Debug)]
pub struct PasswordRefused;

impl fmt::Display for PasswordRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("zitadel refused the password")
    }
}

impl std::error::Error for PasswordRefused {}

pub trait Zitadel {
    async fn device_flow(&self, login: &Login, password: &Secret, scope: &str) -> Result<Grant>;
    /// `None` when ZITADEL refuses the refresh token.
    async fn refresh(&self, refresh: &Secret) -> Result<Option<Grant>>;
    async fn change_password(&self, token: &Secret, old: &Secret, new: &Secret) -> Result<()>;
}

pub struct Http {
    web: Web,
}

impl Http {
    pub fn new() -> Result<Http> {
        Ok(Http {
            web: Web::new(false)?,
        })
    }
}

#[derive(Deserialize)]
struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<Secret>,
    refresh_token: Option<Secret>,
    expires_in: Option<i64>,
    error: Option<String>,
}

enum Outcome {
    Granted(Grant),
    Refused(String),
}

impl TokenResponse {
    fn outcome(self) -> Result<Outcome> {
        match (self.access_token, self.error) {
            (Some(token), _) => {
                let expires_in = self
                    .expires_in
                    .context("token response has no expires_in")?;
                let expires_at = now()
                    .checked_add(expires_in)
                    .context("token expiry overflows")?;
                Ok(Outcome::Granted(Grant {
                    access: Access { token, expires_at },
                    refresh: self.refresh_token,
                }))
            }
            (None, Some(error)) => Ok(Outcome::Refused(error)),
            (None, None) => bail!("token response has neither access_token nor error"),
        }
    }
}

fn pairs(form: &[(&str, &str)]) -> Vec<(String, String)> {
    form.iter()
        .map(|&(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// A body that does not parse is quoted only on failure statuses, since success bodies carry codes and tokens.
fn json<T: DeserializeOwned>(reply: Reply) -> Result<T> {
    let (path, status) = (reply.url.path().to_owned(), reply.status);
    serde_json::from_str(&reply.text).map_err(|e| match status.is_success() {
        true => anyhow!("{path}: HTTP {status}: unreadable body: {e}"),
        false => anyhow!("{path}: HTTP {status}: {}", reply.text),
    })
}

impl Http {
    async fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<Reply> {
        self.web
            .send(
                Method::POST,
                Origin::Auth.url(path),
                Body::Form(&pairs(form)),
                None,
            )
            .await
    }

    async fn token(&self, form: &[(&str, &str)]) -> Result<Outcome> {
        json::<TokenResponse>(self.post("/oauth/v2/token", form).await?)?.outcome()
    }
}

impl Zitadel for Http {
    async fn device_flow(&self, login: &Login, password: &Secret, scope: &str) -> Result<Grant> {
        let form = [("client_id", CLIENT_ID), ("scope", scope)];
        let device: DeviceAuthorization =
            json(self.post("/oauth/v2/device_authorization", &form).await?)?;

        let login = login.to_string();
        let answers = HashMap::from([
            ("user_code", device.user_code.as_str()),
            ("loginName", &login),
            ("password", password.expose()),
        ]);
        let browser = Web::new(true)?;
        let start = Origin::Auth.check(&device.verification_uri)?;
        let mut page = browser.send(Method::GET, start, Body::Empty, None).await?;
        let mut password_posted = false;
        for _ in 0..FORM_STEPS {
            let Some(form) = Form::first(&page.text, &page.url)? else {
                break;
            };
            if form.asks("password") {
                if password_posted {
                    return Err(PasswordRefused.into());
                }
                password_posted = true;
            }
            let post = form.post(&answers, &page.url)?;
            page = browser
                .send(
                    Method::POST,
                    post.url,
                    Body::Form(post.fields.pairs()),
                    None,
                )
                .await?;
            if post.consents {
                break;
            }
        }

        let device_grant = [
            ("client_id", CLIENT_ID),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device.device_code),
        ];
        for _ in 0..TOKEN_POLLS {
            match self.token(&device_grant).await? {
                Outcome::Granted(grant) => return Ok(grant),
                Outcome::Refused(error)
                    if error == "authorization_pending" || error == "slow_down" => {}
                Outcome::Refused(error) => bail!("token: {error}"),
            }
            tokio::time::sleep(TOKEN_POLL_INTERVAL).await;
        }
        bail!(
            "device flow never authorized; last page {}",
            page.url.path()
        )
    }

    async fn refresh(&self, refresh: &Secret) -> Result<Option<Grant>> {
        let form = [
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose()),
        ];
        Ok(match self.token(&form).await? {
            Outcome::Granted(grant) => Some(grant),
            Outcome::Refused(error) => {
                crate::show::warn(&format!(
                    "zitadel refused the refresh token ({error}); signing in"
                ));
                None
            }
        })
    }

    async fn change_password(&self, token: &Secret, old: &Secret, new: &Secret) -> Result<()> {
        let body = json!({ "oldPassword": old.expose(), "newPassword": new.expose() });
        let url = Origin::Auth.url("/auth/v1/users/me/password");
        let reply = self
            .web
            .send(Method::PUT, url, Body::Json(&body), Some(token))
            .await?;
        if !reply.status.is_success() {
            bail!(
                "changing the password: HTTP {}: {}",
                reply.status,
                reply.text
            );
        }
        Ok(())
    }
}
