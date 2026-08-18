//! Child-process helpers: fixed argv, no shell, stdin fed from a thread so a
//! large payload cannot deadlock against stdout.

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

pub struct Exec {
    cmd: Command,
    program: String,
    stdin: Option<Vec<u8>>,
}

impl Exec {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        let program_str = program.as_ref().to_string_lossy().into_owned();
        Exec {
            cmd: Command::new(program),
            program: program_str,
            stdin: None,
        }
    }

    pub fn arg(mut self, a: impl AsRef<OsStr>) -> Self {
        self.cmd.arg(a);
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(mut self, args: I) -> Self {
        self.cmd.args(args);
        self
    }

    pub fn env(mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> Self {
        self.cmd.env(k, v);
        self
    }

    pub fn env_clear(mut self) -> Self {
        self.cmd.env_clear();
        self
    }

    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    /// Run to completion; stdout on success, stderr in the error otherwise.
    pub fn output(mut self) -> Result<String> {
        let stdin = self.stdin.take();
        let mut child = self
            .cmd
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn {}", self.program))?;
        let feeder = stdin.map(|data| {
            let mut pipe = child.stdin.take().expect("piped stdin");
            std::thread::spawn(move || pipe.write_all(&data))
        });
        let out = child
            .wait_with_output()
            .with_context(|| format!("wait for {}", self.program))?;
        if let Some(t) = feeder {
            let _ = t.join();
        }
        if !out.status.success() {
            bail!(
                "{} exited {}: {}",
                self.program,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        String::from_utf8(out.stdout).with_context(|| format!("{}: non-utf8 output", self.program))
    }
}

pub fn env_var(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("{key} unset"))
}

pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}
