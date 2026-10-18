//! The two origins agent-mail talks to, and an HTTP client that cannot leave them.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{Client, Method, StatusCode, Url, header::LOCATION, redirect};
use serde_json::Value;

use crate::{
    policy::{AUTH_HOST, JMAP_HOST},
    secret::Secret,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const REDIRECTS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Auth,
    Mail,
}

impl Origin {
    fn host(self) -> &'static str {
        match self {
            Origin::Auth => AUTH_HOST,
            Origin::Mail => JMAP_HOST,
        }
    }

    pub fn base(self) -> String {
        format!("https://{}", self.host())
    }

    pub fn url(self, path: &str) -> Pinned {
        self.check(&format!("{}{path}", self.base()))
            .expect("compiled-in paths stay on their origin")
    }

    /// Parses `url` and refuses it unless it is `https://<host>` on the default port with no user info.
    pub fn check(self, url: &str) -> Result<Pinned> {
        let parsed =
            Url::parse(url).with_context(|| format!("{} is not a URL", url.escape_debug()))?;
        let on_origin = parsed.scheme() == "https"
            && parsed.host_str() == Some(self.host())
            && parsed.port().is_none()
            && parsed.username().is_empty()
            && parsed.password().is_none();
        if !on_origin {
            bail!(
                "refusing {}: it is not on {}",
                url.escape_debug(),
                self.base()
            );
        }
        Ok(Pinned {
            origin: self,
            url: parsed,
        })
    }
}

/// A URL on one of the two origins; the only constructors check it.
#[derive(Clone, Debug)]
pub struct Pinned {
    origin: Origin,
    url: Url,
}

impl Pinned {
    /// Resolves a link found on this page; it must stay on the same origin.
    pub fn join(&self, reference: &str) -> Result<Pinned> {
        let joined = self
            .url
            .join(reference)
            .with_context(|| format!("{} is not a link", reference.escape_debug()))?;
        self.origin.check(joined.as_str())
    }

    pub fn path(&self) -> &str {
        self.url.path()
    }

    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }
}

pub enum Body<'a> {
    Empty,
    Form(&'a [(String, String)]),
    Json(&'a Value),
}

pub struct Reply {
    pub url: Pinned,
    pub status: StatusCode,
    pub text: String,
}

/// Where a redirect leads, and whether the request keeps its method and body (only 307 and 308 do).
fn redirect(from: &Pinned, status: StatusCode, location: Option<&str>) -> Result<(Pinned, bool)> {
    let location = location
        .with_context(|| format!("{}: HTTP {status} without a usable Location", from.path()))?;
    let keeps_body = matches!(
        status,
        StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
    );
    Ok((from.join(location)?, keeps_body))
}

/// Follows redirects itself, and only within the origin of the request.
pub struct Web {
    client: Client,
}

impl Web {
    pub fn new(cookies: bool) -> Result<Web> {
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .cookie_store(cookies)
            .build()?;
        Ok(Web { client })
    }

    pub async fn send(
        &self,
        method: Method,
        url: Pinned,
        body: Body<'_>,
        bearer: Option<&Secret>,
    ) -> Result<Reply> {
        let (mut method, mut url, mut body) = (method, url, body);
        for _ in 0..REDIRECTS {
            let mut request = self.client.request(method.clone(), url.url.clone());
            if let Some(token) = bearer {
                request = request.bearer_auth(token.expose());
            }
            request = match body {
                Body::Empty => request,
                Body::Form(fields) => request.form(fields),
                Body::Json(json) => request.json(json),
            };
            let response = request
                .send()
                .await
                .with_context(|| url.path().to_owned())?;
            let status = response.status();
            if !status.is_redirection() {
                return Ok(Reply {
                    url,
                    status,
                    text: response.text().await?,
                });
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|l| l.to_str().ok());
            let (next, keeps_body) = redirect(&url, status, location)?;
            url = next;
            if !keeps_body {
                (method, body) = (Method::GET, Body::Empty);
            }
        }
        bail!("{}: more than {REDIRECTS} redirects", url.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_origins_are_accepted() {
        assert!(
            Origin::Auth
                .check("https://auth.ironmoon.dev/ui/login/device")
                .is_ok()
        );
        assert!(
            Origin::Mail
                .check("https://mail.ironmoon.dev/jmap/upload/{accountId}/")
                .is_ok()
        );
        for bad in [
            "http://auth.ironmoon.dev/x",
            "https://auth.ironmoon.dev:8443/x",
            "https://user:pw@auth.ironmoon.dev/x",
            "https://auth.ironmoon.dev.evil.com/x",
            "https://evil.com/?https://auth.ironmoon.dev",
            "https://mail.ironmoon.dev/x",
            "javascript:alert(1)",
        ] {
            assert!(Origin::Auth.check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn redirects_stay_on_the_origin_and_only_307_and_308_keep_the_body() {
        let at = Origin::Auth.url("/ui/login/password");
        let (next, keeps) = redirect(&at, StatusCode::FOUND, Some("/ui/login/device")).unwrap();
        assert_eq!((next.path(), keeps), ("/ui/login/device", false));
        let (_, keeps) = redirect(&at, StatusCode::TEMPORARY_REDIRECT, Some("/x")).unwrap();
        assert!(keeps);
        for off in [
            "https://evil.com/steal",
            "http://auth.ironmoon.dev/x",
            "https://mail.ironmoon.dev/x",
        ] {
            assert!(
                redirect(&at, StatusCode::TEMPORARY_REDIRECT, Some(off)).is_err(),
                "{off}"
            );
        }
        assert!(redirect(&at, StatusCode::FOUND, None).is_err());
    }

    #[test]
    fn links_must_stay_on_the_page_origin() {
        let page = Origin::Auth.url("/ui/login/loginname");
        assert_eq!(
            page.join("password").unwrap().as_str(),
            "https://auth.ironmoon.dev/ui/login/password"
        );
        assert_eq!(
            page.join("/ui/login/device/allowed").unwrap().path(),
            "/ui/login/device/allowed"
        );
        for off in [
            "https://evil.com/collect",
            "//evil.com/collect",
            "http://auth.ironmoon.dev/x",
            "https://mail.ironmoon.dev/x",
        ] {
            assert!(page.join(off).is_err(), "{off}");
        }
    }
}
