//! Mail submission (RFC 6409): STARTTLS, then AUTH PLAIN, then one message to one recipient.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use private::Secret;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::ConfigVerifierExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

pub struct Submission {
    pub host: String,
    pub port: u16,
    pub helo: String,
    pub login: String,
    pub password: Secret,
}

struct Session<S>(BufReader<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    /// The server's next reply, which must have code `want`; a multiline reply is one.
    async fn expect(&mut self, want: u16) -> Result<()> {
        let mut text = String::new();
        loop {
            let start = text.len();
            if self.0.read_line(&mut text).await? == 0 {
                bail!("the server hung up, after {text:?}");
            }
            let line = &text[start..];
            if line.as_bytes().get(3) != Some(&b'-') {
                let code: u16 =
                    line.get(..3).and_then(|c| c.parse().ok()).context("not a reply")?;
                if code != want {
                    bail!("{}", text.trim_end());
                }
                return Ok(());
            }
        }
    }

    async fn command(&mut self, line: &str, want: u16, shown: &str) -> Result<()> {
        self.0.get_mut().write_all(format!("{line}\r\n").as_bytes()).await?;
        self.expect(want).await.with_context(|| format!("after {shown}"))
    }

    async fn hello(&mut self, helo: &str) -> Result<()> {
        self.command(&format!("EHLO {helo}"), 250, "EHLO").await
    }
}

impl Submission {
    /// Sends `message`, its lines ending in CRLF, from `from` to `to`.
    pub async fn send(&self, from: &str, to: &str, message: &str) -> Result<()> {
        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .with_context(|| format!("connecting to {}:{}", self.host, self.port))?;
        let mut plain = Session(BufReader::new(tcp));
        plain.expect(220).await.context("the greeting")?;
        plain.hello(&self.helo).await?;
        plain.command("STARTTLS", 220, "STARTTLS").await?;
        let tls = Arc::new(rustls::ClientConfig::with_platform_verifier()?);
        let name = ServerName::try_from(self.host.clone())?;
        let mut tls = Session(BufReader::new(
            TlsConnector::from(tls).connect(name, plain.0.into_inner()).await?,
        ));
        tls.hello(&self.helo).await?;
        let plain = BASE64.encode(format!("\0{}\0{}", self.login, self.password.expose()));
        tls.command(&format!("AUTH PLAIN {plain}"), 235, "AUTH").await?;
        tls.command(&format!("MAIL FROM:<{from}>"), 250, "MAIL").await?;
        tls.command(&format!("RCPT TO:<{to}>"), 250, "RCPT").await?;
        tls.command("DATA", 354, "DATA").await?;
        let stuffed = message.replace("\r\n.", "\r\n..");
        tls.command(&format!("{stuffed}\r\n."), 250, "the message").await?;
        let _ = tls.command("QUIT", 221, "QUIT").await;
        Ok(())
    }
}
