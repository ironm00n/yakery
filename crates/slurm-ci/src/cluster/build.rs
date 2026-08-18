//! `slurm-ci build <run>`: the compute-node supervisor. Stays on the host
//! (no namespace) holding the callback connection; the gate, the fetch, and
//! the build run in re-exec'd children whose output it relays to `job.out`
//! and to the VPS (§3, §5).

use std::fs;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::sys::statfs::statfs;
use nix::unistd::Pid;

use crate::callback::{self, Client, Hello, Msg, HEARTBEAT};
use crate::cluster::ns;
use crate::cluster::rundir::{JobFile, RunDir};
use crate::cluster::sandbox::BuildStats;
use crate::cluster::site::{self, CiDir, CACHE_DIR, NODE_TMP, NODE_TMP_FLOOR_BYTES};
use crate::ids::RunId;
use crate::proc::env_or;
use crate::report::BuildReport;
use crate::verbs::JobId;
use crate::verdict::exit;

pub const WORKSPACE_PREFIX: &str = "slurm-ci-";

/// Per-job node-local dirs under `/tmp`, UUID-keyed so two jobs can share a
/// node; 0700 so `sharing` co-tenants cannot read checkouts or outputs.
pub struct Workspace {
    pub base: PathBuf,
}

impl Workspace {
    pub fn create(name: &str) -> Result<Self> {
        let base = Path::new(NODE_TMP).join(format!("{WORKSPACE_PREFIX}{name}"));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&base)
            .with_context(|| format!("create {}", base.display()))?;
        let ws = Workspace { base };
        for sub in ["root", "upper", "work", "src", "xtmp"] {
            fs::DirBuilder::new().mode(0o700).create(ws.dir(sub))?;
        }
        Ok(ws)
    }

    pub fn dir(&self, sub: &str) -> PathBuf {
        self.base.join(sub)
    }

    pub fn remove(&self) {
        if let Err(e) = ns::force_remove(&self.base) {
            eprintln!("workspace cleanup: {e:#}");
        }
    }
}

/// Reap leftovers older than the grace: there is no epilog hook, and a node
/// crash or SIGKILL under `volatile` leaves mode-000 workdirs forever (§5).
pub fn reap_stale_workspaces(grace: Duration) {
    let cutoff = SystemTime::now() - grace;
    for entry in fs::read_dir(NODE_TMP).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(WORKSPACE_PREFIX) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.modified().map(|m| m < cutoff).unwrap_or(false) {
            eprintln!("reaping stale workspace {}", entry.path().display());
            let _ = ns::force_remove(&entry.path());
        }
    }
}

/// Pid of the child currently doing the work, for SIGTERM forwarding. Slurm
/// signals the whole step anyway; this keeps the order deterministic.
static CURRENT_CHILD: AtomicI32 = AtomicI32::new(0);

pub fn install_term_forwarding() -> Result<()> {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
    ])?;
    thread::spawn(move || {
        for _ in signals.forever() {
            let pid = CURRENT_CHILD.load(Ordering::Relaxed);
            if pid > 0 {
                let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
            }
        }
    });
    Ok(())
}

/// Where relayed output goes: always `job.out` (our stdout), and the callback
/// while it lives.
pub struct Sink {
    pub client: Option<Arc<Client>>,
}

impl Sink {
    pub fn write(&self, bytes: &[u8]) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
        if let Some(c) = &self.client {
            c.send(&Msg::Data(bytes.to_vec()));
        }
    }

    pub fn line(&self, s: impl AsRef<str>) {
        self.write(format!("{}\n", s.as_ref()).as_bytes());
    }
}

/// Run a child with stdout+stderr merged into one pipe, relayed as it arrives.
pub fn run_relayed(mut cmd: Command, sink: &Sink) -> Result<i32> {
    let (r, w) = nix::unistd::pipe().context("pipe")?;
    let w2: OwnedFd = w.try_clone()?;
    cmd.stdout(Stdio::from(w))
        .stderr(Stdio::from(w2))
        .stdin(Stdio::null());
    let mut child = cmd.spawn().context("spawn child")?;
    drop(cmd);
    CURRENT_CHILD.store(child.id() as i32, Ordering::Relaxed);
    let mut reader = fs::File::from(r);
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => sink.write(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                sink.line(format!("relay read error: {e}"));
                break;
            }
        }
    }
    let status = child.wait()?;
    CURRENT_CHILD.store(0, Ordering::Relaxed);
    use std::os::unix::process::ExitStatusExt;
    Ok(status.code().unwrap_or(128 + status.signal().unwrap_or(0)))
}

pub fn self_exe() -> Command {
    Command::new("/proc/self/exe")
}

pub struct Gate {
    pub bootstrap: PathBuf,
    pub version: String,
}

/// `statfs(2)` magics; nix's named constants are gated off on musl.
const XFS_MAGIC: u64 = 0x5846_5342;
const EXT4_MAGIC: u64 = 0xEF53;

/// Node sanity (§5), before anything is mounted. A failure here is an
/// infrastructure retry owned by `run`, never CI red.
pub fn gate(ci: &CiDir, ws: &Workspace, reap_grace: Duration) -> Result<Gate> {
    let st = statfs(NODE_TMP).with_context(|| format!("statfs {NODE_TMP}"))?;
    let magic = st.filesystem_type().0 as u64;
    if magic != XFS_MAGIC && magic != EXT4_MAGIC {
        bail!("{NODE_TMP} is fs type {magic:#x}, not a local xfs/ext4");
    }
    let avail = st.blocks_available() * st.block_size() as u64;
    if avail < NODE_TMP_FLOOR_BYTES {
        bail!(
            "{NODE_TMP} has {} GiB free, floor is {} GiB",
            avail >> 30,
            NODE_TMP_FLOOR_BYTES >> 30
        );
    }
    if !ns::supports_user_xattr(&ws.base)? {
        bail!("{NODE_TMP} does not support user xattrs; overlay upper would silently degrade");
    }
    ns::usable_resolv_conf()?;
    let (bootstrap, version) = site::resolve_bootstrap()?;
    for required in ["env", "nix/store", "nix/var/nix/db/db.sqlite"] {
        if fs::symlink_metadata(bootstrap.join(required)).is_err() {
            bail!("bootstrap store {} lacks {required}", bootstrap.display());
        }
    }
    if !Path::new(CACHE_DIR).is_dir() {
        bail!("cache dir {CACHE_DIR} missing");
    }
    if !ci.cache_key().is_file() || !ci.cache_pub().is_file() {
        bail!(
            "cache signing key pair missing under {}",
            ci.root().display()
        );
    }
    reap_stale_workspaces(reap_grace);
    Ok(Gate { bootstrap, version })
}

pub fn main(run: RunId) -> Result<i32> {
    let ci = CiDir::from_env()?;
    let dir = RunDir::open(&ci.runs_root(), &run)?;
    let job = dir.job()?;
    let token = dir.take_token()?;
    let node = env_or("SLURMD_NODENAME", "unknown");
    let slurm_job = std::env::var("SLURM_JOB_ID")
        .ok()
        .and_then(|s| JobId::parse(&s).ok());
    let mut report = BuildReport {
        node: node.clone(),
        exit: exit::GATE_FAILED,
        ..Default::default()
    };

    // Opening the callback is the reachability half of the gate: a filtered
    // node fails here instead of building for an hour into a dead socket.
    let (host, port) = callback::split_host_port(&job.submit.callback_host);
    let client = match callback::connect(host, port, &job.submit.fingerprint) {
        Ok(s) => Client::new(s),
        Err(e) => {
            eprintln!(
                "gate: callback to {} unreachable: {e:#}",
                job.submit.callback_host
            );
            report.write(dir.path())?;
            return Ok(exit::GATE_FAILED);
        }
    };
    client.send(&Msg::Hello(Hello {
        run: run.clone(),
        token,
        job: slurm_job,
        node,
    }));
    {
        let client = client.clone();
        thread::spawn(move || {
            while {
                thread::sleep(HEARTBEAT);
                client.send(&Msg::Heartbeat)
            } {}
        });
    }
    install_term_forwarding()?;
    let sink = Sink {
        client: Some(client.clone()),
    };

    let code = match attempt(&ci, &job, &sink, &mut report) {
        Ok(code) => code,
        Err(e) => {
            sink.line(format!("slurm-ci build: {e:#}"));
            1
        }
    };
    report.exit = code;
    if let Err(e) = report.write(dir.path()) {
        sink.line(format!("build.json: {e:#}"));
    }
    client.send(&Msg::Exit(code));
    client.close();
    Ok(code)
}

fn attempt(ci: &CiDir, job: &JobFile, sink: &Sink, report: &mut BuildReport) -> Result<i32> {
    let s = &job.submit;
    sink.line(format!(
        "slurm-ci build: {}@{} ({}) -> {:?}",
        s.repo, s.commit, s.git_ref, s.spec.installables
    ));
    let ws = Workspace::create(s.run.as_str())?;
    let result = attempt_in(ci, job, sink, report, &ws);
    report.upper_bytes = ns::dir_size(&ws.dir("upper"));
    report.src_bytes = ns::dir_size(&ws.dir("src"));
    report.xtmp_bytes = ns::dir_size(&ws.dir("xtmp"));
    ws.remove();
    result
}

fn attempt_in(
    ci: &CiDir,
    job: &JobFile,
    sink: &Sink,
    report: &mut BuildReport,
    ws: &Workspace,
) -> Result<i32> {
    let s = &job.submit;
    let gate = match gate(ci, ws, Duration::from_secs(job.reap_grace_secs)) {
        Ok(g) => g,
        Err(e) => {
            sink.line(format!("gate: {e:#}"));
            return Ok(exit::GATE_FAILED);
        }
    };
    report.bootstrap_version = gate.version.clone();
    sink.line(format!(
        "gate ok: bootstrap v{} at {}",
        gate.version,
        gate.bootstrap.display()
    ));

    let t = Instant::now();
    let mut fetch = self_exe();
    fetch
        .arg("__ns")
        .arg("__fetch")
        .arg("--root")
        .arg(ws.dir("root"))
        .arg("--bootstrap")
        .arg(&gate.bootstrap)
        .arg("--src")
        .arg(ws.dir("src"))
        .arg("--url")
        .arg(format!("https://{}/{}.git", s.git_host, s.repo))
        .arg("--git-ref")
        .arg(&s.git_ref)
        .arg("--commit")
        .arg(&s.commit)
        .env_clear();
    let code = run_relayed(fetch, sink)?;
    report.fetch_secs = t.elapsed().as_secs();
    if code != 0 {
        sink.line(format!("fetch failed ({code})"));
        return Ok(code);
    }

    let stats_file = ws.base.join("stats.json");
    let mut build = self_exe();
    build
        .arg("__ns")
        .arg("__build")
        .arg("--root")
        .arg(ws.dir("root"))
        .arg("--bootstrap")
        .arg(&gate.bootstrap)
        .arg("--upper")
        .arg(ws.dir("upper"))
        .arg("--work")
        .arg(ws.dir("work"))
        .arg("--src")
        .arg(ws.dir("src"))
        .arg("--xtmp")
        .arg(ws.dir("xtmp"))
        .arg("--cache")
        .arg(CACHE_DIR)
        .arg("--key")
        .arg(ci.cache_key())
        .arg("--pubkey")
        .arg(fs::read_to_string(ci.cache_pub())?.trim())
        .arg("--cores")
        .arg(s.spec.cores.to_string())
        .arg("--jobs")
        .arg(s.spec.jobs.to_string())
        .arg("--tmpdir")
        .arg(s.spec.tmpdir.as_str())
        .arg("--stats-file")
        .arg(&stats_file)
        .arg("--")
        .args(&s.spec.installables)
        .env_clear();
    let code = run_relayed(build, sink)?;
    if let Some(stats) = fs::read(&stats_file)
        .ok()
        .and_then(|b| serde_json::from_slice::<BuildStats>(&b).ok())
    {
        report.build_secs = stats.build_secs;
        report.push_secs = stats.push_secs;
        report.pushed_paths = stats.pushed_paths;
    }
    sink.line(format!("build exited {code}"));
    Ok(code)
}
