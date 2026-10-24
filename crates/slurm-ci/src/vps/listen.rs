//! The VPS daemon: owns port 443, the pinned cert, the SSH key, the registry,
//! and the JSONL. N `run` clients attach over a unix socket; each in-flight
//! attempt is one driver thread that waits on the callback and falls back to
//! backed-off `status` probes (§2, §3).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::callback::{self, Hello, Identity, Msg, HEARTBEAT};
use crate::frame::Frame;
use crate::ids::{RunId, Token};
use crate::proc::env_or;
use crate::report::{mem_overshoot, JsonlRecord};
use crate::sacct::SacctRecord;
use crate::verbs::{check_commit, check_ref, check_repo, JobId, Submit};
use crate::verdict::{exit, Verdict};
use crate::vps::cluster::Cluster;
use crate::vps::forge::Forge;
use crate::vps::ipc::{Task, ToDaemon, ToRun, DEFAULT_SOCKET};
use crate::vps::registry::{now, RunRecord, StateDir};

pub const DEFAULT_STATE_DIR: &str = "/var/lib/slurm-ci";
const PROBE_MAX: Duration = Duration::from_secs(30 * 60);
/// `caps::QUEUE_DEADLINE` plus slack: a run that never learns its job id
/// concludes inconclusive after this.
const NO_JOB_WALL: Duration = Duration::from_secs(13 * 3600);
const TASK_TIMEOUT: Duration = Duration::from_secs(30);

pub enum Event {
    Hello { job: Option<JobId>, node: String },
    Data(Vec<u8>),
    Heartbeat,
    Exit(i32),
    CallbackClosed,
    Cancel,
    ClientGone,
}

struct Registered {
    token: Token,
    claimed: bool,
    tx: Sender<Event>,
}

pub struct Daemon {
    state: StateDir,
    /// First status probe after this; doubles up to `PROBE_MAX` (§3).
    probe_min: Duration,
    cluster: Cluster,
    forge: Forge,
    fingerprint: String,
    callback_host: String,
    repo_allowlist: Option<Vec<String>>,
    runs: Mutex<HashMap<RunId, Registered>>,
}

/// Writer half of a `run` client's socket; a failed write detaches the client
/// and the driver carries on to finalize without it.
struct Client {
    stream: Mutex<Option<UnixStream>>,
}

impl Client {
    fn send(&self, msg: &ToRun) {
        let mut guard = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = guard.as_mut() {
            if msg.to_frame().write_to(s).is_err() {
                *guard = None;
            }
        }
    }
    fn attached(&self) -> bool {
        self.stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

pub fn main() -> Result<()> {
    let state = StateDir::open(env_or("CI_STATE_DIR", DEFAULT_STATE_DIR))?;
    let identity = Identity::load_or_create(&state.tls_dir())?;
    let tls = identity.server_config()?;
    let daemon = Arc::new(Daemon {
        state,
        probe_min: Duration::from_secs(
            env_or("CI_PROBE_MIN_SECS", "60")
                .parse()
                .context("CI_PROBE_MIN_SECS")?,
        ),
        cluster: Cluster::from_env()?,
        forge: Forge::from_env()?,
        fingerprint: identity.fingerprint.clone(),
        callback_host: crate::proc::env_var("CI_CALLBACK_HOST")?,
        repo_allowlist: std::env::var("CI_REPO_ALLOWLIST").ok().map(|s| {
            s.split(',')
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty())
                .collect()
        }),
        runs: Mutex::new(HashMap::new()),
    });

    let bind = env_or("CI_CALLBACK_BIND", &format!("0.0.0.0:{}", callback::PORT));
    let listener =
        TcpListener::bind(&bind).with_context(|| format!("bind callback listener {bind}"))?;
    eprintln!("callback listener on {bind}, cert {}", identity.fingerprint);
    {
        let daemon = daemon.clone();
        thread::spawn(move || serve_callbacks(daemon, listener, tls));
    }

    for rec in daemon.state.load_all()? {
        eprintln!(
            "reconcile: re-attaching run {} (job {:?})",
            rec.run, rec.job
        );
        let daemon = daemon.clone();
        thread::spawn(move || drive(&daemon, rec, None));
    }

    let socket = env_or("CI_SOCKET", DEFAULT_SOCKET);
    let _ = fs::remove_file(&socket);
    let unix = UnixListener::bind(&socket).with_context(|| format!("bind {socket}"))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o660))?;
    eprintln!("accepting runs on {socket}");
    for stream in unix.incoming() {
        match stream {
            Ok(stream) => {
                let daemon = daemon.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_client(&daemon, stream) {
                        eprintln!("client: {e:#}");
                    }
                });
            }
            Err(e) => eprintln!("unix accept: {e}"),
        }
    }
    Ok(())
}

fn serve_callbacks(daemon: Arc<Daemon>, listener: TcpListener, tls: Arc<rustls::ServerConfig>) {
    for tcp in listener.incoming() {
        let Ok(tcp) = tcp else { continue };
        let (daemon, tls) = (daemon.clone(), tls.clone());
        thread::spawn(move || match callback::accept(tls, tcp) {
            Ok((hello, stream)) => daemon.attach(hello, stream),
            Err(e) => eprintln!("callback rejected: {e:#}"),
        });
    }
}

impl Daemon {
    /// First valid `HELLO` claims the run and retires its token.
    fn attach(&self, hello: Hello, mut stream: callback::ServerStream) {
        let tx = {
            let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            let Some(r) = runs.get_mut(&hello.run) else {
                eprintln!("callback for unknown run {}", hello.run);
                return;
            };
            if r.claimed || !r.token.matches(&hello.token) {
                eprintln!(
                    "callback for run {} refused (claimed={})",
                    hello.run, r.claimed
                );
                return;
            }
            r.claimed = true;
            r.tx.clone()
        };
        let _ = tx.send(Event::Hello {
            job: hello.job,
            node: hello.node,
        });
        loop {
            match callback::read_next(&mut stream) {
                Ok(Some(Msg::Data(d))) => {
                    let _ = tx.send(Event::Data(d));
                }
                Ok(Some(Msg::Heartbeat)) => {
                    let _ = tx.send(Event::Heartbeat);
                }
                Ok(Some(Msg::Exit(code))) => {
                    let _ = tx.send(Event::Exit(code));
                    return;
                }
                Ok(Some(Msg::Hello(_))) | Ok(None) | Err(_) => {
                    let _ = tx.send(Event::CallbackClosed);
                    return;
                }
            }
        }
    }

    fn register(&self, rec: &RunRecord, tx: Sender<Event>) {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        runs.insert(
            rec.run.clone(),
            Registered {
                token: rec.token.clone(),
                claimed: rec.claimed,
                tx,
            },
        );
    }

    fn unregister(&self, run: &RunId) {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(run);
    }
}

fn handle_client(daemon: &Daemon, stream: UnixStream) -> Result<()> {
    stream.set_read_timeout(Some(TASK_TIMEOUT))?;
    let mut reader = stream.try_clone()?;
    let client = Arc::new(Client {
        stream: Mutex::new(Some(stream)),
    });
    let task = match Frame::read_from(&mut reader)?
        .map(ToDaemon::from_frame)
        .transpose()?
    {
        Some(ToDaemon::Task(t)) => t,
        other => bail!("expected TASK, got {other:?}"),
    };
    reader.set_read_timeout(None)?;

    let rec = match daemon.admit(&task) {
        Ok(rec) => rec,
        Err(e) => {
            client.send(&ToRun::Exit {
                status: 1,
                verdict: format!("refused: {e:#}"),
            });
            return Err(e);
        }
    };

    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        thread::spawn(move || loop {
            match Frame::read_from(&mut reader).map(|f| f.map(ToDaemon::from_frame)) {
                Ok(Some(Ok(ToDaemon::Cancel))) => {
                    let _ = tx.send(Event::Cancel);
                }
                Ok(Some(Ok(ToDaemon::Task(_)))) => {}
                Ok(Some(Err(_))) | Ok(None) | Err(_) => {
                    let _ = tx.send(Event::ClientGone);
                    return;
                }
            }
        });
    }
    drive_with(daemon, rec, Some(client), tx, rx);
    Ok(())
}

impl Daemon {
    /// Trusted refs only (§4): `push` events on repos we serve.
    fn admit(&self, task: &Task) -> Result<RunRecord> {
        if task.event != "push" {
            bail!("event {:?} is not push", task.event);
        }
        check_repo(&task.repo)?;
        check_ref(&task.git_ref)?;
        check_commit(&task.commit)?;
        if let Some(allow) = &self.repo_allowlist {
            if !allow.iter().any(|r| r == &task.repo) {
                bail!("repo {} not in CI_REPO_ALLOWLIST", task.repo);
            }
        }
        let spec = self.forge.job_spec(&task.repo, &task.commit, &task.token)?;
        Ok(RunRecord {
            run: RunId::random()?,
            attempt: 1,
            repo: task.repo.clone(),
            git_ref: task.git_ref.clone(),
            commit: task.commit.clone(),
            spec,
            token: Token::random()?,
            claimed: false,
            job: None,
            node: None,
            submitted_at: 0,
            cancelled: false,
            verdict: None,
        })
    }

    fn submit_spec(&self, rec: &RunRecord, exclude: Option<String>) -> Submit {
        Submit {
            run: rec.run.clone(),
            repo: rec.repo.clone(),
            git_host: self.forge.git_host.clone(),
            git_ref: rec.git_ref.clone(),
            commit: rec.commit.clone(),
            callback_host: self.callback_host.clone(),
            fingerprint: self.fingerprint.clone(),
            spec: rec.spec.clone(),
            exclude,
        }
    }
}

fn drive(daemon: &Daemon, rec: RunRecord, client: Option<Arc<Client>>) {
    let (tx, rx) = mpsc::channel();
    drive_with(daemon, rec, client, tx, rx);
}

enum Outcome {
    Exit(i32),
    Sacct(Box<SacctRecord>),
    NoJob,
}

/// One task from submit to JSONL, across gate retries. Owns the event channel
/// for the client's lifetime.
fn drive_with(
    daemon: &Daemon,
    mut rec: RunRecord,
    client: Option<Arc<Client>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
) {
    let client = client.as_deref();
    let mut exclude = None;
    loop {
        daemon.register(&rec, tx.clone());
        if let Some((verdict, via)) = rec.verdict.clone() {
            finalize(daemon, &rec, verdict, &via, true, client);
            daemon.unregister(&rec.run);
            return;
        }
        if rec.submitted_at == 0 {
            rec.submitted_at = now();
            if let Err(e) = daemon.state.save(&rec) {
                say(client, &rec.run, format!("registry write failed: {e:#}"));
            }
            match daemon
                .cluster
                .submit(&daemon.submit_spec(&rec, exclude.take()), &rec.token)
            {
                Ok(job) => {
                    say(
                        client,
                        &rec.run,
                        format!("submitted Slurm job {job} (attempt {})", rec.attempt),
                    );
                    rec.job = Some(job);
                    let _ = daemon.state.save(&rec);
                }
                Err(e) => {
                    let verdict = Verdict::Inconclusive(format!("submit failed: {e:#}"));
                    finalize(daemon, &rec, verdict, "submit", true, client);
                    daemon.unregister(&rec.run);
                    return;
                }
            }
        }

        let outcome = wait(daemon, &mut rec, client, &rx);
        daemon.unregister(&rec.run);
        let (verdict, via) = match &outcome {
            Outcome::Exit(code) => (Verdict::from_exit(*code), "callback"),
            Outcome::Sacct(s) => (
                Verdict::from_sacct(s)
                    .unwrap_or_else(|| Verdict::Inconclusive("sacct not terminal".into())),
                "sacct",
            ),
            Outcome::NoJob => (
                Verdict::Inconclusive("no job id learned before the queue deadline".into()),
                "deadline",
            ),
        };
        let gate_refused = match &outcome {
            Outcome::Exit(c) => *c == exit::GATE_FAILED,
            Outcome::Sacct(s) => {
                s.state == "FAILED"
                    && s.wrapper_exit()
                        .is_some_and(|e| e.code == exit::GATE_FAILED && e.signal == 0)
            }
            Outcome::NoJob => false,
        };
        let node = rec.node.clone().or_else(|| match &outcome {
            Outcome::Sacct(s) => s.first_node().map(str::to_owned),
            _ => None,
        });
        if gate_refused && rec.attempt == 1 && !rec.cancelled {
            if let Some(node) = node {
                say(
                    client,
                    &rec.run,
                    format!(
                        "node {node} failed the sanity gate; resubmitting with --exclude={node}"
                    ),
                );
                finalize(
                    daemon,
                    &rec,
                    Verdict::Red(format!("gate refused node {node}")),
                    via,
                    false,
                    client,
                );
                rec = RunRecord {
                    run: RunId::random().expect("urandom"),
                    attempt: 2,
                    token: Token::random().expect("urandom"),
                    claimed: false,
                    job: None,
                    node: None,
                    submitted_at: 0,
                    verdict: None,
                    ..rec
                };
                exclude = Some(node);
                continue;
            }
        }
        finalize(daemon, &rec, verdict, via, true, client);
        return;
    }
}

fn say(client: Option<&Client>, run: &RunId, s: String) {
    eprintln!("run {run}: {s}");
    if let Some(c) = client {
        c.send(&ToRun::Info(s));
    }
}

/// Block until the attempt concludes: an `EXIT` over the callback, a terminal
/// sacct state from a backed-off probe, or the queue deadline with no job id.
fn wait(
    daemon: &Daemon,
    rec: &mut RunRecord,
    client: Option<&Client>,
    rx: &Receiver<Event>,
) -> Outcome {
    let started = Instant::now();
    let mut backoff = daemon.probe_min;
    let mut next_probe = Instant::now() + backoff;
    let mut callback_live = false;
    let mut last_heartbeat = Instant::now();
    loop {
        let now_i = Instant::now();
        let mut timeout = next_probe.saturating_duration_since(now_i);
        if callback_live {
            timeout =
                timeout.min((last_heartbeat + HEARTBEAT * 3).saturating_duration_since(now_i));
        }
        match rx.recv_timeout(timeout) {
            Ok(Event::Hello { job, node }) => {
                if rec.job.is_none() {
                    rec.job = job;
                }
                say(
                    client,
                    &rec.run,
                    format!(
                        "job {} started on {node}",
                        rec.job.as_ref().map_or("?".to_owned(), |j| j.to_string())
                    ),
                );
                rec.node = Some(node);
                rec.claimed = true;
                let _ = daemon.state.save(rec);
                callback_live = true;
                last_heartbeat = Instant::now();
            }
            Ok(Event::Data(d)) => {
                if let Some(c) = client {
                    c.send(&ToRun::Data(d));
                }
            }
            Ok(Event::Heartbeat) => last_heartbeat = Instant::now(),
            Ok(Event::Exit(code)) => return Outcome::Exit(code),
            Ok(Event::CallbackClosed) => {
                callback_live = false;
                next_probe = Instant::now() + Duration::from_secs(30);
            }
            Ok(Event::Cancel) => {
                if let Some(job) = &rec.job {
                    say(client, &rec.run, format!("cancel requested; scancel {job}"));
                    if let Err(e) = daemon.cluster.cancel(job, &rec.run) {
                        say(client, &rec.run, format!("scancel failed: {e:#}"));
                    }
                } else {
                    say(client, &rec.run, "cancel requested before a job id was known; the queue deadline will collect it".into());
                }
                rec.cancelled = true;
                let _ = daemon.state.save(rec);
                if let Some(c) = client {
                    c.send(&ToRun::Exit {
                        status: 1,
                        verdict: "cancelled".into(),
                    });
                }
                next_probe = Instant::now() + daemon.probe_min.min(Duration::from_secs(20));
                backoff = daemon.probe_min;
            }
            Ok(Event::ClientGone) => {}
            Err(RecvTimeoutError::Disconnected) => return Outcome::NoJob,
            Err(RecvTimeoutError::Timeout) => {
                if callback_live && last_heartbeat.elapsed() > HEARTBEAT * 3 {
                    say(client, &rec.run, "callback heartbeat lost; probing".into());
                    callback_live = false;
                    next_probe = Instant::now();
                }
                if Instant::now() < next_probe {
                    continue;
                }
                backoff = (backoff * 2).min(PROBE_MAX);
                next_probe = Instant::now() + backoff;
                let Some(job) = rec.job.clone() else {
                    if started.elapsed() > NO_JOB_WALL {
                        return Outcome::NoJob;
                    }
                    continue;
                };
                match daemon.cluster.status(&job, &rec.run) {
                    Ok(Some(s)) if s.is_terminal() => return Outcome::Sacct(Box::new(s)),
                    Ok(Some(s)) => {
                        if rec.node.is_none() {
                            rec.node = s.first_node().map(str::to_owned);
                        }
                        eprintln!("run {}: job {job} {}", rec.run, s.state);
                    }
                    Ok(None) => eprintln!("run {}: job {job} not in sacct yet", rec.run),
                    Err(e) => say(client, &rec.run, format!("status probe failed: {e:#}")),
                }
            }
        }
    }
}

fn finalize(
    daemon: &Daemon,
    rec: &RunRecord,
    verdict: Verdict,
    via: &str,
    notify: bool,
    client: Option<&Client>,
) {
    let mut rec = rec.clone();
    rec.verdict = Some((verdict.clone(), via.to_owned()));
    let _ = daemon.state.save(&rec);

    if via != "callback" {
        if let (Some(job), Some(c)) = (&rec.job, client) {
            if c.attached() {
                match daemon.cluster.log(job, &rec.run) {
                    Ok(log) => c.send(&ToRun::Data(log)),
                    Err(e) => c.send(&ToRun::Info(format!("log fetch failed: {e:#}"))),
                }
            }
        }
    }

    let eff = match &rec.job {
        Some(job) => match daemon.cluster.eff(job, &rec.run) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("run {}: eff failed: {e:#}", rec.run);
                None
            }
        },
        None => None,
    };
    let line = JsonlRecord {
        run: rec.run.clone(),
        attempt: rec.attempt,
        repo: rec.repo.clone(),
        git_ref: rec.git_ref.clone(),
        commit: rec.commit.clone(),
        job: rec.job.clone(),
        verdict: verdict.clone(),
        verdict_via: via.to_owned(),
        submitted_at: rec.submitted_at,
        finished_at: now(),
        mem_overshoot: eff.as_ref().is_some_and(|e| mem_overshoot(&e.sacct)),
        eff,
    };
    if let Err(e) = daemon.state.append_jsonl(&line) {
        eprintln!("run {}: JSONL append failed: {e:#}", rec.run);
    }
    let _ = daemon.state.remove(&rec.run);
    eprintln!("run {}: {verdict} (via {via})", rec.run);
    if notify {
        if let Some(c) = client {
            if line.mem_overshoot {
                c.send(&ToRun::Info(
                    "warning: MaxRSS exceeded ReqMem; raise `mem` in .slurm-ci.toml".into(),
                ));
            }
            c.send(&ToRun::Exit {
                status: verdict.exit_status(),
                verdict: verdict.to_string(),
            });
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(s) = self
            .stream
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = s.flush();
        }
    }
}
