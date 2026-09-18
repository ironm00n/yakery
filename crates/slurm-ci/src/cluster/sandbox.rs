//! The in-namespace roles, each a re-exec of this binary (single-threaded at
//! `unshare`, cleared environment, no inherited fds beyond stdio):
//!
//! - `__ns <role> …`: user + mount + pid namespace, then the role as pid 1 of
//!   it. A proc mount inside a userns is only permitted for a *new* pid
//!   namespace on some kernels, so every role that wants `/proc` gets its own.
//! - `__fetch`: throwaway namespace holding only the bootstrap store (ro) and
//!   `/src`; scrubbed `git` from the store fetches the pinned rev (§1).
//! - `__build`: the §5 stack — minimal rootfs, overlay `/nix`, a nested pid
//!   namespace, `nix build` as mapped uid 1000 via `__stub`, then the push
//!   phase (§7) after the untrusted child is gone.
//! - `__stub`: pid 1 of the innermost pid namespace; maps ns-root to an
//!   ordinary uid, mounts its own `/proc`, forwards SIGTERM to nix.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Args;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{fork, ForkResult, Pid};

use crate::cluster::ns::{self, Rootfs};
use crate::cluster::push;
use crate::cluster::site::CACHE_KEY_NAME;
use crate::proc::Exec;
use crate::spec::TmpDir;
use crate::verdict::exit;

pub const INNER_UID: u32 = 1000;
pub const INNER_GID: u32 = 100;

/// The bootstrap store's `env` symlink target: a `buildEnv` of nix, git,
/// cacert and friends, valid as an absolute path once `/nix` is mounted.
pub struct ToolEnv {
    pub env_dir: PathBuf,
    pub tmpdir: PathBuf,
}

impl ToolEnv {
    pub fn resolve(bootstrap: &Path, tmpdir: PathBuf) -> Result<Self> {
        let env_dir = fs::read_link(bootstrap.join("env"))
            .with_context(|| format!("readlink {}/env", bootstrap.display()))?;
        if !env_dir.starts_with("/nix/store/") {
            bail!(
                "{}/env points outside /nix/store: {}",
                bootstrap.display(),
                env_dir.display()
            );
        }
        Ok(ToolEnv { env_dir, tmpdir })
    }

    pub fn bin(&self, name: &str) -> PathBuf {
        self.env_dir.join("bin").join(name)
    }

    pub fn cacert(&self) -> PathBuf {
        self.env_dir.join("etc/ssl/certs/ca-bundle.crt")
    }

    pub fn vars(&self) -> Vec<(String, String)> {
        let home = "/tmp/home";
        let cacert = self.cacert().display().to_string();
        vec![
            (
                "PATH".into(),
                self.env_dir.join("bin").display().to_string(),
            ),
            ("HOME".into(), home.into()),
            ("USER".into(), "ci".into()),
            ("TMPDIR".into(), self.tmpdir.display().to_string()),
            ("XDG_CACHE_HOME".into(), format!("{home}/.cache")),
            ("XDG_CONFIG_HOME".into(), format!("{home}/.config")),
            ("XDG_STATE_HOME".into(), format!("{home}/.local/state")),
            ("XDG_DATA_HOME".into(), format!("{home}/.local/share")),
            ("NIX_CONF_DIR".into(), "/etc/nix".into()),
            ("NIX_SSL_CERT_FILE".into(), cacert.clone()),
            ("SSL_CERT_FILE".into(), cacert.clone()),
            ("GIT_SSL_CAINFO".into(), cacert),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_ALLOW_PROTOCOL".into(), "https".into()),
        ]
    }

    pub fn exec(&self, name: &str) -> Exec {
        let mut e = Exec::new(self.bin(name)).env_clear();
        for (k, v) in self.vars() {
            e = e.env(k, v);
        }
        e
    }
}

/// pid 1 of a namespace drops default-action signals from its ancestors; a
/// handler is what makes SIGTERM reach the child at all (§5 step 4). Must run
/// before any thread exists, which in practice means first thing in `main`.
fn term_flag() -> Result<Arc<AtomicBool>> {
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, term.clone())?;
    Ok(term)
}

/// Wait for the child, forwarding one SIGTERM; exit code, or 128+signal.
fn supervise(child: std::process::Child, term: &AtomicBool) -> Result<i32> {
    supervise_pid(Pid::from_raw(child.id() as i32), term)
}

fn supervise_pid(pid: Pid, term: &AtomicBool) -> Result<i32> {
    let mut forwarded = false;
    loop {
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)).context("waitpid")? {
            WaitStatus::Exited(_, code) => return Ok(code),
            WaitStatus::Signaled(_, sig, _) => return Ok(128 + sig as i32),
            _ => {}
        }
        if term.load(Ordering::Relaxed) && !forwarded {
            let _ = kill(pid, Signal::SIGTERM);
            forwarded = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- __ns ------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct NsArgs {
    /// The role to run as pid 1, with its arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    inner: Vec<String>,
}

pub fn ns_main(a: NsArgs) -> Result<i32> {
    let term = term_flag()?;
    ns::enter_userns(0, 0)?;
    ns::unshare_pid()?;
    let child = Command::new("/proc/self/exe")
        .args(&a.inner)
        .spawn()
        .context("spawn namespaced role")?;
    supervise(child, &term)
}

fn base_rootfs(root: &Path, resolv: &str) -> Result<Rootfs> {
    let fs = Rootfs::new(root)?;
    fs.proc()?;
    fs.sys_ro()?;
    fs.devices()?;
    fs.tmpfs("/tmp", "mode=1777")?;
    fs.mkdir("/tmp/home", 0o755)?;
    fs.etc(INNER_UID, INNER_GID, resolv)?;
    Ok(fs)
}

// ---- __fetch ---------------------------------------------------------------

#[derive(Args, Debug)]
pub struct FetchArgs {
    #[arg(long)]
    root: PathBuf,
    #[arg(long)]
    bootstrap: PathBuf,
    #[arg(long)]
    src: PathBuf,
    #[arg(long)]
    url: String,
    #[arg(long)]
    git_ref: String,
    #[arg(long)]
    commit: String,
}

pub fn fetch_main(a: FetchArgs) -> Result<i32> {
    let _term = term_flag()?;
    let resolv = ns::usable_resolv_conf()?;
    let tools = ToolEnv::resolve(&a.bootstrap, "/tmp".into())?;
    let fs = base_rootfs(&a.root, &resolv)?;
    fs.bind(&a.bootstrap.join("nix"), "/nix", true)?;
    fs.bind(&a.src, "/src", false)?;
    fs.pivot()?;

    let git = |args: &[&str]| tools.exec("git").args(["-C", "/src"]).args(args).output();
    tools.exec("git").args(["init", "-q", "/src"]).output()?;
    let by_sha = git(&[
        "fetch",
        "-q",
        "--depth=1",
        "--no-tags",
        "--no-recurse-submodules",
        &a.url,
        &a.commit,
    ]);
    if let Err(e) = by_sha {
        eprintln!(
            "fetch by sha failed ({e:#}); fetching {} and verifying",
            a.git_ref
        );
        git(&[
            "fetch",
            "-q",
            "--depth=1",
            "--no-tags",
            "--no-recurse-submodules",
            &a.url,
            &a.git_ref,
        ])?;
    }
    git(&["checkout", "-q", "--detach", "FETCH_HEAD"])?;
    let head = git(&["rev-parse", "HEAD"])?;
    if head.trim() != a.commit {
        bail!("fetched {} but the run pins {}", head.trim(), a.commit);
    }
    if Path::new("/src/.gitmodules").exists() {
        bail!("submodules are not supported: .gitmodules present");
    }
    fs::remove_dir_all("/src/.git").context("remove /src/.git")?;
    eprintln!("fetched {} @ {}", a.url, a.commit);
    Ok(0)
}

// ---- __build ---------------------------------------------------------------

#[derive(Args, Debug)]
pub struct BuildArgs {
    #[arg(long)]
    root: PathBuf,
    #[arg(long)]
    bootstrap: PathBuf,
    #[arg(long)]
    upper: PathBuf,
    #[arg(long)]
    work: PathBuf,
    #[arg(long)]
    src: PathBuf,
    #[arg(long)]
    xtmp: PathBuf,
    #[arg(long)]
    cache: PathBuf,
    #[arg(long)]
    key: PathBuf,
    #[arg(long)]
    pubkey: String,
    #[arg(long)]
    cores: u32,
    #[arg(long)]
    jobs: u32,
    #[arg(long)]
    tmpdir: String,
    /// Rebake mode (TCB): the candidate bootstrap dir, bound rw at `/candidate`;
    /// the single installable is the pinned flake ref, not a `/src` fragment.
    #[arg(long)]
    rebake_candidate: Option<PathBuf>,
    /// Host path, opened before the namespace exists: `BuildStats` lands here.
    #[arg(long)]
    stats_file: Option<PathBuf>,
    #[arg(last = true)]
    installables: Vec<String>,
}

/// What only `__build` can time: the build proper vs the push phase.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct BuildStats {
    pub build_secs: u64,
    pub push_secs: u64,
    pub pushed_paths: usize,
}

pub fn build_main(a: BuildArgs) -> Result<i32> {
    let term = term_flag()?;
    let resolv = ns::usable_resolv_conf()?;
    let tmpdir: PathBuf = match TmpDir::parse(&a.tmpdir)? {
        TmpDir::Ram => "/tmp".into(),
        TmpDir::Disk => "/xtmp".into(),
    };
    let tools = ToolEnv::resolve(&a.bootstrap, tmpdir.clone())?;
    let key = File::open(&a.key).with_context(|| format!("open {}", a.key.display()))?;
    let upper = File::open(&a.upper).with_context(|| format!("open {}", a.upper.display()))?;
    let lower_store = File::open(a.bootstrap.join("nix/store")).context("open lower store")?;
    let mut stats_file = a
        .stats_file
        .as_ref()
        .map(File::create)
        .transpose()
        .context("create stats file")?;
    let rebake = a.rebake_candidate.is_some();

    let fs = base_rootfs(&a.root, &resolv)?;
    fs.overlay("/nix", &a.bootstrap.join("nix"), &a.upper, &a.work)?;
    fs.bind(&a.src, "/src", false)?;
    fs.bind(&a.xtmp, "/xtmp", false)?;
    fs.bind(&a.cache, "/cache", !rebake)?;
    if let Some(c) = &a.rebake_candidate {
        fs.bind(c, "/candidate", false)?;
    }
    fs.tmpfs("/run", "mode=700")?;
    fs.write("/etc/nix/nix.conf", &nix_conf(&a, &tmpdir))?;
    fs.pivot()?;

    let mut nix_args: Vec<String> = vec![
        "build".into(),
        "--no-link".into(),
        "--print-build-logs".into(),
        "--keep-going".into(),
        "--no-update-lock-file".into(),
    ];
    if rebake {
        nix_args.push("--print-out-paths".into());
        nix_args.extend(a.installables.iter().cloned());
    } else {
        nix_args.extend(
            a.installables
                .iter()
                .map(|frag| format!("path:/src#{frag}")),
        );
    }
    let out_file = rebake.then(|| PathBuf::from("/run/out-paths"));
    let t = std::time::Instant::now();
    let code = run_stub(
        &tools,
        &tools.bin("nix"),
        &nix_args,
        out_file.as_deref(),
        &term,
    )?;
    let mut stats = BuildStats {
        build_secs: t.elapsed().as_secs(),
        ..Default::default()
    };

    let t = std::time::Instant::now();
    let code = if let Some(candidate) = &a.rebake_candidate {
        if code != 0 {
            code
        } else {
            let out = fs::read_to_string("/run/out-paths")?;
            let env_path = out
                .lines()
                .next()
                .context("rebake produced no out path")?
                .trim()
                .to_owned();
            push::rebake_publish(&tools, &env_path, Path::new("/candidate"), candidate)?;
            0
        }
    } else {
        match (code, push::push_phase(&tools, &upper, &lower_store, &key)) {
            (_, Err(push::PushError::Tamper(why))) => {
                eprintln!("TAMPER: {why}");
                exit::TAMPER
            }
            (0, Err(push::PushError::Push(why))) => {
                eprintln!("push failed: {why:#}");
                exit::PUSH_FAILED
            }
            (c, Err(push::PushError::Push(why))) => {
                eprintln!("push failed after a red build: {why:#}");
                c
            }
            (c, Ok(pushed)) => {
                stats.pushed_paths = pushed;
                c
            }
        }
    };
    stats.push_secs = t.elapsed().as_secs();
    if let Some(f) = stats_file.as_mut() {
        use std::io::Write;
        let _ = f.write_all(&serde_json::to_vec(&stats)?);
    }
    Ok(code)
}

fn nix_conf(a: &BuildArgs, tmpdir: &Path) -> String {
    format!(
        "sandbox = true\n\
         sandbox-fallback = false\n\
         require-sigs = true\n\
         substituters = https://cache.nixos.org file:///cache\n\
         trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= {CACHE_KEY_NAME}:{pubkey}\n\
         accept-flake-config = false\n\
         build-users-group =\n\
         cores = {cores}\n\
         max-jobs = {jobs}\n\
         build-dir = {tmpdir}/builds\n\
         experimental-features = nix-command flakes\n\
         warn-dirty = false\n",
        pubkey = a.pubkey.trim_start_matches(&format!("{CACHE_KEY_NAME}:")),
        cores = a.cores,
        jobs = a.jobs,
        tmpdir = tmpdir.display(),
    )
}

/// New pid namespace, then `__stub` as its pid 1. The isolation boundary is
/// the child's lifetime: when the stub exits the kernel kills everything left
/// in the namespace, so nothing repo-controlled can watch the push phase.
fn run_stub(
    tools: &ToolEnv,
    program: &Path,
    args: &[String],
    stdout: Option<&Path>,
    term: &AtomicBool,
) -> Result<i32> {
    ns::unshare_pid()?;
    let mut cmd = Command::new("/proc/self/exe");
    cmd.arg("__stub")
        .arg("--uid")
        .arg(INNER_UID.to_string())
        .arg("--gid")
        .arg(INNER_GID.to_string());
    if let Some(p) = stdout {
        cmd.arg("--stdout").arg(p);
    }
    cmd.arg("--").arg(program).args(args);
    cmd.env_clear();
    for (k, v) in tools.vars() {
        cmd.env(k, v);
    }
    let child = cmd.spawn().context("spawn __stub")?;
    let code = supervise(child, term)?;
    ns::restore_pid_for_children()?;
    Ok(code)
}

// ---- __stub ----------------------------------------------------------------

#[derive(Args, Debug)]
pub struct StubArgs {
    #[arg(long)]
    uid: u32,
    #[arg(long)]
    gid: u32,
    #[arg(long)]
    stdout: Option<PathBuf>,
    #[arg(last = true)]
    cmd: Vec<String>,
}

/// procfs for a pid namespace can only be mounted with CAP_SYS_ADMIN in the
/// userns that owns it, and exec as a non-root uid drops capabilities. So the
/// pid namespace is created *inside* the mapped userns and its pid 1 is a
/// plain `fork()` of this process — still our code, still capable — which
/// mounts `/proc` and then execs nix, the only thing that ever runs uncapable.
pub fn stub_main(a: StubArgs) -> Result<i32> {
    let term = term_flag()?;
    ns::enter_userns(a.uid, a.gid)?;
    ns::unshare_pid()?;
    // Single-threaded here (no thread has been spawned), so fork is sound.
    match unsafe { fork() }.context("fork pid 1")? {
        ForkResult::Parent { child } => supervise_pid(child, &term),
        ForkResult::Child => {
            let code = match init_main(&a, &term) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("__stub: {e:#}");
                    1
                }
            };
            std::process::exit(code)
        }
    }
}

fn init_main(a: &StubArgs, term: &AtomicBool) -> Result<i32> {
    ns::mount_proc(Path::new("/proc"))?;
    let (program, args) = a.cmd.split_first().context("__stub: empty command")?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(p) = &a.stdout {
        cmd.stdout(File::create(p)?);
    }
    let child = cmd.spawn().with_context(|| format!("spawn {program}"))?;
    supervise(child, term)
}
