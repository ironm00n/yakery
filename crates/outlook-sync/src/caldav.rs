//! The mirror calendar in Stalwart, over CalDAV on loopback as the account it is shared with.

use std::collections::BTreeSet;
use std::fmt::Display;

use anyhow::{Context, Result};
use private::{Dir, Secret};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};

const CALENDAR: &str = "text/calendar; charset=utf-8";

use crate::file::FileName;
use crate::policy::Policy;

const APP_PASSWORD: &str = "app-password";

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
            std::process::exit(75);
        }
        Ok(reply)
    }
}

pub struct Mirror {
    client: Client,
    url: String,
    account: &'static Account,
}

impl Mirror {
    pub fn new(policy: &Policy, account: &'static Account) -> Result<Mirror> {
        let client = Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let url = format!("http://{}{}", policy.stalwart, policy.calendar);
        Ok(Mirror { client, url, account })
    }

    fn request(&self, method: Method, file: impl Display) -> RequestBuilder {
        self.client.request(method, format!("{}{file}", self.url))
    }

    async fn send(&self, request: RequestBuilder, what: &str) -> Result<Response> {
        self.account.send(request, what).await
    }

    /// The new ETag, or None when the file is no longer the version `over` names.
    pub async fn put(
        &self,
        file: &FileName,
        ics: String,
        over: Option<&str>,
    ) -> Result<Option<String>> {
        let mut request =
            self.request(Method::PUT, file).header("content-type", CALENDAR).body(ics);
        if let Some(etag) = over {
            request = request.header("if-match", etag);
        }
        let reply = self.send(request, &format!("PUT {file}")).await?;
        match reply.status() {
            StatusCode::PRECONDITION_FAILED => Ok(None),
            s if s.is_success() => match etag(reply.headers()) {
                Some(tag) => Ok(Some(tag)),
                None => Ok(self.get(file).await?.map(|(_, tag)| tag)),
            },
            s => anyhow::bail!("stalwart PUT {file}: HTTP {s}"),
        }
    }

    pub async fn get(&self, file: &FileName) -> Result<Option<(String, String)>> {
        let reply = self.send(self.request(Method::GET, file), &format!("GET {file}")).await?;
        match reply.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => {
                let tag = etag(reply.headers()).unwrap_or_default();
                Ok(Some((reply.text().await?, tag)))
            }
            s => anyhow::bail!("stalwart GET {file}: HTTP {s}"),
        }
    }

    pub async fn delete(&self, file: &FileName) -> Result<()> {
        let status = self
            .send(self.request(Method::DELETE, file), &format!("DELETE {file}"))
            .await?
            .status();
        anyhow::ensure!(
            status.is_success() || status == StatusCode::NOT_FOUND,
            "stalwart DELETE {file}: HTTP {status}"
        );
        Ok(())
    }

    async fn multistatus(&self, method: &[u8], body: String) -> Result<Option<String>> {
        let name = String::from_utf8_lossy(method).into_owned();
        let request = self
            .request(Method::from_bytes(method)?, "")
            .header("depth", "1")
            .header("content-type", "application/xml")
            .body(body);
        let reply = self.send(request, &name).await?;
        match reply.status().as_u16() {
            207 => Ok(Some(reply.text().await?)),
            // a sync token the server no longer knows
            403 | 409 => Ok(None),
            s => anyhow::bail!("stalwart {name}: HTTP {s}"),
        }
    }

    pub async fn files(&self) -> Result<BTreeSet<FileName>> {
        let body = "<propfind xmlns='DAV:'><prop><getetag/></prop></propfind>".to_string();
        let xml =
            self.multistatus(b"PROPFIND", body).await?.context("stalwart refused PROPFIND")?;
        Ok(responses(&xml).0.into_iter().filter(|e| !e.gone).map(|e| e.file).collect())
    }

    /// Everything counts as changed when `token` is None or the server no longer knows it.
    pub async fn changes(&self, token: Option<&str>) -> Result<(Vec<Entry>, String)> {
        let request = |token: Option<&str>| {
            format!(
                "<sync-collection xmlns='DAV:'><sync-token>{}</sync-token><sync-level>1</sync-level><prop><getetag/></prop></sync-collection>",
                token.unwrap_or_default()
            )
        };
        let xml = match self.multistatus(b"REPORT", request(token)).await? {
            Some(xml) => xml,
            None => self
                .multistatus(b"REPORT", request(None))
                .await?
                .context("stalwart refused sync-collection")?,
        };
        let (entries, next) = responses(&xml);
        Ok((entries, next.context("a sync-collection answer with no sync-token")?))
    }
}

fn etag(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers.get("etag").and_then(|v| v.to_str().ok()).map(str::to_string)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub file: FileName,
    pub etag: Option<String>,
    pub gone: bool,
}

/// Tags match by local name, whatever namespace prefix the server picked.
fn responses(xml: &str) -> (Vec<Entry>, Option<String>) {
    let (mut entries, mut token) = (vec![], None);
    let mut current: Option<(Option<FileName>, Option<String>, bool)> = None;
    for tag in xml.split('<') {
        let Some((name, text)) = tag.split_once('>') else { continue };
        let closing = name.starts_with('/');
        let local = name
            .trim_start_matches('/')
            .rsplit_once(':')
            .map_or(name.trim_start_matches('/'), |(_, l)| l);
        let local = local.split_whitespace().next().unwrap_or_default();
        match (local, closing) {
            ("response", false) => current = Some((None, None, false)),
            ("response", true) => {
                if let Some((Some(file), etag, gone)) = current.take() {
                    entries.push(Entry { file, etag, gone });
                }
            }
            ("href", false) => {
                if let Some((file, _, _)) = current.as_mut() {
                    *file = text.rsplit_once('/').map_or(text, |(_, f)| f).parse().ok();
                }
            }
            ("getetag", false) => {
                if let Some((_, etag, _)) = current.as_mut() {
                    *etag = Some(text.replace("&quot;", "\""));
                }
            }
            ("status", false) if text.contains(" 404") => {
                if let Some((_, _, gone)) = current.as_mut() {
                    *gone = true;
                }
            }
            ("sync-token", false) if current.is_none() => token = Some(text.to_string()),
            _ => {}
        }
    }
    (entries, token)
}

#[cfg(test)]
#[path = "../tests/caldav.rs"]
mod tests;
