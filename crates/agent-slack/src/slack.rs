use std::{net::IpAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use private::Secret;
use reqwest::{Client, StatusCode, Url, header};
use rustls_platform_verifier::ConfigVerifierExt;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_tungstenite::WebSocketStream;

use crate::{Channel, Raw, Ts, User};

pub const TIMEOUT: Duration = Duration::from_secs(30);
const RATE_LIMITED_TRIES: usize = 5;

pub struct Slack {
    http: Client,
    api: Url,
    bot: Secret,
    app: Secret,
}

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub type Socket = WebSocketStream<Box<dyn Io>>;

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    messages: Vec<Raw>,
    #[serde(default)]
    has_more: bool,
    response_metadata: Option<Cursor>,
}

#[derive(Deserialize)]
struct Cursor {
    next_cursor: Option<String>,
}

impl Slack {
    pub fn new(api: Url, bot: Secret, app: Secret) -> Result<Slack> {
        ensure!(api.path().ends_with('/'), "the API base {api} must end in /");
        let http = Client::builder().timeout(TIMEOUT).build()?;
        Ok(Slack { http, api, bot, app })
    }

    /// A form POST, the encoding every web API method takes, waiting out a 429 as Slack's Retry-After asks.
    async fn call<T: DeserializeOwned>(
        &self,
        token: &Secret,
        method: &str,
        form: &[(&str, String)],
    ) -> Result<T> {
        for _ in 0..RATE_LIMITED_TRIES {
            let response = self
                .http
                .post(self.api.join(method)?)
                .bearer_auth(token.expose())
                .form(form)
                .send()
                .await
                .with_context(|| format!("calling {method}"))?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                let after = response.headers().get(header::RETRY_AFTER);
                let secs = after.and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(1);
                tokio::time::sleep(Duration::from_secs(secs)).await;
                continue;
            }
            let body: Value = response.error_for_status()?.json().await?;
            if body["ok"] != true {
                bail!("{method}: {}", body["error"].as_str().unwrap_or("not ok"));
            }
            return serde_json::from_value(body).with_context(|| format!("reading {method}"));
        }
        bail!("{method}: still rate limited after {RATE_LIMITED_TRIES} tries")
    }

    pub async fn bot_user(&self) -> Result<User> {
        #[derive(Deserialize)]
        struct Auth {
            user_id: User,
        }
        Ok(self.call::<Auth>(&self.bot, "auth.test", &[]).await?.user_id)
    }

    pub async fn post(&self, channel: &Channel, thread: Option<Ts>, markdown: &str) -> Result<Ts> {
        #[derive(Deserialize)]
        struct Posted {
            ts: Ts,
        }
        let mut form = vec![("channel", channel.to_string()), ("markdown_text", markdown.into())];
        form.extend(thread.map(|t| ("thread_ts", t.to_string())));
        Ok(self.call::<Posted>(&self.bot, "chat.postMessage", &form).await?.ts)
    }

    async fn pages(&self, method: &str, form: Vec<(&str, String)>) -> Result<Vec<Raw>> {
        let mut out = Vec::new();
        let mut cursor = None;
        loop {
            let mut form = form.clone();
            form.push(("limit", "200".into()));
            form.extend(cursor.take().map(|c| ("cursor", c)));
            let page: Page = self.call(&self.bot, method, &form).await?;
            out.extend(page.messages);
            cursor = page.response_metadata.and_then(|m| m.next_cursor).filter(|c| !c.is_empty());
            if !page.has_more || cursor.is_none() {
                return Ok(out);
            }
        }
    }

    /// The channel's top-level messages after `since`, its channel filled in.
    pub async fn history(&self, channel: &Channel, since: Ts) -> Result<Vec<Raw>> {
        let form = vec![("channel", channel.to_string()), ("oldest", since.to_string())];
        Ok(with_channel(self.pages("conversations.history", form).await?, channel))
    }

    /// A thread oldest first, its first message included, or only its replies after `since`.
    pub async fn replies(
        &self,
        channel: &Channel,
        thread: Ts,
        since: Option<Ts>,
    ) -> Result<Vec<Raw>> {
        let mut form = vec![("channel", channel.to_string()), ("ts", thread.to_string())];
        form.extend(since.map(|s| ("oldest", s.to_string())));
        Ok(with_channel(self.pages("conversations.replies", form).await?, channel))
    }

    /// A new Socket Mode connection: wss, or plain ws to this host only, which tests use.
    pub async fn connect(&self) -> Result<Socket> {
        #[derive(Deserialize)]
        struct Opened {
            url: String,
        }
        let url: Url =
            self.call::<Opened>(&self.app, "apps.connections.open", &[]).await?.url.parse()?;
        let host = url.host_str().context("the socket URL has no host")?.to_owned();
        let port = url.port_or_known_default().context("the socket URL has no port")?;
        let tcp = TcpStream::connect((host.as_str(), port)).await?;
        let io: Box<dyn Io> = match url.scheme() {
            "wss" => {
                let tls = Arc::new(rustls::ClientConfig::with_platform_verifier()?);
                let name = rustls::pki_types::ServerName::try_from(host)?;
                Box::new(tokio_rustls::TlsConnector::from(tls).connect(name, tcp).await?)
            }
            "ws" if host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) => Box::new(tcp),
            scheme => bail!("refusing a Socket Mode URL on {scheme}://{host}"),
        };
        Ok(tokio_tungstenite::client_async(url.as_str(), io).await?.0)
    }
}

fn with_channel(mut raws: Vec<Raw>, channel: &Channel) -> Vec<Raw> {
    raws.iter_mut().for_each(|r| r.channel = Some(channel.clone()));
    raws
}
