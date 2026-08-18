//! `slurm-ci run`: what the patched runner execs per task. Hands the task to
//! the daemon, relays its output to the job log, exits with the verdict. A
//! SIGTERM (Forgejo cancel or timeout) becomes a `CANCEL` frame; the daemon
//! answers promptly, since the runner SIGKILLs after `WaitDelay`.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::frame::Frame;
use crate::proc::{env_or, env_var};
use crate::vps::ipc::{Task, ToDaemon, ToRun, DEFAULT_SOCKET};

pub fn main() -> Result<i32> {
    let task = Task {
        repo: env_var("CI_REPO")?,
        git_ref: env_var("CI_REF")?,
        commit: env_var("CI_COMMIT")?,
        event: env_var("CI_EVENT")?,
        token: env_var("CI_TOKEN")?,
    };
    let socket = env_or("CI_SOCKET", DEFAULT_SOCKET);
    let mut stream = UnixStream::connect(&socket).with_context(|| format!("connect {socket}"))?;
    ToDaemon::Task(task).to_frame().write_to(&mut stream)?;

    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, term.clone())?;
    let mut writer = stream.try_clone()?;
    let flag = term.clone();
    thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
        }
        let _ = ToDaemon::Cancel.to_frame().write_to(&mut writer);
    });

    let stdout = std::io::stdout();
    loop {
        let Some(frame) = Frame::read_from(&mut stream)? else {
            eprintln!("daemon closed the connection without a verdict");
            return Ok(1);
        };
        match ToRun::from_frame(frame)? {
            ToRun::Info(s) => eprintln!("slurm-ci: {s}"),
            ToRun::Data(d) => {
                let mut out = stdout.lock();
                out.write_all(&d)?;
                out.flush()?;
            }
            ToRun::Exit { status, verdict } => {
                eprintln!("slurm-ci: {verdict}");
                return Ok(status);
            }
        }
    }
}
