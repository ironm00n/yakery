//! Wakes an agent whenever the mail server reports a delivery attempt to it: Stalwart posts events here as a webhook
//! (which events is the stalwart bundle's `deliveryWebhooks`), and every recipient in an event's `to` on the agent
//! domain whose name is on the agent list gets a wake.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use agent_host::Vm;
use agent_proto::{Key, Request, read_agents};
use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::Value;
use text::Address;
use tokio::net::TcpListener;
use yakery_log::{fail, note, warn};

const BODY_LIMIT: usize = 1 << 20;
const MESSAGE: &str = "New mail arrived for you.";
/// How long a wake keeps trying while the VM is down: a desktop switch that
/// changes the VM restarts it, and the webhook has been answered by then.
const RETRY_EVERY: Duration = Duration::from_secs(10);
const ATTEMPTS: u32 = 30;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    listen: SocketAddr,
    /// The mail server's addresses, which alone may post: this holds the key,
    /// so it must not wake agents for anyone else who can reach it.
    #[arg(long, required = true)]
    allow_from: Vec<IpAddr>,
    /// JSON object of agent name to uid, the same file the VM's daemon reads.
    #[arg(long)]
    agents: PathBuf,
    #[arg(long)]
    domain: String,
    #[command(flatten)]
    vm: Vm,
}

#[derive(Deserialize)]
struct Batch {
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(default)]
    data: Recipients,
}

/// `to` is an array in delivery events and a string in some others; any other shape names no one.
#[derive(Deserialize, Default)]
struct Recipients {
    #[serde(default)]
    to: Value,
}

struct Dispatch {
    /// Each listed agent's address on the agent domain, and its name.
    agents: Vec<(Address, String)>,
    cid: u32,
    key: Key,
}

impl Dispatch {
    fn new(
        names: impl IntoIterator<Item = String>,
        domain: &str,
        cid: u32,
        key: Key,
    ) -> Result<Self> {
        let address = |name: String| Ok((format!("{name}@{domain}").parse()?, name));
        Ok(Dispatch { agents: names.into_iter().map(address).collect::<Result<_>>()?, cid, key })
    }

    fn recipients(&self, batch: &Batch) -> BTreeSet<String> {
        batch
            .events
            .iter()
            .flat_map(|event| match &event.data.to {
                Value::String(address) => vec![address.as_str()],
                to => to.as_array().into_iter().flatten().filter_map(Value::as_str).collect(),
            })
            .filter_map(|address| address.parse::<Address>().ok())
            .filter_map(|to| self.agents.iter().find(|(agent, _)| *agent == to))
            .map(|(_, name)| name.clone())
            .collect()
    }

    async fn wake(&self, agent: &str) -> Result<String> {
        let request = Request::Wake { agent: agent.to_owned(), message: MESSAGE.to_owned() };
        let mut replies = agent_proto::wake(self.cid, &self.key, &request).await?;
        replies.next_line().await?.context("the daemon hung up without replying")
    }

    async fn wake_until_answered(&self, agent: &str) -> Result<String> {
        let mut attempt = 1;
        loop {
            match self.wake(agent).await {
                Err(e) if attempt < ATTEMPTS => {
                    warn!("{agent}: wake attempt {attempt} failed, retrying: {e:#}");
                    attempt += 1;
                    tokio::time::sleep(RETRY_EVERY).await;
                }
                answered => return answered,
            }
        }
    }
}

async fn handle(
    request: hyper::Request<Incoming>,
    dispatch: Arc<Dispatch>,
) -> Response<Empty<Bytes>> {
    let status = if request.method() != Method::POST {
        StatusCode::METHOD_NOT_ALLOWED
    } else {
        match Limited::new(request.into_body(), BODY_LIMIT).collect().await {
            Ok(body) => match serde_json::from_slice::<Batch>(&body.to_bytes()) {
                Ok(batch) => {
                    for agent in dispatch.recipients(&batch) {
                        let dispatch = dispatch.clone();
                        tokio::spawn(async move {
                            match dispatch.wake_until_answered(&agent).await {
                                Ok(reply) => note!("{agent}: {reply}"),
                                Err(e) => fail!("{agent}: wake failed: {e:#}"),
                            }
                        });
                    }
                    StatusCode::OK
                }
                Err(_) => StatusCode::BAD_REQUEST,
            },
            Err(_) => StatusCode::PAYLOAD_TOO_LARGE,
        }
    };
    let mut response = Response::new(Empty::new());
    *response.status_mut() = status;
    response
}

fn main() -> ExitCode {
    yakery_log::exit_code(serve(Args::parse()).map(|()| ExitCode::SUCCESS))
}

#[tokio::main]
async fn serve(args: Args) -> Result<()> {
    let names = read_agents(&args.agents)?.into_keys();
    let dispatch = Arc::new(Dispatch::new(names, &args.domain, args.vm.cid, args.vm.key()?)?);
    let listener = TcpListener::bind(args.listen).await?;
    loop {
        let (stream, peer) = yakery_serve::accept(async || listener.accept().await).await;
        if !args.allow_from.contains(&peer.ip().to_canonical()) {
            warn!("refused a connection from {peer}");
            continue;
        }
        let dispatch = dispatch.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let dispatch = dispatch.clone();
                async move { Ok::<_, Infallible>(handle(request, dispatch).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

#[cfg(test)]
#[path = "../../tests/agent_dispatch.rs"]
mod tests;
