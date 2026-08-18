//! The control plane end to end on one machine: the daemon, a `run` client,
//! and `dispatch` behind a fake `ssh`, with fake Slurm commands and the test
//! itself playing the compute node over the real TLS callback.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use slurm_ci::callback::{self, Hello, Msg};
use slurm_ci::cluster::rundir::JobFile;
use slurm_ci::ids::Token;
use slurm_ci::verbs::JobId;
use slurm_ci::verdict::exit;

const BIN: &str = env!("CARGO_BIN_EXE_slurm-ci");
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

struct Harness {
    _dir: tempfile::TempDir,
    fake_home: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    port: u16,
    daemon: Child,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn script(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(50));
    }
}

impl Harness {
    fn new(toml: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fake_home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let state = dir.path().join("state");
        let socket = dir.path().join("listen.sock");
        fs::create_dir_all(&fake_home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(fake_home.join("slurm-ci.toml"), toml).unwrap();

        script(
            &bin,
            "ssh",
            r#"for last; do :; done
exec env SSH_ORIGINAL_COMMAND="$last" HOME="$FAKE_HOME" CI_SLURM_ACCOUNT=test PATH="$FAKE_BIN:$PATH" "$SLURM_CI" dispatch"#,
        );
        script(
            &bin,
            "sbatch",
            r#"n=$(cat "$FAKE_HOME/next_job" 2>/dev/null || echo 1000)
echo $((n+1)) > "$FAKE_HOME/next_job"
printf '%s\n' "$@" > "$FAKE_HOME/sbatch.$n.args"
printf '%s|PENDING|0:0|None assigned|sharing|2026-09-13T10:00:00|Unknown|Unknown|00:00:00|00:45:00|32|65536M||\n' "$n" > "$FAKE_HOME/sacct.$n"
echo "$n""#,
        );
        script(
            &bin,
            "sacct",
            r#"while [ $# -gt 0 ]; do [ "$1" = -j ] && id=$2; shift; done
cat "$FAKE_HOME/sacct.$id" 2>/dev/null; exit 0"#,
        );
        script(
            &bin,
            "scancel",
            r#"printf '%s|CANCELLED by 1000|0:0|d0020|sharing|a|b|c|00:00:01|00:45:00|32|65536M||\n%s.batch|CANCELLED|0:15|d0020||a|b|c|00:00:01||32||00:00:01|1000K\n' "$1" "$1" > "$FAKE_HOME/sacct.$1"
echo "$1" >> "$FAKE_HOME/scancelled""#,
        );
        script(&bin, "squeue", "exit 0");
        script(&bin, "sinfo", "echo 1:00:00");
        script(
            &bin,
            "curl",
            r#"cat > /dev/null; cat "$FAKE_HOME/slurm-ci.toml""#,
        );

        let port = free_port();
        let daemon = Command::new(BIN)
            .arg("listen")
            .env("CI_STATE_DIR", &state)
            .env("CI_SOCKET", &socket)
            .env("CI_CALLBACK_BIND", format!("127.0.0.1:{port}"))
            .env("CI_CALLBACK_HOST", format!("127.0.0.1:{port}"))
            .env("CI_FORGEJO_API", "http://forge.invalid")
            .env("CI_FORGEJO_GIT_HOST", "git.example")
            .env("CI_CLUSTER_USER", "ci")
            .env("CI_CLUSTER_HOST", "login.invalid")
            .env("CI_SSH_KEY", "/dev/null")
            .env("CI_KNOWN_HOSTS", "/dev/null")
            .env("CI_SSH_BIN", bin.join("ssh"))
            .env("CI_CURL_BIN", bin.join("curl"))
            .env("CI_PROBE_MIN_SECS", "1")
            .env("FAKE_HOME", &fake_home)
            .env("FAKE_BIN", &bin)
            .env("SLURM_CI", BIN)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let h = Harness {
            _dir: dir,
            fake_home,
            state,
            socket,
            port,
            daemon,
        };
        wait_for("daemon socket", || h.socket.exists());
        h
    }

    fn start_run(&self, event: &str) -> Child {
        Command::new(BIN)
            .arg("run")
            .env("CI_SOCKET", &self.socket)
            .env("CI_REPO", "ironmoon/yakery")
            .env("CI_REF", "refs/heads/master")
            .env("CI_COMMIT", COMMIT)
            .env("CI_EVENT", event)
            .env("CI_TOKEN", "forge-token")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// The newest run dir with a job id: what a compute node would open.
    fn wait_run_dir(&self, seen: &[PathBuf]) -> (PathBuf, JobFile, Token, JobId) {
        let ci = self.fake_home.join("ci");
        let mut found = None;
        wait_for("run dir with jobid", || {
            let Ok(entries) = fs::read_dir(&ci) else {
                return false;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.file_name().unwrap().to_string_lossy().starts_with("run-")
                    && !seen.contains(&p)
                    && p.join("jobid").exists()
                {
                    found = Some(p);
                    return true;
                }
            }
            false
        });
        let dir = found.unwrap();
        let job: JobFile =
            serde_json::from_slice(&fs::read(dir.join("job.json")).unwrap()).unwrap();
        let token = Token::parse(fs::read_to_string(dir.join("cbtoken")).unwrap().trim()).unwrap();
        let jobid = JobId::parse(fs::read_to_string(dir.join("jobid")).unwrap().trim()).unwrap();
        (dir, job, token, jobid)
    }

    fn compute_node(&self, job: &JobFile, token: Token, jobid: &JobId) -> callback::ClientStream {
        let mut s = callback::connect("127.0.0.1", self.port, &job.submit.fingerprint).unwrap();
        let hello = Hello {
            run: job.submit.run.clone(),
            token,
            job: Some(jobid.clone()),
            node: "d0020".into(),
        };
        Msg::Hello(hello).to_frame().write_to(&mut s).unwrap();
        s
    }

    fn set_sacct(&self, jobid: &JobId, state: &str, batch_exit: &str) {
        let rows = format!(
            "{j}|{state}|0:0|d0020|sharing|2026-09-13T10:00:00|2026-09-13T10:00:05|2026-09-13T10:10:00|00:09:55|00:45:00|32|65536M||\n\
             {j}.batch|{state}|{batch_exit}|d0020||2026-09-13T10:00:05|2026-09-13T10:00:05|2026-09-13T10:10:00|00:09:55||32||00:20:00|4000000K\n",
            j = jobid
        );
        fs::write(self.fake_home.join(format!("sacct.{jobid}")), rows).unwrap();
    }

    fn finish(mut run: Child) -> (i32, String, String) {
        let mut out = String::new();
        let mut err = String::new();
        let mut stdout = run.stdout.take().unwrap();
        let mut stderr = run.stderr.take().unwrap();
        let t = thread::spawn(move || {
            let mut e = String::new();
            stderr.read_to_string(&mut e).unwrap();
            e
        });
        stdout.read_to_string(&mut out).unwrap();
        err.push_str(&t.join().unwrap());
        let status = run.wait().unwrap();
        (status.code().unwrap_or(-1), out, err)
    }

    fn jsonl(&self) -> Vec<serde_json::Value> {
        let text = fs::read_to_string(self.state.join("efficiency.jsonl")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn sbatch_args(&self, jobid: &JobId) -> Vec<String> {
        fs::read_to_string(self.fake_home.join(format!("sbatch.{jobid}.args")))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

const TOML: &str =
    "installables = [\"packages.x86_64-linux.default\"]\ncores = 4\nmem = \"8G\"\ntime = \"30m\"\n";

#[test]
fn green_via_callback_streams_log() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (dir, job, token, jobid) = h.wait_run_dir(&[]);
    assert_eq!(job.submit.commit, COMMIT);
    assert_eq!(job.submit.spec.cores, 4);
    let args = h.sbatch_args(&jobid);
    assert!(
        args.iter()
            .any(|a| a == &format!("--comment=slurm-ci:{}", job.submit.run)),
        "{args:?}"
    );
    assert!(args.iter().any(|a| a == "--account=test"));
    assert!(args.iter().any(|a| a == "--mem=8192M"));
    assert!(args.iter().any(|a| a == "--time=30"));
    assert!(args.iter().any(|a| a == "--partition=sharing"));
    assert!(args
        .iter()
        .any(|a| a.starts_with("--wrap=") && a.ends_with(&format!("build {}", job.submit.run))));
    assert!(
        !args.iter().any(|a| a.contains(token.as_str())),
        "token on the sbatch line"
    );

    let mut node = h.compute_node(&job, token, &jobid);
    Msg::Data(b"building...\n".to_vec())
        .to_frame()
        .write_to(&mut node)
        .unwrap();
    Msg::Heartbeat.to_frame().write_to(&mut node).unwrap();
    Msg::Data(b"done\n".to_vec())
        .to_frame()
        .write_to(&mut node)
        .unwrap();
    h.set_sacct(&jobid, "COMPLETED", "0:0");
    Msg::Exit(0).to_frame().write_to(&mut node).unwrap();
    callback::close(node);

    let (code, out, err) = Harness::finish(run);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "building...\ndone\n");
    assert!(err.contains("green"), "{err}");
    assert!(err.contains("started on d0020"), "{err}");

    wait_for("done marker", || dir.join("done").exists());
    let records = h.jsonl();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["verdict"]["kind"], "green");
    assert_eq!(records[0]["verdict_via"], "callback");
    assert_eq!(records[0]["eff"]["sacct"]["max_rss"], "4000000K");
    assert_eq!(records[0]["mem_overshoot"], false);
    wait_for("registry cleared", || {
        fs::read_dir(h.state.join("runs")).unwrap().count() == 0
    });
}

#[test]
fn red_via_sacct_fallback_fetches_log() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (dir, _job, _token, jobid) = h.wait_run_dir(&[]);
    fs::write(dir.join("job.out"), "compiler said no\n").unwrap();
    h.set_sacct(&jobid, "FAILED", "1:0");
    let (code, out, err) = Harness::finish(run);
    assert_eq!(code, 1);
    assert_eq!(out, "compiler said no\n");
    assert!(err.contains("red: build exited 1"), "{err}");
    let records = h.jsonl();
    assert_eq!(records[0]["verdict_via"], "sacct");
}

#[test]
fn push_failed_is_green_with_warning_and_timeout_is_red() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (dir1, _, _, jobid) = h.wait_run_dir(&[]);
    h.set_sacct(&jobid, "FAILED", &format!("{}:0", exit::PUSH_FAILED));
    let (code, _, err) = Harness::finish(run);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("cache push failed"), "{err}");

    let run = h.start_run("push");
    let (_, _, _, jobid) = h.wait_run_dir(&[dir1]);
    // job 9442675's shape: TIMEOUT with 0:0 on the allocation row
    let rows = format!(
        "{j}|TIMEOUT|0:0|d3206|sharing|a|b|c|01:00:18|01:00:00|128|126000M||\n{j}.batch|CANCELLED|0:15|d3206||a|b|c|01:00:18||128||4-06:52:31|183917204K\n",
        j = jobid
    );
    fs::write(h.fake_home.join(format!("sacct.{jobid}")), rows).unwrap();
    let (code, _, err) = Harness::finish(run);
    assert_eq!(code, 1);
    assert!(err.contains("TIMEOUT"), "{err}");
    let last = h.jsonl().pop().unwrap();
    assert_eq!(last["mem_overshoot"], true);
}

#[test]
fn gate_failure_resubmits_excluding_node() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (dir1, job1, token1, jobid1) = h.wait_run_dir(&[]);
    let mut node = h.compute_node(&job1, token1, &jobid1);
    Msg::Data(b"gate: /tmp is nfs\n".to_vec())
        .to_frame()
        .write_to(&mut node)
        .unwrap();
    h.set_sacct(&jobid1, "FAILED", &format!("{}:0", exit::GATE_FAILED));
    Msg::Exit(exit::GATE_FAILED)
        .to_frame()
        .write_to(&mut node)
        .unwrap();
    callback::close(node);

    let (_, job2, token2, jobid2) = h.wait_run_dir(&[dir1]);
    assert_ne!(job2.submit.run, job1.submit.run);
    assert_eq!(job2.submit.exclude.as_deref(), Some("d0020"));
    assert!(h
        .sbatch_args(&jobid2)
        .iter()
        .any(|a| a == "--exclude=d0020"));
    let mut node = h.compute_node(&job2, token2, &jobid2);
    h.set_sacct(&jobid2, "COMPLETED", "0:0");
    Msg::Exit(0).to_frame().write_to(&mut node).unwrap();
    callback::close(node);
    let (code, out, err) = Harness::finish(run);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("gate: /tmp is nfs"));
    assert!(err.contains("resubmitting with --exclude=d0020"), "{err}");
    let records = h.jsonl();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["attempt"], 1);
    assert_eq!(records[1]["attempt"], 2);
    assert_eq!(records[1]["verdict"]["kind"], "green");
}

#[test]
fn cancel_scancels_and_exits_promptly() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (_, _, _, jobid) = h.wait_run_dir(&[]);
    let pid = nix::unistd::Pid::from_raw(run.id() as i32);
    let t = Instant::now();
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).unwrap();
    let (code, _, err) = Harness::finish(run);
    assert_eq!(code, 1);
    assert!(t.elapsed() < Duration::from_secs(10));
    assert!(err.contains("cancelled"), "{err}");
    wait_for("scancel", || h.fake_home.join("scancelled").exists());
    assert_eq!(
        fs::read_to_string(h.fake_home.join("scancelled"))
            .unwrap()
            .trim(),
        jobid.as_str()
    );
    wait_for("jsonl", || !h.jsonl().is_empty());
    assert_eq!(h.jsonl()[0]["verdict"]["kind"], "red");
}

#[test]
fn wrong_token_cannot_claim_a_run() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (dir1, job, token, jobid) = h.wait_run_dir(&[]);
    let mut thief = h.compute_node(&job, Token::random().unwrap(), &jobid);
    let _ = Msg::Exit(0).to_frame().write_to(&mut thief);
    let mut buf = [0u8; 1];
    let _ = thief.read(&mut buf);
    drop(thief);
    thread::sleep(Duration::from_millis(300));
    assert!(
        fs::read_dir(h.state.join("runs")).unwrap().count() == 1,
        "thief concluded the run"
    );

    let mut node = h.compute_node(&job, token.clone(), &jobid);
    h.set_sacct(&jobid, "COMPLETED", "0:0");
    Msg::Exit(0).to_frame().write_to(&mut node).unwrap();
    callback::close(node);
    let (code, _, _) = Harness::finish(run);
    assert_eq!(code, 0);

    // the token was retired by the first valid HELLO: a replay is refused
    let run2 = h.start_run("push");
    let (_, job2, _, jobid2) = h.wait_run_dir(&[dir1]);
    let mut replay = h.compute_node(&job2, token, &jobid2);
    let _ = Msg::Exit(0).to_frame().write_to(&mut replay);
    drop(replay);
    thread::sleep(Duration::from_millis(300));
    h.set_sacct(&jobid2, "FAILED", "1:0");
    let (code, _, _) = Harness::finish(run2);
    assert_eq!(code, 1);
}

#[test]
fn non_push_events_and_bad_specs_are_refused() {
    let h = Harness::new("installables = [\"x\"]\ntime = \"3h\"\n");
    let (code, _, err) = Harness::finish(h.start_run("pull_request"));
    assert_eq!(code, 1);
    assert!(err.contains("not push"), "{err}");
    let (code, _, err) = Harness::finish(h.start_run("push"));
    assert_eq!(code, 1);
    assert!(err.contains("exceeds sharing cap"), "{err}");
    assert!(
        !h.fake_home.join("ci").exists(),
        "nothing should have reached dispatch"
    );
}

#[test]
fn dispatch_refuses_foreign_job_ids() {
    let h = Harness::new(TOML);
    let run = h.start_run("push");
    let (_, job, _, jobid) = h.wait_run_dir(&[]);
    let other = JobId::parse("424242").unwrap();
    let out = Command::new(BIN)
        .arg("dispatch")
        .env(
            "SSH_ORIGINAL_COMMAND",
            format!("cancel {other} {}", job.submit.run),
        )
        .env("HOME", &h.fake_home)
        .env("CI_SLURM_ACCOUNT", "test")
        .env(
            "PATH",
            format!(
                "{}:{}",
                h._dir.path().join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("dispatch refused"));
    assert!(!h.fake_home.join("scancelled").exists());
    h.set_sacct(&jobid, "COMPLETED", "0:0");
    let (code, _, _) = Harness::finish(run);
    assert_eq!(code, 0);
}

#[test]
fn daemon_restart_reattaches() {
    let mut h = Harness::new(TOML);
    let run = h.start_run("push");
    let (_, _, _, jobid) = h.wait_run_dir(&[]);
    h.daemon.kill().unwrap();
    h.daemon.wait().unwrap();
    let (code, _, err) = Harness::finish(run);
    assert_eq!(code, 1);
    assert!(err.contains("without a verdict"), "{err}");
    assert_eq!(fs::read_dir(h.state.join("runs")).unwrap().count(), 1);

    h.set_sacct(&jobid, "COMPLETED", "0:0");
    let bin = h._dir.path().join("bin");
    let _ = fs::remove_file(&h.socket);
    h.daemon = Command::new(BIN)
        .arg("listen")
        .env("CI_STATE_DIR", &h.state)
        .env("CI_SOCKET", &h.socket)
        .env("CI_CALLBACK_BIND", format!("127.0.0.1:{}", h.port))
        .env("CI_CALLBACK_HOST", format!("127.0.0.1:{}", h.port))
        .env("CI_FORGEJO_API", "http://forge.invalid")
        .env("CI_FORGEJO_GIT_HOST", "git.example")
        .env("CI_CLUSTER_USER", "ci")
        .env("CI_CLUSTER_HOST", "login.invalid")
        .env("CI_SSH_KEY", "/dev/null")
        .env("CI_KNOWN_HOSTS", "/dev/null")
        .env("CI_SSH_BIN", bin.join("ssh"))
        .env("CI_CURL_BIN", bin.join("curl"))
        .env("CI_PROBE_MIN_SECS", "1")
        .env("FAKE_HOME", &h.fake_home)
        .env("FAKE_BIN", &bin)
        .env("SLURM_CI", BIN)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    wait_for("reconciled", || {
        fs::read_dir(h.state.join("runs")).unwrap().count() == 0
    });
    let records = h.jsonl();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["verdict"]["kind"], "green");
    assert_eq!(records[0]["verdict_via"], "sacct");
}
