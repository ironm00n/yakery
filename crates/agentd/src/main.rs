//! The agent VM's root daemon: Claude Code reaches the host's gateway through it, each connection tagged with the
//! uid it came from, and the host's wakes run as transient units in the agent's slice.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::Permissions;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_proto::{
    CA, CONTROL_PORT, HOST_CID, Key, Reply, Request, Service, WAKE, accept, challenge, prove,
    read_agents, read_key,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use private::{Dir, Mode, Replace, now};
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
    copy_bidirectional,
};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_vsock::{VsockAddr, VsockListener, VsockStream};
use yakery_log::{note, warn};

/// How long a session stays open after a turn ends. A message injected while
/// the turn was closing starts the next one inside this window.
const QUIET: Duration = Duration::from_secs(5);

const MAX_REQUEST: u64 = 1 << 20;

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
    /// Where the VM's hosts file sends api.anthropic.com, for what Claude Code
    /// fetches there instead of over the API socket.
    #[arg(long)]
    api_address: SocketAddrV4,
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
    stdin: ChildStdin,
    injected: Instant,
}

impl Session {
    async fn send(&mut self, message: &str) -> Result<()> {
        let line = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": message },
            "parent_tool_use_id": null,
            "priority": "next",
        });
        self.stdin.write_all(format!("{line}\n").as_bytes()).await?;
        self.injected = Instant::now();
        Ok(())
    }
}

enum Woken {
    Started(mpsc::UnboundedReceiver<Reply>),
    Injected,
}

struct State {
    args: Args,
    key: Key,
    agents: HashMap<String, u32>,
    sessions: Mutex<HashMap<String, Session>>,
    slots: Arc<Semaphore>,
}

impl State {
    fn agent_of(&self, uid: u32) -> Option<&str> {
        self.agents.iter().find(|&(_, &u)| u == uid).map(|(name, _)| name.as_str())
    }

    async fn wake(self: &Arc<Self>, agent: &str, message: &str) -> Result<Woken> {
        let &uid = self.agents.get(agent).with_context(|| format!("no agent named {agent}"))?;
        if let Some(session) = self.sessions.lock().await.get_mut(agent) {
            session.send(message).await?;
            return Ok(Woken::Injected);
        }
        if !has_quota(&self.args.quota_device, uid)? {
            bail!("{agent} has no home quota");
        }
        let slot = self.slots.clone().acquire_owned().await?;
        let mut sessions = self.sessions.lock().await;
        // Another wake may have started this agent while this one waited.
        if let Some(session) = sessions.get_mut(agent) {
            session.send(message).await?;
            return Ok(Woken::Injected);
        }
        self.refresh_ca().await.context("fetch the gateway's CA")?;
        let mut child = self.spawn(agent, uid)?;
        let mut session =
            Session { stdin: child.stdin.take().expect("piped"), injected: Instant::now() };
        session.send(message).await?;
        sessions.insert(agent.to_owned(), session);
        let stdout = child.stdout.take().expect("piped");
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(self.clone().drive(agent.to_owned(), child, stdout, tx, slot));
        Ok(Woken::Started(rx))
    }

    async fn refresh_ca(&self) -> Result<()> {
        let mut gateway = VsockStream::connect(VsockAddr::new(HOST_CID, self.args.ca_port)).await?;
        prove(&mut gateway, &self.key, Service::Ca, CA).await?;
        let mut pem = Vec::new();
        gateway.read_to_end(&mut pem).await?;
        let dir = Dir::open(&self.args.run_dir, &[])?;
        dir.drop_stale_partials()?;
        dir.write("ca.pem", &pem, Mode::Readable, Replace::Yes).map(drop)
    }

    fn spawn(&self, agent: &str, uid: u32) -> Result<Child> {
        let home = Path::new("/home").join(agent);
        let config = home.join(".claude");
        let tmp = home.join(".cache").join("tmp");
        let path = format!(
            "{home}/.nix-profile/bin:/etc/profiles/per-user/{agent}/bin:/run/wrappers/bin:/run/current-system/sw/bin",
            home = home.display()
        );
        let env = [
            ("PATH", path),
            ("ANTHROPIC_UNIX_SOCKET", self.args.run_dir.join("api.sock").display().to_string()),
            ("CLAUDE_CODE_OAUTH_TOKEN", "ssh-placeholder".into()),
            ("NODE_EXTRA_CA_CERTS", self.args.run_dir.join("ca.pem").display().to_string()),
            ("CLAUDE_CONFIG_DIR", config.display().to_string()),
            ("CLAUDE_CODE_PROJECT_DIR_NAME", agent.into()),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
            ("TMPDIR", tmp.display().to_string()),
        ];
        let stamp = now();
        Command::new("systemd-run")
            .args(["--quiet", "--pipe", "--wait", "--collect"])
            .arg(format!("--unit=agent-{agent}-{stamp}"))
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
        mut child: Child,
        stdout: ChildStdout,
        tx: mpsc::UnboundedSender<Reply>,
        _slot: OwnedSemaphorePermit,
    ) {
        #[derive(Deserialize)]
        struct Kind {
            r#type: String,
        }
        let mut lines = BufReader::new(stdout).lines();
        let mut ended: Option<Instant> = None;
        loop {
            let quiet = async {
                match ended {
                    Some(at) => tokio::time::sleep_until((at + QUIET).into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        let finished = serde_json::from_str::<Kind>(&line)
                            .is_ok_and(|kind| kind.r#type == "result");
                        ended = finished.then(Instant::now);
                        if let Ok(event) = RawValue::from_string(line) {
                            let _ = tx.send(Reply::Event(event));
                        }
                    }
                    _ => break,
                },
                () = quiet => {
                    let at = ended.take().expect("armed");
                    let mut sessions = self.sessions.lock().await;
                    if sessions.get(&agent).is_some_and(|session| session.injected <= at) {
                        sessions.remove(&agent);
                    }
                }
            }
        }
        self.sessions.lock().await.remove(&agent);
        let code = child.wait().await.ok().and_then(|status| status.code());
        note!("{agent}: session exited with {code:?}");
        let _ = tx.send(Reply::Exited { code });
    }
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

    fn uid(stream: &Self::Stream) -> Result<u32>;
}

impl Local for UnixListener {
    type Stream = UnixStream;

    async fn next(&self) -> std::io::Result<UnixStream> {
        Ok(self.accept().await?.0)
    }

    fn uid(stream: &UnixStream) -> Result<u32> {
        Ok(stream.peer_cred()?.uid())
    }
}

impl Local for TcpListener {
    type Stream = TcpStream;

    async fn next(&self) -> std::io::Result<TcpStream> {
        Ok(self.accept().await?.0)
    }

    /// On loopback the client's end is in this kernel's table too, with its
    /// owner.
    fn uid(stream: &TcpStream) -> Result<u32> {
        let (SocketAddr::V4(client), SocketAddr::V4(server)) =
            (stream.peer_addr()?, stream.local_addr()?)
        else {
            bail!("not an IPv4 connection");
        };
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
}

/// An address as /proc/net/tcp writes it: the IPv4 address as one hex word in
/// host order, then the port in hex.
fn proc_net_address(field: &str) -> Option<SocketAddrV4> {
    let (ip, port) = field.split_once(':')?;
    let ip = Ipv4Addr::from(u32::from_str_radix(ip, 16).ok()?.to_ne_bytes());
    Some(SocketAddrV4::new(ip, u16::from_str_radix(port, 16).ok()?))
}

async fn relay<L: Local>(listener: L, state: Arc<State>) {
    loop {
        let client = accept(async || listener.next().await).await;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = tunnel::<L>(client, &state).await {
                warn!("api tunnel: {e:#}");
            }
        });
    }
}

async fn tunnel<L: Local>(mut client: L::Stream, state: &State) -> Result<()> {
    let uid = L::uid(&client)?;
    if state.agent_of(uid).is_none() {
        bail!("uid {uid} is not an agent");
    }
    let mut gateway =
        VsockStream::connect(VsockAddr::new(HOST_CID, state.args.gateway_port)).await?;
    prove(&mut gateway, &state.key, Service::Gateway, &format!("uid {uid}")).await?;
    copy_bidirectional(&mut client, &mut gateway).await?;
    Ok(())
}

async fn serve(listener: VsockListener, state: Arc<State>) {
    loop {
        let (stream, peer) = accept(async || listener.accept().await).await;
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
        Ok(Woken::Started(mut replies)) => {
            note!("{agent}: session started");
            write.write_all(Reply::Started.line().as_bytes()).await?;
            while let Some(reply) = replies.recv().await {
                // A host that hung up stops listening; the session runs on.
                if write.write_all(reply.line().as_bytes()).await.is_err() {
                    while replies.recv().await.is_some() {}
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let agents = read_agents(&args.agents)?;
    let socket = args.run_dir.join("api.sock");
    let _ = std::fs::remove_file(&socket);
    let api = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, Permissions::from_mode(0o666))?;
    let direct = TcpListener::bind(args.api_address).await?;
    let control = VsockListener::bind(VsockAddr::new(libc::VMADDR_CID_ANY, CONTROL_PORT))?;
    let state = Arc::new(State {
        slots: Arc::new(Semaphore::new(args.sessions)),
        key: read_key(&args.key)?,
        args,
        agents,
        sessions: Mutex::default(),
    });
    tokio::join!(relay(api, state.clone()), relay(direct, state.clone()), serve(control, state));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_proc_net_addresses() {
        assert_eq!(
            proc_net_address("0900007F:01BB"),
            Some(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 9), 443))
        );
    }

    #[tokio::test]
    #[allow(unsafe_code)]
    async fn a_loopback_client_is_its_owner() {
        let listener = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 9), 0)).await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let server = listener.next().await.unwrap();
        assert_eq!(TcpListener::uid(&server).unwrap(), unsafe { libc::getuid() });
    }
}
