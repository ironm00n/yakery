//! Forgejo's webhook for one repository's issue comments: a person's comment is mailed to the lane its issue's owner
//! label names, or to `--fallback` when none does, so the session watching that lane's mail topic wakes with it.

mod smtp;

use std::{
    collections::HashMap, convert::Infallible, fs, net::TcpListener as StdListener, path::Path,
    path::PathBuf, process::ExitCode, sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use clap::Parser;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::{Method, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use private::{Secret, hex, listen_fds};
use ring::hmac;
use serde::Deserialize;
use smtp::Submission;
use tokio::net::TcpListener;
use yakery_log::{note, warn};

const BODY_LIMIT: usize = 1 << 20;

/// Answers Forgejo's webhook requests on the one socket systemd passes.
#[derive(Parser)]
struct Args {
    /// The webhook's secret, which signs each request.
    #[arg(long)]
    secret: PathBuf,
    /// The submission server, reached with STARTTLS.
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = 587)]
    port: u16,
    /// The name this host greets the server with.
    #[arg(long)]
    helo: String,
    /// The account mail is sent as.
    #[arg(long)]
    login: String,
    /// Its password.
    #[arg(long)]
    password: PathBuf,
    /// The address mail comes from, one the account may send from.
    #[arg(long)]
    from: String,
    /// Where a lane's mail goes, `{lane}` standing for the lane.
    #[arg(long)]
    to: String,
    /// What an issue's owner label starts with; the rest names its lane.
    #[arg(long, default_value = "owner/")]
    label: String,
    /// A lane mailed in another's place, as `lane=instead`.
    #[arg(long = "route", value_parser = route)]
    routes: Vec<(String, String)>,
    /// The lane of an issue no owner label names.
    #[arg(long)]
    fallback: String,
    /// A Forgejo user whose comments wake no one.
    #[arg(long = "quiet")]
    quiet: Vec<String>,
}

fn route(arg: &str) -> Result<(String, String)> {
    let (lane, instead) = arg.split_once('=').context("not lane=instead")?;
    Ok((lane.into(), instead.into()))
}

#[derive(Deserialize)]
struct Hook {
    action: String,
    issue: Issue,
    comment: Comment,
    repository: Repository,
    sender: User,
    #[serde(default)]
    is_pull: bool,
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    labels: Vec<Label>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct Comment {
    id: u64,
    body: String,
    html_url: String,
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

/// A mail to send: its recipient and its text, lines ending in CRLF.
#[derive(Debug, PartialEq)]
struct Mail {
    to: String,
    text: String,
}

struct Router {
    to: String,
    from: String,
    label: String,
    routes: HashMap<String, String>,
    fallback: String,
    quiet: Vec<String>,
}

/// A lane names a mail topic: letters, digits and dashes.
fn is_lane(lane: &str) -> bool {
    !lane.is_empty() && lane.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// RFC 2047's encoded words, for a header that isn't ASCII.
fn encoded(text: &str) -> String {
    if text.is_ascii() {
        return text.into();
    }
    let mut words = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(45);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        words.push(format!("=?utf-8?b?{}?=", BASE64.encode(&rest[..end])));
        rest = &rest[end..];
    }
    words.join("\r\n ")
}

impl Router {
    fn lane<'a>(&'a self, labels: &'a [Label]) -> &'a str {
        let lane = match labels.iter().find_map(|l| l.name.strip_prefix(&self.label)) {
            Some(lane) if is_lane(lane) => lane,
            Some(lane) => {
                warn!("an owner label names {lane:?}, which is no lane; mailing {}", self.fallback);
                &self.fallback
            }
            None => &self.fallback,
        };
        self.routes.get(lane).map_or(lane, String::as_str)
    }

    /// The mail `hook` sends, dated `date`, or why it sends none.
    fn mail(&self, hook: &Hook, date: &str) -> Result<Mail, String> {
        if hook.action != "created" {
            return Err(format!("a comment {}", hook.action));
        }
        if self.quiet.contains(&hook.sender.login) {
            return Err(format!("a comment by {}", hook.sender.login));
        }
        let to = self.to.replace("{lane}", self.lane(&hook.issue.labels));
        let (repo, n) = (&hook.repository.full_name, hook.issue.number);
        let host = hook.comment.html_url.split('/').nth(2).unwrap_or_default();
        let kind = if hook.is_pull { "pulls" } else { "issues" };
        let thread = format!("<{repo}/{kind}/{n}@{host}>");
        let body = format!(
            "{} commented on {repo}#{n}, {}:\n\n{}\n\n{}\n",
            hook.sender.login, hook.issue.title, hook.comment.body, hook.comment.html_url
        );
        let encoded_body = BASE64.encode(body);
        let lines =
            encoded_body.as_bytes().chunks(76).map(|c| std::str::from_utf8(c).unwrap_or(""));
        let headers = [
            format!("From: {}", self.from),
            format!("To: {to}"),
            format!("Subject: {}", encoded(&format!("Re: [{repo}] {} (#{n})", hook.issue.title))),
            format!("Date: {date}"),
            format!("Message-ID: <{repo}/{kind}/{n}/comment/{}/wake@{host}>", hook.comment.id),
            format!("In-Reply-To: {thread}"),
            format!("References: {thread}"),
            "MIME-Version: 1.0".into(),
            "Content-Type: text/plain; charset=utf-8".into(),
            "Content-Transfer-Encoding: base64".into(),
        ];
        let text = headers.into_iter().chain([String::new()]).chain(lines.map(String::from));
        Ok(Mail { to, text: text.collect::<Vec<_>>().join("\r\n") })
    }
}

struct Wake {
    key: hmac::Key,
    router: Router,
    submission: Submission,
}

/// Whether `signature`, hex, is the HMAC-SHA256 of `body` under `key`.
fn signed(key: &hmac::Key, body: &[u8], signature: Option<&str>) -> bool {
    signature
        .and_then(|s| hex::decode(s).ok())
        .is_some_and(|tag| hmac::verify(key, body, &tag).is_ok())
}

async fn handle(request: hyper::Request<Incoming>, wake: Arc<Wake>) -> StatusCode {
    let header = |name| request.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from);
    let (signature, event) = (header("x-forgejo-signature"), header("x-forgejo-event"));
    if request.method() != Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED;
    }
    let Ok(body) = Limited::new(request.into_body(), BODY_LIMIT).collect().await else {
        return StatusCode::PAYLOAD_TOO_LARGE;
    };
    let body = body.to_bytes();
    if !signed(&wake.key, &body, signature.as_deref()) {
        warn!("refused a request its signature doesn't match");
        return StatusCode::UNAUTHORIZED;
    }
    if event.as_deref() != Some("issue_comment") {
        return StatusCode::NO_CONTENT;
    }
    let hook: Hook = match serde_json::from_slice(&body) {
        Ok(hook) => hook,
        Err(e) => {
            warn!("an issue_comment request that doesn't parse: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };
    let date = jiff::fmt::rfc2822::to_string(&jiff::Zoned::now()).unwrap_or_default();
    match wake.router.mail(&hook, &date) {
        Err(why) => {
            note!("{}#{}: {why} wakes no one", hook.repository.full_name, hook.issue.number);
            StatusCode::NO_CONTENT
        }
        Ok(mail) => match wake.submission.send(&wake.router.from, &mail.to, &mail.text).await {
            Ok(()) => {
                note!("{}#{}: mailed {}", hook.repository.full_name, hook.issue.number, mail.to);
                StatusCode::ACCEPTED
            }
            Err(e) => {
                warn!(
                    "{}#{}: mailing {}: {e:#}",
                    hook.repository.full_name, hook.issue.number, mail.to
                );
                StatusCode::BAD_GATEWAY
            }
        },
    }
}

#[tokio::main(flavor = "current_thread")]
async fn serve(listener: StdListener, wake: Wake) -> Result<ExitCode> {
    let listener = TcpListener::from_std(listener)?;
    let wake = Arc::new(wake);
    loop {
        let (stream, _) = yakery_serve::accept("the webhook socket", || listener.accept()).await?;
        let wake = wake.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let wake = wake.clone();
                async move {
                    let mut response = Response::new(Empty::<Bytes>::new());
                    *response.status_mut() = handle(request, wake).await;
                    Ok::<_, Infallible>(response)
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

fn read(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .trim()
        .into())
}

fn run(args: Args) -> Result<ExitCode> {
    let [(_, socket)]: [_; 1] = listen_fds()?
        .try_into()
        .map_err(|passed: Vec<_>| anyhow!("systemd passed {} sockets, not one", passed.len()))?;
    let listener = StdListener::from(socket);
    listener.set_nonblocking(true)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let wake = Wake {
        key: hmac::Key::new(hmac::HMAC_SHA256, read(&args.secret)?.as_bytes()),
        submission: Submission {
            host: args.host,
            port: args.port,
            helo: args.helo,
            login: args.login,
            password: Secret::new(read(&args.password)?),
        },
        router: Router {
            to: args.to,
            from: args.from,
            label: args.label,
            routes: args.routes.into_iter().collect(),
            fallback: args.fallback,
            quiet: args.quiet,
        },
    };
    serve(listener, wake)
}

fn main() -> ExitCode {
    yakery_log::exit_code(run(Args::parse()))
}

#[cfg(test)]
mod tests;
