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

    pub async fn put(&self, file: &str, ics: String) -> Result<()> {
        let status = self.send(Method::PUT, file, Some(ics)).await?;
        anyhow::ensure!(status.is_success(), "stalwart PUT {file}: HTTP {status}");
        Ok(())
    }

    pub async fn delete(&self, file: &str) -> Result<()> {
        let status = self.send(Method::DELETE, file, None).await?;
        anyhow::ensure!(
            status.is_success() || status == StatusCode::NOT_FOUND,
            "stalwart DELETE {file}: HTTP {status}"
        );
        Ok(())
    }

    /// The files the calendar holds, by name.
    pub async fn files(&self) -> Result<BTreeSet<String>> {
        let body = "<propfind xmlns='DAV:'><prop><getetag/></prop></propfind>";
        let reply = self
            .client
            .request(Method::from_bytes(b"PROPFIND")?, &self.url)
            .basic_auth(&self.user, Some(&self.password))
            .header("depth", "1")
            .header("content-type", "application/xml")
            .body(body)
            .send()
            .await
            .context("stalwart PROPFIND")?;
        let status = reply.status();
        anyhow::ensure!(status.as_u16() == 207, "stalwart PROPFIND: HTTP {status}");
        Ok(hrefs(&reply.text().await?)
            .filter_map(|h| h.rsplit_once('/').map(|(_, name)| name.to_string()))
            .filter(|name| name.ends_with(".ics"))
            .collect())
    }
}

/// Every `href` element's text in a multistatus, whatever namespace prefix the server picked.
fn hrefs(xml: &str) -> impl Iterator<Item = &str> {
    xml.split('<').filter_map(|tag| {
        let (name, text) = tag.split_once('>')?;
        let local = name.rsplit_once(':').map_or(name, |(_, l)| l);
        (local == "href" && !name.starts_with('/')).then_some(text)
    })
}

#[cfg(test)]
#[path = "../tests/caldav.rs"]
mod tests;
