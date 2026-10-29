//! Stalwart's DATA-stage hook for mail an account submits: a From or Sender that isn't the envelope sender's own
//! address, subaddresses aside, is refused, and a message with a recipient at one of `--domain` gets a
//! `Submission-Signature` naming the account and the sender.

use std::{
    convert::Infallible,
    fs,
    io::{Read, stdin},
    net::TcpListener as StdListener,
    path::Path,
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use clap::Parser;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Response, StatusCode, body::Incoming, header::AUTHORIZATION, service::service_fn,
};
use hyper_util::rt::TokioIo;
use mail_parser::{HeaderValue, parsers::MessageStream};
use private::{Secret, listen_fds, now};
use serde::Deserialize;
use serde_json::{Value, json};
use submission_signature::{HEADER, Signer, Stamp, verify};
use text::Address;
use tokio::net::TcpListener;
use yakery_log::{note, warn};

/// A request carries the whole message as JSON text.
const BODY_LIMIT: usize = 256 << 20;
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

#[derive(Parser)]
enum Command {
    /// Answer Stalwart's hook requests on the one socket systemd passes.
    Serve {
        /// The ed25519 seed the header is signed with, 32 bytes in base64.
        #[arg(long)]
        seed: PathBuf,
        /// The bearer token every request must carry.
        #[arg(long)]
        token: PathBuf,
        /// A domain whose recipients get the header.
        #[arg(long = "domain", required = true)]
        domains: Vec<String>,
    },
    /// Print a new seed on stdout as a JSON string, the value `sops set --value-stdin` takes, and its public key on
    /// stderr.
    Seed,
    /// Print the public key of a seed.
    Public {
        #[arg(long)]
        seed: PathBuf,
    },
    /// Print the stamp of the message on stdin, when its header verifies with `public`.
    Check {
        #[arg(long)]
        public: String,
    },
}

#[derive(Deserialize)]
struct Hook {
    context: Session,
    envelope: Option<Envelope>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Session {
    sasl: Option<Login>,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct Envelope {
    from: Mailbox,
    to: Vec<Mailbox>,
}

#[derive(Deserialize)]
struct Mailbox {
    address: String,
}

#[derive(Deserialize)]
struct Message {
    headers: Vec<(String, String)>,
    contents: String,
    size: usize,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Pass,
    Stamp(String),
    Refuse(String),
}

impl Verdict {
    /// As Stalwart's hook response.
    fn answer(&self) -> Value {
        match self {
            Verdict::Pass => json!({ "action": "accept" }),
            Verdict::Stamp(value) => json!({
                "action": "accept",
                "modifications": [{ "type": "addHeader", "name": HEADER, "value": value }],
            }),
            Verdict::Refuse(why) => json!({
                "action": "reject",
                "response": { "status": 550, "enhanced_status": "5.7.1", "message": why },
            }),
        }
    }
}

struct Judge {
    signer: Signer,
    domains: Vec<String>,
}

/// The local part Stalwart writes an account's calendar alarms from, at the account's domain, when none is configured.
const ALARMS_FROM: &str = "calendar-notification";

/// Whether the message says no one typed it (RFC 3834), as the calendar replies and alarms Stalwart writes for an
/// account do.
fn automatic(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.trim_end().eq_ignore_ascii_case("Auto-Submitted")
            && !value.trim().eq_ignore_ascii_case("no")
    })
}

/// Why the message's From or Sender isn't `sender`'s own address, when one isn't. Stalwart parses them with the same
/// parser, so what Bulwark shows is what this checks. An `alarm` may also come From [`ALARMS_FROM`].
fn foreign_author(headers: &[(String, String)], sender: &Address, alarm: bool) -> Option<String> {
    if headers
        .iter()
        .flat_map(|(name, value)| [name, value])
        .any(|t| t.contains(char::REPLACEMENT_CHARACTER))
    {
        return Some("a header is not UTF-8".into());
    }
    ["From", "Sender"].into_iter().find_map(|name| {
        let fields: Vec<_> =
            headers.iter().filter(|(n, _)| n.trim_end().eq_ignore_ascii_case(name)).collect();
        let [(_, value)] = fields[..] else {
            return (name == "From" || !fields.is_empty())
                .then(|| format!("{} {name} fields, not one", fields.len()));
        };
        let parsed = MessageStream::new(value.as_bytes()).parse_address();
        let named: Vec<_> = match &parsed {
            HeaderValue::Address(a) => a.iter().collect(),
            _ => Vec::new(),
        };
        let [one] = named[..] else {
            return Some(format!("{name} names {} addresses, not one", named.len()));
        };
        match one.address.as_deref().map(str::parse::<Address>) {
            Some(Ok(a))
                if a.domain().eq_ignore_ascii_case(sender.domain())
                    && (a.user().eq_ignore_ascii_case(sender.user())
                        || alarm && name == "From" && a.user() == ALARMS_FROM) =>
            {
                None
            }
            _ => Some(format!("{name} is not {sender}, the address this account sends from")),
        }
    })
}

/// The message as the client sent it, as far as the request's lossy text keeps it.
fn rebuilt(m: &Message) -> Vec<u8> {
    let headers: String =
        m.headers.iter().flat_map(|(name, value)| [name.as_str(), ":", value.as_str()]).collect();
    [headers.as_str(), "\r\n", &m.contents].concat().into_bytes()
}

impl Judge {
    fn judge(&self, hook: Hook, t: i64) -> Result<Verdict> {
        let Some(Login { login }) = hook.context.sasl else { return Ok(Verdict::Pass) };
        let (Some(envelope), Some(message)) = (hook.envelope, hook.message) else {
            bail!(
                "{login}: a request with no envelope or message, which the DATA stage always has"
            );
        };
        let Ok(sender) = envelope.from.address.parse::<Address>() else {
            return Ok(Verdict::Refuse(format!(
                "{:?} is no address to send from",
                envelope.from.address
            )));
        };
        let local = |m: &Mailbox| {
            m.address
                .rsplit_once('@')
                .is_some_and(|(_, d)| self.domains.iter().any(|l| l.eq_ignore_ascii_case(d)))
        };
        let automatic = automatic(&message.headers);
        let alarm = automatic && envelope.to.iter().all(local);
        if let Some(why) = foreign_author(&message.headers, &sender, alarm) {
            return Ok(Verdict::Refuse(why));
        }
        if message.headers.iter().any(|(name, _)| name.trim_end().eq_ignore_ascii_case(HEADER)) {
            return Ok(Verdict::Refuse(format!(
                "a submitted message carries no {HEADER} of its own"
            )));
        }
        if automatic || !envelope.to.iter().any(local) {
            return Ok(Verdict::Pass);
        }
        let account = match login.parse() {
            Ok(account) => account,
            Err(e) => {
                warn!("{e:#}, so its mail goes out unstamped");
                return Ok(Verdict::Pass);
            }
        };
        let raw = rebuilt(&message);
        let lost = if raw.len() != message.size {
            Some(format!("came through as {} bytes, not {}", raw.len(), message.size))
        } else {
            message
                .contents
                .contains(char::REPLACEMENT_CHARACTER)
                .then(|| "came through with bytes that are not UTF-8".into())
        };
        if let Some(how) = lost {
            warn!("{login}: a message {how}, so it goes out unstamped");
            return Ok(Verdict::Pass);
        }
        Ok(Verdict::Stamp(self.signer.sign(&Stamp { account, sender, t }, &raw)))
    }
}

async fn handle(
    request: hyper::Request<Incoming>,
    judge: Arc<Judge>,
    bearer: Arc<Secret>,
) -> Response<Full<Bytes>> {
    let presented = request.headers().get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let (status, answer) = if request.method() != Method::POST {
        (StatusCode::METHOD_NOT_ALLOWED, None)
    } else if !presented.is_some_and(|v| Secret::new(v) == *bearer) {
        warn!("refused a request without the token");
        (StatusCode::UNAUTHORIZED, None)
    } else {
        match Limited::new(request.into_body(), BODY_LIMIT).collect().await {
            Err(_) => (StatusCode::PAYLOAD_TOO_LARGE, None),
            Ok(body) => match serde_json::from_slice(&body.to_bytes())
                .map_err(anyhow::Error::from)
                .and_then(|hook| judge.judge(hook, now()))
            {
                Ok(verdict) => {
                    match &verdict {
                        Verdict::Refuse(why) => note!("refused: {why}"),
                        Verdict::Stamp(value) => {
                            note!("stamped: {}", value.split("; sig=").next().unwrap_or(""))
                        }
                        Verdict::Pass => {}
                    }
                    (StatusCode::OK, Some(verdict.answer()))
                }
                Err(e) => {
                    warn!("{e:#}");
                    (StatusCode::BAD_REQUEST, None)
                }
            },
        }
    };
    let mut response =
        Response::new(Full::new(answer.map_or_else(Bytes::new, |a| a.to_string().into())));
    *response.status_mut() = status;
    response
}

#[tokio::main(flavor = "current_thread")]
async fn serve(listener: StdListener, judge: Judge, bearer: Secret) -> Result<ExitCode> {
    let listener = TcpListener::from_std(listener)?;
    let (judge, bearer) = (Arc::new(judge), Arc::new(bearer));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                warn!("accept: {e}");
                tokio::time::sleep(ACCEPT_PAUSE).await;
                continue;
            }
        };
        let (judge, bearer) = (judge.clone(), bearer.clone());
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let (judge, bearer) = (judge.clone(), bearer.clone());
                async move { Ok::<_, Infallible>(handle(request, judge, bearer).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn run(command: Command) -> Result<ExitCode> {
    match command {
        Command::Seed => {
            let seed = Signer::new_seed()?;
            eprintln!("{}", Signer::from_seed(&seed)?.public());
            println!("{}", json!(seed));
        }
        Command::Public { seed } => println!("{}", Signer::from_seed(&read(&seed)?)?.public()),
        Command::Check { public } => {
            let mut raw = Vec::new();
            stdin().read_to_end(&mut raw)?;
            println!("{}", verify(&public, &raw)?.stamp);
        }
        Command::Serve { seed, token, domains } => {
            let [(_, socket)]: [_; 1] = listen_fds()?.try_into().map_err(|passed: Vec<_>| {
                anyhow!("systemd passed {} sockets, not one", passed.len())
            })?;
            let listener = StdListener::from(socket);
            listener.set_nonblocking(true)?;
            let bearer = Secret::new(format!("Bearer {}", read(&token)?.trim()));
            return serve(
                listener,
                Judge { signer: Signer::from_seed(&read(&seed)?)?, domains },
                bearer,
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    yakery_log::exit_code(run(Command::parse()))
}

#[cfg(test)]
mod tests;
