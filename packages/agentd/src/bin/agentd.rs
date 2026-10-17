//! The agent VM's root daemon. Every agent uid gets one Claude Code API socket
//! that leads to the host's gateway, tagged with the uid it came from, and the
//! host's wakes run as transient units in the agent's slice.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::Permissions;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agentd::{CONTROL_PORT, HOST_CID, Reply, Request};
use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, copy_bidirectional};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_vsock::{VsockAddr, VsockListener, VsockStream};

/// How long a session stays open after a turn ends. A message injected while
/// the turn was closing starts the next one inside this window.
const QUIET: Duration = Duration::from_secs(5);

#[derive(Parser)]
struct Args {
    /// JSON object of agent name to uid.
    #[arg(long)]
    agents: PathBuf,
    #[arg(long)]
    claude: PathBuf,
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
    agents: HashMap<String, u32>,
    sessions: Mutex<HashMap<String, Session>>,
    slots: Arc<Semaphore>,
}

impl State {
    fn agent_of(&self, uid: u32) -> Option<&str> {
        self.agents
            .iter()
            .find(|&(_, &u)| u == uid)
            .map(|(name, _)| name.as_str())
    }

    fn ca(&self) -> PathBuf {
        self.args.run_dir.join("ca.pem")
    }

    async fn wake(self: &Arc<Self>, agent: &str, message: &str) -> Result<Woken> {
        let &uid = self
            .agents
            .get(agent)
            .with_context(|| format!("no agent named {agent}"))?;
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
        let mut session = Session {
            stdin: child.stdin.take().expect("piped"),
            injected: Instant::now(),
        };
        session.send(message).await?;
        sessions.insert(agent.to_owned(), session);
        let stdout = child.stdout.take().expect("piped");
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(
            self.clone()
                .drive(agent.to_owned(), child, stdout, tx, slot),
        );
        Ok(Woken::Started(rx))
    }

    async fn refresh_ca(&self) -> Result<()> {
        let mut gateway = VsockStream::connect(VsockAddr::new(HOST_CID, self.args.ca_port)).await?;
        let mut pem = Vec::new();
        gateway.read_to_end(&mut pem).await?;
        let staged = self.ca().with_extension("new");
        tokio::fs::write(&staged, &pem).await?;
        tokio::fs::set_permissions(&staged, Permissions::from_mode(0o644)).await?;
        tokio::fs::rename(&staged, self.ca()).await?;
        Ok(())
    }

    fn spawn(&self, agent: &str, uid: u32) -> Result<Child> {
        let home = Path::new("/home").join(agent);
        let config = home.join(".claude");
        let cache = home.join(".cache");
        let tmp = cache.join("tmp");
        for dir in [&config, &cache, &tmp] {
            std::fs::create_dir_all(dir)?;
            chown(dir, Some(uid), None)?;
        }
        let path = format!(
            "{home}/.nix-profile/bin:/etc/profiles/per-user/{agent}/bin:/run/wrappers/bin:/run/current-system/sw/bin",
            home = home.display()
        );
        let env = [
            ("PATH", path),
            (
                "ANTHROPIC_UNIX_SOCKET",
                self.args.run_dir.join("api.sock").display().to_string(),
            ),
            ("CLAUDE_CODE_OAUTH_TOKEN", "ssh-placeholder".into()),
            ("NODE_EXTRA_CA_CERTS", self.ca().display().to_string()),
            ("CLAUDE_CONFIG_DIR", config.display().to_string()),
            ("CLAUDE_CODE_PROJECT_DIR_NAME", agent.into()),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
            ("TMPDIR", tmp.display().to_string()),
        ];
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        Command::new("systemd-run")
            .args(["--quiet", "--pipe", "--wait", "--collect"])
            .arg(format!("--unit=agent-{agent}-{stamp}"))
            .arg(format!("--slice=agent-{agent}.slice"))
            .arg(format!("--uid={uid}"))
            .arg(format!("--working-directory={}", home.display()))
            .arg(format!("--property=MemoryMax={}", self.args.memory_max))
            .args(
                env.iter()
                    .map(|(name, value)| format!("--setenv={name}={value}")),
            )
            .arg("--")
            .arg(&self.args.claude)
            .args(["-p", "--verbose", "--dangerously-skip-permissions"])
            .args([
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("start systemd-run")
    }

    /// Relay the session's output until it exits, closing its stdin once a
    /// turn has ended and nothing was injected after it.
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
        eprintln!("{agent}: session exited with {code:?}");
        let _ = tx.send(Reply::Exited { code });
    }
}

fn has_quota(device: &Path, uid: u32) -> Result<bool> {
    let device = CString::new(device.as_os_str().as_bytes())?;
    let mut quota: libc::dqblk = unsafe { std::mem::zeroed() };
    let command = libc::QCMD(libc::Q_GETQUOTA, libc::USRQUOTA);
    // SAFETY: `device` is a NUL-terminated path and `quota` the dqblk Q_GETQUOTA fills.
    let status = unsafe {
        libc::quotactl(
            command,
            device.as_ptr(),
            uid as libc::c_int,
            (&raw mut quota).cast(),
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error()).context("quotactl Q_GETQUOTA");
    }
    Ok(quota.dqb_bhardlimit != 0)
}

async fn relay(listener: UnixListener, state: Arc<State>) -> Result<()> {
    loop {
        let (client, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = tunnel(client, &state).await {
                eprintln!("api tunnel: {e:#}");
            }
        });
    }
}

async fn tunnel(mut client: UnixStream, state: &State) -> Result<()> {
    let uid = client.peer_cred()?.uid();
    if state.agent_of(uid).is_none() {
        bail!("uid {uid} is not an agent");
    }
    let mut gateway =
        VsockStream::connect(VsockAddr::new(HOST_CID, state.args.gateway_port)).await?;
    gateway.write_all(format!("uid {uid}\n").as_bytes()).await?;
    copy_bidirectional(&mut client, &mut gateway).await?;
    Ok(())
}

async fn serve(listener: VsockListener, state: Arc<State>) -> Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        if peer.cid() != HOST_CID {
            continue;
        }
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = control(stream, state).await {
                eprintln!("control: {e:#}");
            }
        });
    }
}

async fn control(stream: VsockStream, state: Arc<State>) -> Result<()> {
    let (read, mut write) = tokio::io::split(stream);
    let mut request = String::new();
    BufReader::new(read).read_line(&mut request).await?;
    let Request::Wake { agent, message } = serde_json::from_str(&request)?;
    match state.wake(&agent, &message).await {
        Ok(Woken::Injected) => write.write_all(Reply::Injected.line().as_bytes()).await?,
        Ok(Woken::Started(mut replies)) => {
            eprintln!("{agent}: session started");
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
            write
                .write_all(Reply::Refused { reason }.line().as_bytes())
                .await?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let agents = serde_json::from_slice(&std::fs::read(&args.agents)?)
        .with_context(|| format!("read {}", args.agents.display()))?;
    let socket = args.run_dir.join("api.sock");
    let _ = std::fs::remove_file(&socket);
    let api = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, Permissions::from_mode(0o666))?;
    let control = VsockListener::bind(VsockAddr::new(libc::VMADDR_CID_ANY, CONTROL_PORT))?;
    let state = Arc::new(State {
        slots: Arc::new(Semaphore::new(args.sessions)),
        args,
        agents,
        sessions: Mutex::default(),
    });
    tokio::try_join!(relay(api, state.clone()), serve(control, state))?;
    Ok(())
}
