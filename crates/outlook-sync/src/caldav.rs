//! The mirror calendar in Stalwart, over CalDAV on loopback as the account it is shared with.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use reqwest::{Client, Method, StatusCode};

use crate::policy::Policy;

pub struct Mirror {
    client: Client,
    url: String,
    user: String,
    password: String,
}

impl Mirror {
    pub fn new(policy: &Policy, password: &Path) -> Result<Mirror> {
        let password =
            std::fs::read_to_string(password).context("the app password")?.trim().to_string();
        let client = Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let url = format!("http://{}{}", policy.stalwart, policy.calendar);
        Ok(Mirror { client, url, user: policy.account.clone(), password })
    }

    async fn send(&self, method: Method, file: &str, body: Option<String>) -> Result<StatusCode> {
        let mut request = self
            .client
            .request(method.clone(), format!("{}{file}", self.url))
            .basic_auth(&self.user, Some(&self.password));
        if let Some(body) = body {
            request = request.header("content-type", "text/calendar; charset=utf-8").body(body);
        }
        let status =
            request.send().await.with_context(|| format!("stalwart {method} {file}"))?.status();
        if status == StatusCode::UNAUTHORIZED {
            yakery_log::warn!("{} was refused; exiting to read the app password again", self.user);
            std::process::exit(75);
        }
        Ok(status)
    }

    /// Writes `file`, only over the version `over` names when one is given; the new ETag, or
    /// None when someone changed the file since.
    pub async fn put(&self, file: &str, ics: String, over: Option<&str>) -> Result<Option<String>> {
        let mut request = self
            .client
            .put(format!("{}{file}", self.url))
            .basic_auth(&self.user, Some(&self.password))
            .header("content-type", "text/calendar; charset=utf-8")
            .body(ics);
        if let Some(etag) = over {
            request = request.header("if-match", etag);
        }
        let reply = request.send().await.with_context(|| format!("stalwart PUT {file}"))?;
        match reply.status() {
            StatusCode::PRECONDITION_FAILED => Ok(None),
            s if s.is_success() => match etag(reply.headers()) {
                Some(tag) => Ok(Some(tag)),
                None => Ok(self.get(file).await?.map(|(_, tag)| tag)),
            },
            s => anyhow::bail!("stalwart PUT {file}: HTTP {s}"),
        }
    }

    pub async fn get(&self, file: &str) -> Result<Option<(String, String)>> {
        let reply = self
            .client
            .get(format!("{}{file}", self.url))
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .await
            .with_context(|| format!("stalwart GET {file}"))?;
        match reply.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => {
                let tag = etag(reply.headers()).unwrap_or_default();
                Ok(Some((reply.text().await?, tag)))
            }
            s => anyhow::bail!("stalwart GET {file}: HTTP {s}"),
        }
    }

    pub async fn delete(&self, file: &str) -> Result<()> {
        let status = self.send(Method::DELETE, file, None).await?;
        anyhow::ensure!(
            status.is_success() || status == StatusCode::NOT_FOUND,
            "stalwart DELETE {file}: HTTP {status}"
        );
        Ok(())
    }

    async fn multistatus(&self, method: &[u8], body: String) -> Result<Option<String>> {
        let reply = self
            .client
            .request(Method::from_bytes(method)?, &self.url)
            .basic_auth(&self.user, Some(&self.password))
            .header("depth", "1")
            .header("content-type", "application/xml")
            .body(body)
            .send()
            .await
            .context("stalwart multistatus")?;
        match reply.status().as_u16() {
            207 => Ok(Some(reply.text().await?)),
            // a sync token the server no longer knows
            403 | 409 => Ok(None),
            s => anyhow::bail!("stalwart {}: HTTP {s}", String::from_utf8_lossy(method)),
        }
    }

    /// The files the calendar holds, by name.
    pub async fn files(&self) -> Result<BTreeSet<String>> {
        let body = "<propfind xmlns='DAV:'><prop><getetag/></prop></propfind>".to_string();
        let xml =
            self.multistatus(b"PROPFIND", body).await?.context("stalwart refused PROPFIND")?;
        Ok(responses(&xml).0.into_iter().filter(|e| !e.gone).map(|e| e.file).collect())
    }

    /// What changed in the calendar since `token`, and the token to ask from next; everything,
    /// as changed, when `token` is None or the server no longer knows it.
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

/// One response of a multistatus, for an event file.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub file: String,
    pub etag: Option<String>,
    pub gone: bool,
}

/// A multistatus's responses for `.ics` files, and its sync-token, whatever namespace prefix the
/// server picked.
fn responses(xml: &str) -> (Vec<Entry>, Option<String>) {
    let (mut entries, mut token, mut current) = (vec![], None, None::<Entry>);
    for tag in xml.split('<') {
        let Some((name, text)) = tag.split_once('>') else { continue };
        let closing = name.starts_with('/');
        let local = name
            .trim_start_matches('/')
            .rsplit_once(':')
            .map_or(name.trim_start_matches('/'), |(_, l)| l);
        let local = local.split_whitespace().next().unwrap_or_default();
        match (local, closing) {
            ("response", false) => {
                current = Some(Entry { file: String::new(), etag: None, gone: false })
            }
            ("response", true) => {
                entries.extend(current.take().filter(|e| e.file.ends_with(".ics")))
            }
            ("href", false) => {
                if let Some(e) = current.as_mut() {
                    e.file = text.rsplit_once('/').map_or(text, |(_, f)| f).to_string();
                }
            }
            ("getetag", false) => {
                if let Some(e) = current.as_mut() {
                    e.etag = Some(text.replace("&quot;", "\""));
                }
            }
            ("status", false) if text.contains(" 404") => {
                if let Some(e) = current.as_mut() {
                    e.gone = true;
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
