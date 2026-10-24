//! Wakes an agent whenever the mail server reports a delivery to it. Stalwart
//! posts its `delivery.completed` events here as a webhook; every recipient on
//! the agent domain whose name is on the agent list gets a wake.

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_proto::{CONTROL_PORT, Key, Request, Service, WAKE, accept, prove, read_key};
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use clap::Parser;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_vsock::{VsockAddr, VsockStream};

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
    /// The agent VM's vsock CID.
    #[arg(long, default_value_t = 4)]
    cid: u32,
    /// The key shared with the VM's daemon.
    #[arg(long)]
    key: PathBuf,
}

#[derive(Deserialize)]
struct Batch {
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    r#type: String,
    #[serde(default)]
    data: HashMap<String, Value>,
}

struct Dispatch {
    agents: BTreeSet<String>,
    domain: String,
    cid: u32,
    key: Key,
}

impl Dispatch {
    fn recipients<'a>(&self, batch: &'a Batch) -> BTreeSet<&'a str> {
        batch
            .events
            .iter()
            .filter(|event| event.r#type == "delivery.completed")
            .filter_map(|event| event.data.get("to"))
            .flat_map(|to| match to {
                Value::Array(addresses) => addresses.iter().filter_map(Value::as_str).collect(),
                Value::String(address) => vec![address.as_str()],
                _ => vec![],
            })
            .filter_map(|address| {
                let (name, domain) = address.split_once('@')?;
                (domain.eq_ignore_ascii_case(&self.domain) && self.agents.contains(name))
                    .then_some(name)
            })
            .collect()
    }

    async fn wake(&self, agent: &str) -> Result<String> {
        let mut stream = VsockStream::connect(VsockAddr::new(self.cid, CONTROL_PORT)).await?;
        prove(&mut stream, &self.key, Service::Control, WAKE).await?;
        let (read, mut write) = tokio::io::split(stream);
        let request = Request::Wake { agent: agent.to_owned(), message: MESSAGE.to_owned() };
        write.write_all(format!("{}\n", serde_json::to_string(&request)?).as_bytes()).await?;
        let mut reply = String::new();
        if BufReader::new(read).read_line(&mut reply).await? == 0 {
            bail!("the daemon hung up without replying");
        }
        Ok(reply.trim_end().to_owned())
    }

    async fn wake_until_answered(&self, agent: &str) -> Result<String> {
        let mut attempt = 1;
        loop {
            match self.wake(agent).await {
                Err(e) if attempt < ATTEMPTS => {
                    eprintln!("{agent}: wake attempt {attempt} failed, retrying: {e:#}");
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
                        let agent = agent.to_owned();
                        tokio::spawn(async move {
                            match dispatch.wake_until_answered(&agent).await {
                                Ok(reply) => eprintln!("{agent}: {reply}"),
                                Err(e) => eprintln!("{agent}: wake failed: {e:#}"),
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let agents: HashMap<String, u32> = serde_json::from_slice(&std::fs::read(&args.agents)?)
        .with_context(|| format!("read {}", args.agents.display()))?;
    let dispatch = Arc::new(Dispatch {
        agents: agents.into_keys().collect(),
        domain: args.domain,
        cid: args.cid,
        key: read_key(&args.key)?,
    });
    let listener = TcpListener::bind(args.listen).await?;
    loop {
        let (stream, peer) = accept(async || listener.accept().await).await;
        if !args.allow_from.contains(&peer.ip().to_canonical()) {
            eprintln!("refused a connection from {peer}");
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
mod tests {
    use super::*;

    #[test]
    fn wakes_listed_agents_on_the_agent_domain_once() {
        let dispatch = Dispatch {
            agents: ["alpha", "beta"].map(String::from).into(),
            domain: "agent.ironmoon.dev".into(),
            cid: 4,
            key: [0; 32],
        };
        let batch: Batch = serde_json::from_value(serde_json::json!({ "events": [
            { "type": "delivery.completed", "data": { "to": [
                "alpha@agent.ironmoon.dev", "beta@agent.ironmoon.dev",
                "gamma@agent.ironmoon.dev", "alpha@ironmoon.dev", "ironmoon@ironmoon.dev",
            ] } },
            { "type": "delivery.completed", "data": { "to": "alpha@AGENT.ironmoon.dev" } },
            { "type": "message-ingest.ham", "data": { "to": ["beta@agent.ironmoon.dev"] } },
        ] }))
        .unwrap();
        assert_eq!(dispatch.recipients(&batch), BTreeSet::from(["alpha", "beta"]));
    }
}
