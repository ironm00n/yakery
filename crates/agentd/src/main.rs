//! The agent VM's root daemon: Claude Code reaches the host's gateway through it, each connection tagged with the
//! uid it came from, and the host's wakes run as transient units in the agent's slice.

use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::NonZero;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use agent_proto::{
    CA, CONTROL_PORT, FIRST_REPLY, HOST_CID, Key, Reply, Request, Service, WAKE, challenge, prove,
    read_agents, read_key,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::Parser;
use private::{Dir, Mode, Replace, listen_fds};
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
    copy_bidirectional,
};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Mutex, MutexGuard, Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::Instant;
use tokio_vsock::{VsockAddr, VsockListener, VsockStream};
use yakery_log::{note, warn};

/// How long a session stays open after a turn ends. A message injected while
/// the turn was closing starts the next one inside this window.
const QUIET: Duration = Duration::from_secs(5);

const MAX_REQUEST: u64 = 1 << 20;

/// The longest stdout line relayed as a session's event: the API's limit on one Messages request, which no event in
/// the conversation can exceed.
const MAX_EVENT: usize = 128 << 20;

/// Wakes queued for a session past what its stdin pipe holds. One more is refused: the session has stopped reading,
/// and queueing on would only grow agentd's memory.
const QUEUED: usize = 8;

/// How long a start waits for the gateway's CA once the handshake is done.
const CA_FETCH: Duration = Duration::from_secs(10);

/// Loopback owner lookups at once, as many as the runtime has workers: the bound the reads had when they ran on
/// them, so a burst drains as fast as before, with no more copies of the socket table at once.
static LOOKUPS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(workers()));

/// As tokio counts its runtime's workers.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, NonZero::get)
}

#[derive(Parser)]
struct Args {
    /// JSON object of agent name to uid.
    #[arg(long)]
    agents: PathBuf,
    /// The key shared with the host, which only root can read.
    #[arg(long)]
    key: PathBuf,
    #[arg(long)]
    claude: PathBuf,
    #[arg(long)]
    mkdir: PathBuf,
    /// Block device of the homes filesystem, whose quotas gate every wake.
    #[arg(long)]
    quota_device: PathBuf,
    #[arg(long, default_value = "/run/agentd")]
    run_dir: PathBuf,
    #[arg(long, default_value_t = 5000)]
    gateway_port: u32,
    #[arg(long, default_value_t = 5001)]
    ca_port: u32,
    #[arg(long, default_value = "8G")]
    memory_max: String,
    /// Sessions that may run at once; wakes past it wait for one to end.
    #[arg(long)]
    sessions: usize,
}

struct Session {
    id: u64,
    input: mpsc::Sender<String>,
    injected: Instant,
}

impl Session {
    /// Session `id`, whose stdin a task of its own writes, so that no wake waits on a session that has stopped
    /// reading. Dropping the session ends its input.
    fn new(id: u64, mut stdin: ChildStdin) -> Self {
        let (input, mut queued) = mpsc::channel::<String>(QUEUED);
        tokio::spawn(async move {
            while let Some(line) = queued.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        Session { id, input, injected: Instant::now() }
    }

    fn send(&mut self, message: &str) -> Result<()> {
        let line = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": message },
            "parent_tool_use_id": null,
            "priority": "next",
        });
        self.input.try_send(format!("{line}\n")).map_err(|refused| match refused {
            TrySendError::Full(_) => anyhow!("the session has stopped reading its input"),
            TrySendError::Closed(_) => anyhow!("the session's input has closed"),
        })?;
        self.injected = Instant::now();
        Ok(())
    }
}

/// An agent's session. An ending one has lost its input but not yet exited, and holds back the agent's next session,
/// so that two never run in one home.
enum Run {
    Idle,
    Running(Session),
    Ending(u64),
}

impl Run {
    fn id(&self) -> Option<u64> {
        match self {
            Run::Idle => None,
            Run::Running(session) => Some(session.id),
            Run::Ending(id) => Some(*id),
        }
    }
}

/// What a session tells the waker that started it: its events, the oldest dropped first past `budget` (each counted
/// with EVENT_COST) but the newest always kept; then how many were dropped; then its exit. The session never waits on
/// its waker, and once the waker's `Reader` is gone the relay holds nothing.
struct Relay {
    budget: usize,
    queue: std::sync::Mutex<Queue>,
    ready: Notify,
}

/// What an event costs the relay beyond its bytes: the four queue slots `trim` lets it keep and the allocator's
/// smallest chunk.
const EVENT_COST: usize = 4 * size_of::<Box<RawValue>>() + 32;

fn cost(event: &RawValue) -> usize {
    event.get().len() + EVENT_COST
}

#[derive(Default)]
struct Queue {
    events: VecDeque<Box<RawValue>>,
    cost: usize,
    dropped: u64,
    end: End,
    unread: bool,
}

impl Queue {
    /// Gives back the room the queue grew into for events since dropped or read.
    fn trim(&mut self) {
        if self.events.capacity() > 64.max(4 * self.events.len()) {
            self.events.shrink_to(2 * self.events.len());
        }
    }
}

#[derive(Default)]
enum End {
    #[default]
    Running,
    Exited(Option<i32>),
    Told,
}

impl Relay {
    /// The relay, and its one reader.
    fn new(budget: usize) -> (Arc<Self>, Reader) {
        let relay = Arc::new(Relay { budget, queue: Default::default(), ready: Notify::new() });
        (relay.clone(), Reader(relay))
    }

    /// Nothing that holds the lock can panic midway, so a poisoned queue is still whole.
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn event(&self, event: Box<RawValue>) {
        let mut queue = self.queue();
        if queue.unread {
            return;
        }
        queue.cost += cost(&event);
        queue.events.push_back(event);
        while queue.cost > self.budget && queue.events.len() > 1 {
            let oldest = queue.events.pop_front().expect("more than one");
            queue.cost -= cost(&oldest);
            queue.dropped += 1;
        }
        queue.trim();
        drop(queue);
        self.ready.notify_one();
    }

    fn exit(&self, code: Option<i32>) {
        self.queue().end = End::Exited(code);
        self.ready.notify_one();
    }

    /// None once the exit has been told; the one reader waits on `ready`, whose permit outlives a missed wakeup.
    async fn next(&self) -> Option<Reply> {
        loop {
            {
                let mut queue = self.queue();
                if let Some(event) = queue.events.pop_front() {
                    queue.cost -= cost(&event);
                    queue.trim();
                    return Some(Reply::Event(event));
                }
                match queue.end {
                    End::Running => {}
                    End::Exited(_) if queue.dropped > 0 => {
                        return Some(Reply::Dropped { events: std::mem::take(&mut queue.dropped) });
                    }
                    End::Exited(code) => {
                        queue.end = End::Told;
                        return Some(Reply::Exited { code });
                    }
                    End::Told => return None,
                }
            }
            self.ready.notified().await;
        }
    }
}

/// The waker's end of a relay: while it is held the relay keeps events for it, and once it is gone, nothing.
struct Reader(Arc<Relay>);

impl Reader {
    async fn next(&self) -> Option<Reply> {
        self.0.next().await
    }
}

impl Drop for Reader {
    /// The events are freed after the lock is let go, so `drive` doesn't wait on their freeing.
    fn drop(&mut self) {
        let mut queue = self.0.queue();
        queue.unread = true;
        queue.cost = 0;
        let events = std::mem::take(&mut queue.events);
        drop(queue);
        drop(events);
    }
}

enum Woken {
    Started(Reader),
    Injected,
    Busy,
}

/// Lines of at most `max` bytes, the last with or without its newline: a longer one is skipped whole, so no writer
/// makes the reader hold more.
struct Lines<R> {
    reader: R,
    max: usize,
    line: Vec<u8>,
    over: bool,
}

enum Line {
    Whole(Vec<u8>),
    Over,
}

impl<R: AsyncBufRead + Unpin> Lines<R> {
    fn new(reader: R, max: usize) -> Self {
        Lines { reader, max, line: Vec::new(), over: false }
    }

    /// Cancel safe: a partly read line waits in `line`.
    async fn next(&mut self) -> std::io::Result<Option<Line>> {
        loop {
            let buf = self.reader.fill_buf().await?;
            if buf.is_empty() {
                return Ok((self.over || !self.line.is_empty()).then(|| self.finish()));
            }
            let end = buf.iter().position(|&b| b == b'\n');
            let chunk = &buf[..end.unwrap_or(buf.len())];
            self.over |= self.line.len() + chunk.len() > self.max;
            if self.over {
                self.line.clear();
            } else {
                self.line.extend_from_slice(chunk);
            }
            let read = end.map_or(buf.len(), |end| end + 1);
            self.reader.consume(read);
            if end.is_some() {
                return Ok(Some(self.finish()));
            }
        }
    }

    fn finish(&mut self) -> Line {
        let line = std::mem::take(&mut self.line);
        if std::mem::take(&mut self.over) { Line::Over } else { Line::Whole(line) }
    }
}

struct Agent {
    uid: u32,
    run: Mutex<Run>,
    /// Told of every change of `run`, for the wakes waiting on one.
    changed: Notify,
}

impl Agent {
    fn set(&self, run: &mut Run, next: Run) {
        *run = next;
        self.changed.notify_waiters();
    }

    /// Moves session `id` to `next` if it is still the agent's and `now` holds of it, so that a session never changes
    /// a later one.
    async fn advance(&self, id: u64, now: impl FnOnce(&Run) -> bool, next: Run) {
        let mut run = self.run.lock().await;
        if run.id() == Some(id) && now(&run) {
            self.set(&mut run, next);
        }
    }
}

struct State {
    args: Args,
    api: PathBuf,
    key: Key,
    agents: HashMap<String, Agent>,
    slots: Arc<Semaphore>,
    /// The next session's id. Ids count on from the run's start in nanoseconds, so a run's sessions take none an
    /// earlier run's took.
    started: AtomicU64,
}

impl State {
    fn new(args: Args, api: PathBuf, key: Key, agents: HashMap<String, u32>) -> Self {
        let agents = agents.into_iter().map(|(name, uid)| {
            (name, Agent { uid, run: Mutex::new(Run::Idle), changed: Notify::new() })
        });
        State {
            slots: Arc::new(Semaphore::new(args.sessions)),
            args,
            api,
            key,
            agents: agents.collect(),
            started: AtomicU64::new(nanos_since_epoch()),
        }
    }

    fn agent_of(&self, uid: u32) -> Option<&str> {
        self.agents.iter().find(|(_, agent)| agent.uid == uid).map(|(name, _)| name.as_str())
    }

    /// Busy if no session takes `message` within FIRST_REPLY. Until it spawns a session a wake only waits, for the
    /// agent's lock, a slot, a change or the CA, so dropping it there leaves nothing to start later.
    async fn wake(self: &Arc<Self>, name: &str, message: &str) -> Result<Woken> {
        let handed = tokio::time::timeout(FIRST_REPLY, self.hand(name, message)).await;
        handed.unwrap_or(Ok(Woken::Busy))
    }

    async fn hand(self: &Arc<Self>, name: &str, message: &str) -> Result<Woken> {
        let agent = self.agents.get(name).with_context(|| format!("no agent named {name}"))?;
        let mut slot = None;
        loop {
            let changed = agent.changed.notified();
            let mut changed = std::pin::pin!(changed);
            changed.as_mut().enable();
            let mut run = match agent.run.try_lock() {
                Ok(run) => run,
                // A start holds the lock across its CA fetch: wait for it with no slot held.
                Err(_) => {
                    slot = None;
                    agent.run.lock().await
                }
            };
            let idle = match &mut *run {
                Run::Running(session) => return session.send(message).map(|()| Woken::Injected),
                Run::Idle => true,
                Run::Ending(_) => false,
            };
            if !idle {
                slot = None;
            } else if let Some(slot) =
                slot.take().or_else(|| self.slots.clone().try_acquire_owned().ok())
            {
                return self.start(name, agent, run, slot, message).await;
            }
            drop(run);
            // Unlocked while it waits: a session's end takes its agent's lock before giving its slot back.
            tokio::select! {
                permit = self.slots.clone().acquire_owned(), if idle => slot = Some(permit?),
                () = changed => {}
            }
        }
    }

    async fn start(
        self: &Arc<Self>,
        name: &str,
        agent: &Agent,
        mut run: MutexGuard<'_, Run>,
        slot: OwnedSemaphorePermit,
        message: &str,
    ) -> Result<Woken> {
        ensure!(has_quota(&self.args.quota_device, agent.uid)?, "{name} has no home quota");
        self.refresh_ca().await.context("fetch the gateway's CA")?;
        let id = self.started.fetch_add(1, Ordering::Relaxed);
        let mut child = self.spawn(name, agent.uid, id)?;
        let mut session = Session::new(id, child.stdin.take().expect("piped"));
        session.send(message)?;
        agent.set(&mut run, Run::Running(session));
        let stdout = child.stdout.take().expect("piped");
        let (relay, reader) = Relay::new(MAX_EVENT);
        tokio::spawn(self.clone().drive(name.to_owned(), id, child, stdout, relay, slot));
        Ok(Woken::Started(reader))
    }

    async fn refresh_ca(&self) -> Result<()> {
        let mut gateway = VsockStream::connect(VsockAddr::new(HOST_CID, self.args.ca_port)).await?;
        prove(&mut gateway, &self.key, Service::Ca, CA).await?;
        let mut pem = Vec::new();
        tokio::time::timeout(CA_FETCH, gateway.read_to_end(&mut pem))
            .await
            .context("the CA did not arrive")??;
        let dir = Dir::open(&self.args.run_dir, &[])?;
        dir.drop_stale_partials()?;
        dir.write("ca.pem", &pem, Mode::Readable, Replace::Yes).map(drop)
    }

    fn spawn(&self, agent: &str, uid: u32, id: u64) -> Result<Child> {
        let home = Path::new("/home").join(agent);
        let config = home.join(".claude");
        let tmp = home.join(".cache").join("tmp");
        let path = format!(
            "{home}/.nix-profile/bin:/etc/profiles/per-user/{agent}/bin:/run/wrappers/bin:/run/current-system/sw/bin",
            home = home.display()
        );
        let env = [
            ("PATH", path),
            ("ANTHROPIC_UNIX_SOCKET", self.api.display().to_string()),
            ("CLAUDE_CODE_OAUTH_TOKEN", "ssh-placeholder".into()),
            ("NODE_EXTRA_CA_CERTS", self.args.run_dir.join("ca.pem").display().to_string()),
            ("CLAUDE_CONFIG_DIR", config.display().to_string()),
            ("CLAUDE_CODE_PROJECT_DIR_NAME", agent.into()),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
            ("TMPDIR", tmp.display().to_string()),
        ];
        Command::new("systemd-run")
            .args(["--quiet", "--pipe", "--wait", "--collect"])
            .arg(format!("--unit={}", unit(agent, id)))
            .arg(format!("--slice=agent-{agent}.slice"))
            .arg(format!("--uid={uid}"))
            .arg(format!("--working-directory={}", home.display()))
            .arg(format!("--property=MemoryMax={}", self.args.memory_max))
            // Made by the session itself: agentd, as root, would follow
            // symlinks the agent planted in its home.
            .arg(format!(
                "--property=ExecStartPre={} -p {} {}",
                self.args.mkdir.display(),
                config.display(),
                tmp.display()
            ))
            // Only agentd reaches the host over vsock, so the uid it sends
            // the gateway is one it checked.
            .arg("--property=RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK")
            .args(env.iter().map(|(name, value)| format!("--setenv={name}={value}")))
            .arg("--")
            .arg(&self.args.claude)
            .args(["-p", "--verbose", "--dangerously-skip-permissions"])
            .args(["--input-format", "stream-json", "--output-format", "stream-json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("start systemd-run")
    }

    async fn drive(
        self: Arc<Self>,
        agent: String,
        id: u64,
        mut child: Child,
        stdout: ChildStdout,
        relay: Arc<Relay>,
        _slot: OwnedSemaphorePermit,
    ) {
        #[derive(Deserialize)]
        struct Kind {
            r#type: String,
        }
        let mut lines = Lines::new(BufReader::new(stdout), MAX_EVENT);
        let mut ended: Option<Instant> = None;
        loop {
            let quiet = async {
                match ended {
                    Some(at) => tokio::time::sleep_until(at + QUIET).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                line = lines.next() => match line {
                    Ok(Some(Line::Whole(line))) => {
                        let finished = serde_json::from_slice::<Kind>(&line)
                            .is_ok_and(|kind| kind.r#type == "result");
                        ended = finished.then(Instant::now);
                        let event = String::from_utf8(line).ok().map(RawValue::from_string);
                        if let Some(Ok(event)) = event {
                            relay.event(event);
                        }
                    }
                    Ok(Some(Line::Over)) => {
                        ended = None;
                        warn!("{agent}: dropped an event over {MAX_EVENT} bytes");
                    }
                    _ => break,
                },
                () = quiet => {
                    let at = ended.take().expect("armed");
                    let unwoken = |run: &Run| matches!(run, Run::Running(s) if s.injected <= at);
                    self.agents[&agent].advance(id, unwoken, Run::Ending(id)).await;
                }
            }
        }
        self.agents[&agent].advance(id, |_| true, Run::Ending(id)).await;
        let code = child.wait().await.ok().and_then(|status| status.code());
        self.agents[&agent].advance(id, |_| true, Run::Idle).await;
        note!("{agent}: session exited with {code:?}");
        relay.exit(code);
    }
}

fn unit(agent: &str, id: u64) -> String {
    format!("agent-{agent}-{id}")
}

fn nanos_since_epoch() -> u64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    since.map_or(0, |since| since.as_nanos() as u64)
}

#[allow(unsafe_code)]
fn has_quota(device: &Path, uid: u32) -> Result<bool> {
    let device = CString::new(device.as_os_str().as_bytes())?;
    let mut quota: libc::dqblk = unsafe { std::mem::zeroed() };
    let command = libc::QCMD(libc::Q_GETQUOTA, libc::USRQUOTA);
    // SAFETY: `device` is a NUL-terminated path and `quota` the dqblk Q_GETQUOTA fills.
    let status = unsafe {
        libc::quotactl(command, device.as_ptr(), uid as libc::c_int, (&raw mut quota).cast())
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error()).context("quotactl Q_GETQUOTA");
    }
    Ok(quota.dqb_bhardlimit != 0)
}

trait Local {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    async fn next(&self) -> std::io::Result<Self::Stream>;

    fn uid(stream: &Self::Stream) -> impl Future<Output = Result<u32>> + Send;
}

impl Local for UnixListener {
    type Stream = UnixStream;

    async fn next(&self) -> std::io::Result<UnixStream> {
        Ok(self.accept().await?.0)
    }

    async fn uid(stream: &UnixStream) -> Result<u32> {
        Ok(stream.peer_cred()?.uid())
    }
}

impl Local for TcpListener {
    type Stream = TcpStream;

    async fn next(&self) -> std::io::Result<TcpStream> {
        Ok(self.accept().await?.0)
    }

    /// On loopback the client's end is in this kernel's table too, with its owner. The read takes time in
    /// proportion to every socket in the VM, so it runs on a blocking thread.
    async fn uid(stream: &TcpStream) -> Result<u32> {
        let (SocketAddr::V4(client), SocketAddr::V4(server)) =
            (stream.peer_addr()?, stream.local_addr()?)
        else {
            bail!("not an IPv4 connection");
        };
        loopback_owner(client, server, &LOOKUPS).await
    }
}

/// Held by this task, not by the read: a read that never gets a thread must not keep its turn.
async fn loopback_owner(
    client: SocketAddrV4,
    server: SocketAddrV4,
    turns: &Semaphore,
) -> Result<u32> {
    let _turn = turns.acquire().await?;
    tokio::task::spawn_blocking(move || owner(client, server)).await?
}

fn owner(client: SocketAddrV4, server: SocketAddrV4) -> Result<u32> {
    std::fs::read_to_string("/proc/net/tcp")?
        .lines()
        .skip(1)
        .find_map(|row| {
            let mut fields = row.split_whitespace().skip(1);
            let local = proc_net_address(fields.next()?)?;
            let remote = proc_net_address(fields.next()?)?;
            if (local, remote) != (client, server) {
                return None;
            }
            fields.nth(4)?.parse().ok()
        })
        .with_context(|| format!("no socket owner for {client}"))
}

/// An address as /proc/net/tcp writes it: the IPv4 address as one hex word in
/// host order, then the port in hex.
fn proc_net_address(field: &str) -> Option<SocketAddrV4> {
    let (ip, port) = field.split_once(':')?;
    let ip = Ipv4Addr::from(u32::from_str_radix(ip, 16).ok()?.to_ne_bytes());
    Some(SocketAddrV4::new(ip, u16::from_str_radix(port, 16).ok()?))
}

async fn relay<L: Local>(what: &str, listener: L, state: Arc<State>) -> Result<()> {
    loop {
        let client = yakery_serve::accept(what, || listener.next()).await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = tunnel::<L>(client, &state).await {
                warn!("api tunnel: {e:#}");
            }
        });
    }
}

async fn tunnel<L: Local>(mut client: L::Stream, state: &State) -> Result<()> {
    let uid = L::uid(&client).await?;
    if state.agent_of(uid).is_none() {
        bail!("uid {uid} is not an agent");
    }
    let mut gateway =
        VsockStream::connect(VsockAddr::new(HOST_CID, state.args.gateway_port)).await?;
    prove(&mut gateway, &state.key, Service::Gateway, &format!("uid {uid}")).await?;
    copy_bidirectional(&mut client, &mut gateway).await?;
    Ok(())
}

async fn serve(listener: VsockListener, state: Arc<State>) -> Result<()> {
    loop {
        let (stream, peer) = yakery_serve::accept("the control port", || listener.accept()).await?;
        if peer.cid() != HOST_CID {
            continue;
        }
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = control(stream, state).await {
                warn!("control: {e:#}");
            }
        });
    }
}

async fn control(stream: VsockStream, state: Arc<State>) -> Result<()> {
    let mut stream = BufReader::new(stream);
    let claim = challenge(&mut stream, &state.key, Service::Control).await?;
    if claim != WAKE {
        bail!("a control connection claimed {claim:?}");
    }
    let mut request = String::new();
    (&mut stream).take(MAX_REQUEST).read_line(&mut request).await?;
    let Request::Wake { agent, message } = serde_json::from_str(&request)?;
    let mut write = stream.into_inner();
    match state.wake(&agent, &message).await {
        Ok(Woken::Injected) => write.write_all(Reply::Injected.line().as_bytes()).await?,
        Ok(Woken::Busy) => write.write_all(Reply::Busy.line().as_bytes()).await?,
        Ok(Woken::Started(relay)) => {
            note!("{agent}: session started");
            write.write_all(Reply::Started.line().as_bytes()).await?;
            // A host that hung up stops listening, and the reader's drop empties the relay; the session runs on.
            while let Some(reply) = relay.next().await {
                if write.write_all(reply.line().as_bytes()).await.is_err() {
                    break;
                }
            }
        }
        Err(e) => {
            let reason = format!("{e:#}");
            write.write_all(Reply::Refused { reason }.line().as_bytes()).await?;
        }
    }
    Ok(())
}

/// systemd binds the sockets (agentd's socket units), so their modes are in Nix and connections queue while agentd
/// restarts; it passes them by name before any thread starts, as `listen_fds` requires.
fn main() -> ExitCode {
    let args = Args::parse();
    let started = listen_fds().and_then(|passed| {
        let mut named = HashMap::new();
        for (name, socket) in passed {
            ensure!(
                named.insert(name.clone(), socket).is_none(),
                "systemd passed two sockets named {name}"
            );
        }
        let mut take = |name: &str| {
            let socket = named.remove(name);
            socket.with_context(|| {
                format!("systemd passed no socket named {name}; agentd runs from its socket units")
            })
        };
        let (api, direct, control) = (take("api")?, take("direct")?, take("control")?);
        ensure!(
            named.is_empty(),
            "systemd passed sockets agentd does not know: {:?}",
            named.keys()
        );
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run(args, api, direct, control))
    });
    yakery_log::exit_code(started.map(|()| ExitCode::SUCCESS))
}

async fn run(args: Args, api: OwnedFd, direct: OwnedFd, control: OwnedFd) -> Result<()> {
    let (api, direct) =
        (std::os::unix::net::UnixListener::from(api), std::net::TcpListener::from(direct));
    api.set_nonblocking(true)?;
    direct.set_nonblocking(true)?;
    let path = api.local_addr()?.as_pathname().context("the api socket has no path")?.into();
    let (key, agents) = (read_key(&args.key)?, read_agents(&args.agents)?);
    let state = Arc::new(State::new(args, path, key, agents));
    // SAFETY: the fd is open, and the listener becomes its one owner.
    #[allow(unsafe_code)]
    let control = unsafe { VsockListener::from_raw_fd(control.into_raw_fd()) };
    let port = control.local_addr()?.port();
    ensure!(port == CONTROL_PORT, "the control socket is on port {port}, not {CONTROL_PORT}");
    let (api, direct) = (UnixListener::from_std(api)?, TcpListener::from_std(direct)?);
    tokio::try_join!(
        relay("the api socket", api, state.clone()),
        relay("the direct port", direct, state.clone()),
        serve(control, state)
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/main.rs"]
mod tests;
