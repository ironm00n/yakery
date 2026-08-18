//! `slurm-ci` — drive heavy Nix builds from a Forgejo Actions runner onto the
//! Explorer Slurm cluster, gently. One static binary, three roles:
//!
//!   * `run`      — runner side (VPS): read the repo's `.slurm-ci.toml` over the
//!                  Netbird mesh, submit one Slurm job, then *listen* for the
//!                  compute node to ping back its result. No polling of the queue;
//!                  the login node sees one `sbatch` and (at most) one log fetch.
//!                  A SIGTERM (Forgejo cancel/timeout) `scancel`s the job so it
//!                  never orphans.
//!   * `dispatch` — cluster login: the SSH forced-command entry point. Parses the
//!                  untrusted `$SSH_ORIGINAL_COMMAND` as argv (never a shell) and
//!                  reads the ephemeral Forgejo token from *stdin* — never argv,
//!                  so it can't leak into `ps` on the shared login node.
//!   * `build`    — compute node: `nix build`s each installable straight from
//!                  `git+https`, in the hardened nixenv, then pings the runner.
//!
//! The Forgejo token is the per-run, repo-scoped job token (Forgejo's OIDC-shaped
//! `${{ github.token }}` equivalent): ephemeral, never stored, used only to fetch
//! source. Connection + nixenv details come from the environment, so the binary is
//! host-agnostic (VM today, dorm Pi later).

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;

const LOG_DIR: &str = "/projects/dbp/ci-logs";
const SECRETS_DIR: &str = "/projects/dbp/ci-secrets";
const CONFIG_PATH: &str = ".slurm-ci.toml";
const TICK: Duration = Duration::from_secs(1);
/// Sparse fallback: only matters if a callback is lost (dead node), so it's gentle.
const STATUS_PROBE: Duration = Duration::from_secs(300);
const OVERALL_DEADLINE: Duration = Duration::from_secs(6 * 3600);
const LOG_KEEP: Duration = Duration::from_secs(14 * 24 * 3600);

#[derive(Parser)]
#[command(name = "slurm-ci", about = "Explorer Slurm CI dispatcher")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Runner side (VPS): read `.slurm-ci.toml`, submit, await the callback.
    Run,
    /// Cluster login: SSH forced-command entry. Reads `$SSH_ORIGINAL_COMMAND` + stdin.
    Dispatch,
    /// Compute node: the build `sbatch` runs (via the `slurm-ci-job` wrapper).
    Build {
        repo: String,
        git_ref: String,
        commit: String,
        callback: String,
        cbtoken: String,
        netrc: String,
        git_host: String,
        /// Repo-flake fragments to build (everything after `--`).
        #[arg(last = true)]
        installables: Vec<String>,
    },
}

/// The repo's `.slurm-ci.toml`: what to build, and how much cluster to ask for.
/// `installables` are fragments of the repo's *own* flake — the `git+https://…`
/// flake ref is implied — so `"packages.x86_64-linux.default"`, not `".#…"`.
#[derive(Deserialize)]
struct Config {
    installables: Vec<String>,
    #[serde(default = "default_cores")]
    cores: u32,
    #[serde(default = "default_mem")]
    mem: String,
    #[serde(default = "default_time")]
    time: String,
}
fn default_cores() -> u32 {
    8
}
fn default_mem() -> String {
    "16G".to_owned()
}
fn default_time() -> String {
    "00:45:00".to_owned()
}

fn main() -> Result<()> {
    let code = match Cli::parse().cmd {
        Cmd::Run => run()?,
        Cmd::Dispatch => dispatch()?,
        Cmd::Build {
            repo,
            git_ref,
            commit,
            callback,
            cbtoken,
            netrc,
            git_host,
            installables,
        } => build(
            &repo,
            &git_ref,
            &commit,
            &callback,
            &cbtoken,
            &netrc,
            &git_host,
            &installables,
        )?,
    };
    std::io::stdout().flush().ok();
    std::process::exit(code);
}

// ---- runner side (VPS) -----------------------------------------------------

fn run() -> Result<i32> {
    let repo = env_var("CI_REPO")?;
    let git_ref = env_var("CI_REF")?;
    let commit = env_var("CI_COMMIT")?;
    let token = env_var("CI_TOKEN")?; // ephemeral, repo-scoped job token
    let api = env_var("CI_FORGEJO_API")?; // mesh base (no Anubis, plain HTTP)
    let git_host = env_var("CI_FORGEJO_GIT_HOST")?; // public host the cluster fetches from
    let callback_host = env_var("CI_CALLBACK_HOST")?;

    let config = fetch_config(&api, &repo, &commit, &token).context("read .slurm-ci.toml")?;
    for inst in &config.installables {
        check(inst, is_installable_char, "installable")?;
        if inst.starts_with('-') {
            bail!("installable may not start with '-': {inst:?}");
        }
    }

    let listener = TcpListener::bind("0.0.0.0:0").context("bind callback listener")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let cbtoken = random_token()?;
    let callback = format!("{callback_host}:{port}");

    let submit = format!(
        "submit {repo} {git_ref} {commit} {callback} {cbtoken} {cores} {mem} {time} {git_host} -- {insts}",
        cores = config.cores,
        mem = config.mem,
        time = config.time,
        insts = config.installables.join(" "),
    );
    let job_id = submit_with_retry(&submit, &token)?;
    eprintln!("submitted Slurm job {job_id} (callback {callback})");

    // Forgejo cancel/timeout SIGTERMs us; never leave a job running unwatched.
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, term.clone())?;

    let start = Instant::now();
    let mut last_probe = Instant::now();
    loop {
        if term.load(Ordering::Relaxed) {
            eprintln!("cancelled — scancel {job_id}");
            let _ = ssh(&format!("cancel {job_id}"));
            return Ok(1);
        }

        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(code) = read_callback(stream, &cbtoken) {
                    let _ = ssh(&format!("log {job_id}")).map(|l| print!("{l}"));
                    return Ok(code);
                }
                // bad token: ignore and keep waiting
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e).context("callback listener"),
        }

        if last_probe.elapsed() >= STATUS_PROBE {
            last_probe = Instant::now();
            if let Ok((state, code)) =
                ssh(&format!("status {job_id}")).and_then(|l| parse_status(&l))
            {
                match classify(&state) {
                    Progress::Pending => {}
                    Progress::Succeeded => {
                        let _ = ssh(&format!("log {job_id}")).map(|l| print!("{l}"));
                        return Ok(if code == "0:0" { 0 } else { 1 });
                    }
                    Progress::Failed => {
                        let _ = ssh(&format!("log {job_id}")).map(|l| print!("{l}"));
                        eprintln!("Slurm job {job_id} ended: {state} ({code})");
                        return Ok(1);
                    }
                }
            }
        }

        if start.elapsed() >= OVERALL_DEADLINE {
            let _ = ssh(&format!("cancel {job_id}"));
            bail!("job {job_id} exceeded {OVERALL_DEADLINE:?} without completing");
        }
        sleep(TICK);
    }
}

/// Fetch `.slurm-ci.toml` from Forgejo's raw API over the mesh. The token rides a
/// `curl -K -` config on stdin so it never lands in argv / `ps`.
fn fetch_config(api: &str, repo: &str, commit: &str, token: &str) -> Result<Config> {
    let url = format!("{api}/api/v1/repos/{repo}/raw/{CONFIG_PATH}?ref={commit}");
    let curlrc = format!("header = \"Authorization: token {token}\"\n");
    let body = capture_stdin("curl", &["-sfS", "-K", "-", &url], Some(&curlrc))?;
    let config: Config = toml::from_str(&body).context("parse .slurm-ci.toml")?;
    if config.installables.is_empty() {
        bail!("`.slurm-ci.toml` lists no installables");
    }
    check(&config.mem, is_mem_char, "mem")?;
    check(&config.time, is_time_char, "time")?;
    Ok(config)
}

fn submit_with_retry(cmd: &str, token: &str) -> Result<String> {
    for attempt in 1..=3u64 {
        match ssh_stdin(cmd, Some(token)) {
            Ok(out) if !out.trim().is_empty() => return Ok(out.trim().to_owned()),
            Ok(_) if attempt == 3 => break,
            Ok(_) => {}
            Err(e) if attempt < 3 => eprintln!("submit attempt {attempt} failed: {e:#}"),
            Err(e) => return Err(e).context("submit failed"),
        }
        sleep(Duration::from_secs(attempt * 10));
    }
    bail!("submit produced no job id");
}

fn read_callback(mut stream: TcpStream, expected: &str) -> Option<i32> {
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    let mut parts = buf.split_whitespace();
    let token = parts.next()?;
    let code: i32 = parts.next()?.parse().ok()?;
    (token == expected).then_some(code)
}

fn random_token() -> Result<String> {
    let mut buf = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

enum Progress {
    Pending,
    Succeeded,
    Failed,
}

/// `sacct` can suffix states ("CANCELLED by 42", "COMPLETING"); classify on the
/// leading word and treat anything not explicitly live/done as a failure.
fn classify(state: &str) -> Progress {
    match state.split_whitespace().next().unwrap_or("") {
        "COMPLETED" => Progress::Succeeded,
        "PENDING" | "RUNNING" | "REQUEUED" | "RESIZING" | "SUSPENDED" | "COMPLETING"
        | "CONFIGURING" => Progress::Pending,
        _ => Progress::Failed,
    }
}

fn parse_status(line: &str) -> Result<(String, String)> {
    let line = line.trim();
    let (state, code) = line
        .split_once('|')
        .with_context(|| format!("unparseable sacct line {line:?}"))?;
    Ok((state.trim().to_owned(), code.trim().to_owned()))
}

// ---- cluster login (forced command) ----------------------------------------

/// The authorized_keys `command=` forces this; the client's requested command
/// arrives in `$SSH_ORIGINAL_COMMAND` (untrusted) and the token on stdin.
fn dispatch() -> Result<i32> {
    let orig = env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default();
    let parts: Vec<&str> = orig.split_whitespace().collect();
    match parts.split_first() {
        Some((&"submit", rest)) => {
            let sep = rest
                .iter()
                .position(|&t| t == "--")
                .context("submit: missing `--` before installables")?;
            let (fixed, tail) = rest.split_at(sep);
            let installables = &tail[1..];
            if fixed.len() != 9 {
                bail!("submit: expected 9 fixed args, got {}", fixed.len());
            }
            let token = read_stdin_token()?;
            submit(
                fixed[0], fixed[1], fixed[2], fixed[3], fixed[4], fixed[5], fixed[6], fixed[7],
                fixed[8], installables, &token,
            )
        }
        Some((&"status", [job])) => status(job),
        Some((&"log", [job])) => log(job),
        Some((&"cancel", [job])) => cancel(job),
        _ => bail!("rejected command: {orig:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn submit(
    repo: &str,
    git_ref: &str,
    commit: &str,
    callback: &str,
    cbtoken: &str,
    cores: &str,
    mem: &str,
    time: &str,
    git_host: &str,
    installables: &[&str],
    token: &str,
) -> Result<i32> {
    check(repo, is_path_char, "repo")?;
    check(git_ref, is_path_char, "ref")?;
    check_commit(commit)?;
    check(callback, is_endpoint_char, "callback")?;
    check(cbtoken, |c| c.is_ascii_hexdigit(), "cbtoken")?;
    check(cores, |c| c.is_ascii_digit(), "cores")?;
    check(mem, is_mem_char, "mem")?;
    check(time, is_time_char, "time")?;
    check(git_host, is_host_char, "git_host")?;
    if installables.is_empty() {
        bail!("submit: no installables");
    }
    for inst in installables {
        check(inst, is_installable_char, "installable")?;
        if inst.starts_with('-') {
            bail!("installable may not start with '-': {inst:?}");
        }
    }

    std::fs::create_dir_all(LOG_DIR).ok();
    prune_logs();
    let owner = repo.split('/').next().unwrap_or(repo);
    let netrc = write_netrc(git_host, owner, token)?;
    let batch = batch_script()?;

    let mut args: Vec<String> = vec![
        "--parsable".into(),
        "--job-name=slurm-ci".into(),
        format!("--time={time}"),
        format!("--cpus-per-task={cores}"),
        format!("--mem={mem}"),
        format!("--output={LOG_DIR}/%j.out"),
        batch,
        repo.into(),
        git_ref.into(),
        commit.into(),
        callback.into(),
        cbtoken.into(),
        netrc,
        git_host.into(),
        "--".into(),
    ];
    args.extend(installables.iter().map(|s| (*s).to_owned()));
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();

    let job_id = capture("sbatch", &argv)?;
    print!("{}", job_id.trim());
    Ok(0)
}

/// Write the ephemeral token into a 0600 netrc the build job points Nix at.
fn write_netrc(git_host: &str, owner: &str, token: &str) -> Result<String> {
    std::fs::create_dir_all(SECRETS_DIR).ok();
    let _ = std::fs::set_permissions(SECRETS_DIR, std::fs::Permissions::from_mode(0o700));
    let path = format!("{SECRETS_DIR}/{}.netrc", random_token()?);
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("create {path}"))?;
    // Forgejo accepts the per-run token as the HTTP password. `login` is the repo
    // owner; if your instance wants a different username (e.g. `x-access-token`),
    // change it here — trivially verifiable with a manual `git clone`.
    writeln!(f, "machine {git_host} login {owner} password {token}")?;
    Ok(path)
}

fn batch_script() -> Result<String> {
    match env::var("CI_BATCH_SCRIPT") {
        Ok(p) => Ok(p),
        Err(_) => Ok(format!("{}/bin/slurm-ci-job", env_var("HOME")?)),
    }
}

fn status(job: &str) -> Result<i32> {
    check(job, |c| c.is_ascii_digit(), "job id")?;
    let out = capture("sacct", &["-j", job, "-X", "-n", "-P", "-o", "State,ExitCode"])?;
    print!("{}", out.lines().next().unwrap_or("").trim());
    Ok(0)
}

fn log(job: &str) -> Result<i32> {
    check(job, |c| c.is_ascii_digit(), "job id")?;
    let data = std::fs::read(format!("{LOG_DIR}/{job}.out")).unwrap_or_default();
    let tail = &data[data.len().saturating_sub(1 << 20)..]; // last 1 MiB
    std::io::stdout().write_all(tail).ok();
    Ok(0)
}

fn cancel(job: &str) -> Result<i32> {
    check(job, |c| c.is_ascii_digit(), "job id")?;
    capture("scancel", &[job])?;
    Ok(0)
}

/// Keep `/projects` tidy: the build host's job, but cheap to do on submit.
fn prune_logs() {
    let Some(cutoff) = SystemTime::now().checked_sub(LOG_KEEP) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(LOG_DIR) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("out") {
            continue;
        }
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            if modified < cutoff {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

// ---- compute node ----------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build(
    repo: &str,
    git_ref: &str,
    commit: &str,
    callback: &str,
    cbtoken: &str,
    netrc: &str,
    git_host: &str,
    installables: &[String],
) -> Result<i32> {
    // stdout/stderr are inherited -> captured by sbatch's --output file, which the
    // runner fetches via the `log` verb. We only ship the exit code back.
    eprintln!("slurm-ci build: {repo}@{commit} (ref {git_ref}) -> {installables:?}");
    let np = env_var("CI_NIX_PORTABLE")?;
    let mut code = 0;
    for inst in installables {
        let url = format!("git+https://{git_host}/{repo}?ref={git_ref}&rev={commit}#{inst}");
        eprintln!("== nix build {url}");
        let status = Command::new(&np)
            .args([
                "nix",
                "build",
                "--option",
                "netrc-file",
                netrc,
                "--option",
                "sandbox",
                "true",
                "--option",
                "sandbox-fallback",
                "false",
                "--option",
                "accept-flake-config",
                "false",
                "--option",
                "require-sigs",
                "true",
                "--option",
                "substituters",
                "https://cache.nixos.org",
                "--no-link",
                "--print-build-logs",
                &url,
            ])
            .status();
        let c = status.ok().and_then(|s| s.code()).unwrap_or(1);
        if c != 0 {
            code = c;
            break;
        }
    }
    let _ = std::fs::remove_file(netrc); // burn the token netrc, success or fail
    if let Err(e) = send_callback(callback, cbtoken, code) {
        // Best-effort: the runner's status probe is the backstop.
        eprintln!("callback to {callback} failed: {e:#}");
    }
    Ok(code)
}

fn send_callback(callback: &str, cbtoken: &str, code: i32) -> Result<()> {
    let mut stream = TcpStream::connect(callback).context("connect callback")?;
    write!(stream, "{cbtoken} {code}")?;
    Ok(())
}

// ---- helpers ---------------------------------------------------------------

/// SSH to the cluster with a fixed, hardened option set. With the callback model
/// this fires rarely (submit + an occasional probe / log fetch).
fn ssh(remote: &str) -> Result<String> {
    ssh_stdin(remote, None)
}

fn ssh_stdin(remote: &str, stdin: Option<&str>) -> Result<String> {
    let user = env_var("CI_CLUSTER_USER")?;
    let host = env_var("CI_CLUSTER_HOST")?;
    let key = env_var("CI_SSH_KEY")?;
    let known = env_var("CI_KNOWN_HOSTS")?;
    capture_stdin(
        "ssh",
        &[
            "-i",
            &key,
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            &format!("UserKnownHostsFile={known}"),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=20",
            &format!("{user}@{host}"),
            remote,
        ],
        stdin,
    )
}

fn capture(program: &str, args: &[&str]) -> Result<String> {
    capture_stdin(program, args, None)
}

/// Spawn `program`, optionally feed `stdin` (small payloads only — we write it all
/// before draining stdout, so a large stdin could deadlock; ours are tiny), and
/// return stdout on success.
fn capture_stdin(program: &str, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {program}"))?;
    if let Some(data) = stdin {
        child
            .stdin
            .take()
            .expect("stdin piped")
            .write_all(data.as_bytes())
            .context("write child stdin")?;
    }
    let out = child
        .wait_with_output()
        .with_context(|| format!("wait for {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8(out.stdout).context("non-utf8 output")
}

fn read_stdin_token() -> Result<String> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .context("read token from stdin")?;
    let s = s.trim().to_owned();
    if s.is_empty() {
        bail!("empty token on stdin");
    }
    Ok(s)
}

fn env_var(key: &str) -> Result<String> {
    env::var(key).with_context(|| format!("{key} unset"))
}

fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._/-".contains(c)
}

fn is_endpoint_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || ".:-".contains(c)
}

fn is_host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || ".-".contains(c)
}

fn is_mem_char(c: char) -> bool {
    c.is_ascii_digit() || "KMGTkmgt".contains(c)
}

fn is_time_char(c: char) -> bool {
    c.is_ascii_digit() || ":-".contains(c)
}

/// Flake fragment: a dotted attr path like `packages.x86_64-linux.default`.
fn is_installable_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._-".contains(c)
}

fn check(s: &str, pred: impl Fn(char) -> bool, what: &str) -> Result<()> {
    if !s.is_empty() && s.chars().all(pred) {
        Ok(())
    } else {
        bail!("bad {what}: {s:?}");
    }
}

fn check_commit(s: &str) -> Result<()> {
    if (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        bail!("bad commit: {s:?}");
    }
}
