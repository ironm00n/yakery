//! Wakes an agent whenever the mail server reports a delivery to it. Stalwart
//! posts its `delivery.completed` events here as a webhook; every recipient on
//! the agent domain whose name is on the agent list gets a wake.

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use agentd::{CONTROL_PORT, Request};
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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_vsock::{VsockAddr, VsockStream};

const BODY_LIMIT: usize = 1 << 20;
const MESSAGE: &str = "New mail arrived for you.";

#[derive(Parser)]
struct Args {
    #[arg(long)]
    listen: SocketAddr,
    /// JSON object of agent name to uid, the same file the VM's daemon reads.
    #[arg(long)]
    agents: PathBuf,
    #[arg(long)]
    domain: String,
    /// The agent VM's vsock CID.
    #[arg(long, default_value_t = 4)]
    cid: u32,
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
        let stream = VsockStream::connect(VsockAddr::new(self.cid, CONTROL_PORT)).await?;
        let (read, mut write) = tokio::io::split(stream);
        let request = Request::Wake {
            agent: agent.to_owned(),
            message: MESSAGE.to_owned(),
        };
        write
            .write_all(format!("{}\n", serde_json::to_string(&request)?).as_bytes())
            .await?;
        let mut reply = String::new();
        BufReader::new(read).read_line(&mut reply).await?;
        Ok(reply.trim_end().to_owned())
    }
}

async fn handle(
    request: hyper::Request<Incoming>,
    dispatch: Arc<Dispatch>,
) -> Response<Empty<Bytes>> {
    let status = if request.method() != Method::POST {
        StatusCode::METHOD_NOT_ALLOWED
    } else {
        match Limited::new(request.into_body(), BODY_LIMIT)
            .collect()
            .await
        {
            Ok(body) => match serde_json::from_slice::<Batch>(&body.to_bytes()) {
                Ok(batch) => {
                    for agent in dispatch.recipients(&batch) {
                        let dispatch = dispatch.clone();
                        let agent = agent.to_owned();
                        tokio::spawn(async move {
                            match dispatch.wake(&agent).await {
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
    });
    let listener = TcpListener::bind(args.listen).await?;
    loop {
        let (stream, _) = listener.accept().await?;
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
        assert_eq!(
            dispatch.recipients(&batch),
            BTreeSet::from(["alpha", "beta"])
        );
    }
}
