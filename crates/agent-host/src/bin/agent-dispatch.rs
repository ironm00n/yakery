//! Wakes an agent whenever the mail server reports a delivery attempt to it: Stalwart posts events here as a webhook
//! (which events is the stalwart bundle's `deliveryWebhooks`), and every recipient in an event's `to` on the agent
//! domain whose name is on the agent list gets a wake.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use agent_host::Vm;
use agent_proto::{FIRST_REPLY, Key, Reply, Request, read_agents};
use anyhow::{Result, bail};
use bytes::Bytes;
use clap::Parser;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Response, StatusCode};
use hyper_util::rt::TokioIo;
use private::Dir;
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
    /// Where the agents owed a wake are saved, so a restart wakes them again.
    #[arg(long)]
    state: PathBuf,
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

/// The agents owed a wake, each with the mails taken for it since; their names are saved to `owed.json`, so a
/// restart wakes each again.
struct Owed {
    dir: Dir,
    mails: Mutex<BTreeMap<String, u64>>,
}

impl Owed {
    const FILE: &str = "owed.json";

    /// Those an earlier run left under `path`, for the agents still `listed`.
    fn open(path: &Path, listed: impl Fn(&str) -> bool) -> Result<Self> {
        let dir = Dir::open(path, &[])?;
        dir.drop_stale_partials()?;
        let names: BTreeSet<String> = dir.load(Self::FILE)?;
        let mails =
            names.into_iter().filter(|agent| listed(agent)).map(|agent| (agent, 1)).collect();
        Ok(Owed { dir, mails: Mutex::new(mails) })
    }

    fn mails(&self) -> MutexGuard<'_, BTreeMap<String, u64>> {
        self.mails.lock().expect("no panic holds the mails")
    }

    fn agents(&self) -> Vec<String> {
        self.mails().keys().cloned().collect()
    }

    /// One more mail for `agent`: true if it was owed nothing, so no wake for it is under way. Saved every time, so
    /// a mail retries a save that failed.
    fn owe(&self, agent: &str) -> bool {
        let mut mails = self.mails();
        let first = !mails.contains_key(agent);
        *mails.entry(agent.to_owned()).or_default() += 1;
        self.save(&mails);
        first
    }

    /// The mails taken for `agent` so far, which a wake begun now covers.
    fn seen(&self, agent: &str) -> u64 {
        self.mails().get(agent).copied().unwrap_or(0)
    }

    /// Clears what `agent` is owed unless a mail came after `seen`; true if it did.
    fn settle(&self, agent: &str, seen: u64) -> bool {
        let mut mails = self.mails();
        let settled = mails.get(agent) == Some(&seen);
        if settled {
            mails.remove(agent);
            self.save(&mails);
        }
        settled
    }

    /// Wakes `agent` with `wake` until a wake begun after its last mail has ended.
    async fn until_settled<F: Future<Output = ()>>(
        &self,
        agent: &str,
        mut wake: impl FnMut() -> F,
    ) {
        loop {
            let seen = self.seen(agent);
            wake().await;
            if self.settle(agent, seen) {
                break;
            }
        }
    }

    /// A failed save leaves the names to the next one.
    fn save(&self, mails: &BTreeMap<String, u64>) {
        if let Err(e) = self.dir.save(Self::FILE, &mails.keys().collect::<Vec<_>>()) {
            warn!("could not save the owed wakes; a restart now would get them wrong: {e:#}");
        }
    }
}

struct Dispatch {
    /// Each listed agent's address on the agent domain, and its name.
    agents: Vec<(Address, String)>,
    cid: u32,
    key: Key,
    owed: Owed,
}

impl Dispatch {
    fn new(
        names: impl IntoIterator<Item = String>,
        domain: &str,
        cid: u32,
        key: Key,
        state: &Path,
    ) -> Result<Self> {
        let address = |name: String| Ok((format!("{name}@{domain}").parse()?, name));
        let agents: Vec<(Address, String)> =
            names.into_iter().map(address).collect::<Result<_>>()?;
        let owed = Owed::open(state, |agent| agents.iter().any(|(_, name)| name == agent))?;
        Ok(Dispatch { agents, cid, key, owed })
    }

    /// The agents `batch` names that were owed nothing, whose wakes must start; the mail server is answered after.
    fn take(&self, batch: &Batch) -> Vec<String> {
        self.recipients(batch).into_iter().filter(|agent| self.owed.owe(agent)).collect()
    }

    /// Wakes `agent` until a wake begun after its last mail is answered or given up.
    fn spawn_wakes(self: &Arc<Self>, agent: String) {
        let dispatch = self.clone();
        tokio::spawn(async move {
            let (dispatching, agent) = (&dispatch, &agent);
            let wake = move || async move {
                match dispatching.wake_until_answered(agent).await {
                    Ok(reply) => note!("{agent}: {reply:?}"),
                    Err(e) => fail!("{agent}: wake failed: {e:#}"),
                }
            };
            dispatch.owed.until_settled(agent, wake).await;
        });
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

    async fn wake(&self, agent: &str) -> Result<Reply> {
        let request = Request::Wake { agent: agent.to_owned(), message: MESSAGE.to_owned() };
        let busy = |_: &str| {
            warn!("{agent}: no session took the wake within {FIRST_REPLY:?}, asking again")
        };
        let (first, _) = agent_proto::wake(self.cid, &self.key, &request, busy).await?;
        taken(&first)
    }

    async fn wake_until_answered(&self, agent: &str) -> Result<Reply> {
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

/// The daemon's first reply, if it took the wake: a refusal is retried like a failure to reach it.
fn taken(line: &str) -> Result<Reply> {
    match serde_json::from_str(line)? {
        Reply::Refused { reason } => bail!("refused: {reason}"),
        reply => Ok(reply),
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
                    for agent in dispatch.take(&batch) {
                        dispatch.spawn_wakes(agent);
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
    let dispatch =
        Arc::new(Dispatch::new(names, &args.domain, args.vm.cid, args.vm.key()?, &args.state)?);
    for agent in dispatch.owed.agents() {
        dispatch.spawn_wakes(agent);
    }
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
