//! run ↔ daemon protocol over the unix socket. `run` is a thin client: it
//! sends one `TASK`, relays what comes back to the runner's log, and sends
//! `CANCEL` on SIGTERM.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::frame::Frame;

pub const DEFAULT_SOCKET: &str = "/run/slurm-ci/listen.sock";

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Task {
    pub repo: String,
    pub git_ref: String,
    pub commit: String,
    pub event: String,
    /// The ephemeral Forgejo job token; used for the toml fetch only.
    pub token: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToDaemon {
    Task(Task),
    Cancel,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToRun {
    /// A status line for the runner's log.
    Info(String),
    /// Raw log bytes from the build.
    Data(Vec<u8>),
    /// The run's exit status and a one-line verdict.
    Exit { status: i32, verdict: String },
}

mod kind {
    pub const TASK: u8 = 10;
    pub const CANCEL: u8 = 11;
    pub const INFO: u8 = 20;
    pub const DATA: u8 = 21;
    pub const EXIT: u8 = 22;
}

impl ToDaemon {
    pub fn to_frame(&self) -> Frame {
        match self {
            ToDaemon::Task(t) => {
                Frame::new(kind::TASK, serde_json::to_vec(t).expect("task serialises"))
            }
            ToDaemon::Cancel => Frame::new(kind::CANCEL, Vec::new()),
        }
    }
    pub fn from_frame(f: Frame) -> Result<Self> {
        Ok(match f.kind {
            kind::TASK => ToDaemon::Task(serde_json::from_slice(&f.payload).context("bad TASK")?),
            kind::CANCEL => ToDaemon::Cancel,
            k => bail!("unknown run→daemon frame {k}"),
        })
    }
}

impl ToRun {
    pub fn to_frame(&self) -> Frame {
        match self {
            ToRun::Info(s) => Frame::new(kind::INFO, s.as_bytes().to_vec()),
            ToRun::Data(d) => Frame::new(kind::DATA, d.clone()),
            ToRun::Exit { status, verdict } => {
                let mut p = status.to_be_bytes().to_vec();
                p.extend_from_slice(verdict.as_bytes());
                Frame::new(kind::EXIT, p)
            }
        }
    }
    pub fn from_frame(f: Frame) -> Result<Self> {
        Ok(match f.kind {
            kind::INFO => ToRun::Info(String::from_utf8_lossy(&f.payload).into_owned()),
            kind::DATA => ToRun::Data(f.payload),
            kind::EXIT => {
                if f.payload.len() < 4 {
                    bail!("bad EXIT");
                }
                let status = i32::from_be_bytes(f.payload[..4].try_into().expect("4 bytes"));
                ToRun::Exit {
                    status,
                    verdict: String::from_utf8_lossy(&f.payload[4..]).into_owned(),
                }
            }
            k => bail!("unknown daemon→run frame {k}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let task = Task {
            repo: "o/r".into(),
            git_ref: "refs/heads/x".into(),
            commit: "0".repeat(40),
            event: "push".into(),
            token: "t".into(),
        };
        for m in [ToDaemon::Task(task), ToDaemon::Cancel] {
            assert_eq!(ToDaemon::from_frame(m.to_frame()).unwrap(), m);
        }
        for m in [
            ToRun::Info("hi".into()),
            ToRun::Data(vec![0, 1, 2]),
            ToRun::Exit {
                status: 1,
                verdict: "red: x".into(),
            },
        ] {
            assert_eq!(ToRun::from_frame(m.to_frame()).unwrap(), m);
        }
        assert!(ToRun::from_frame(ToDaemon::Cancel.to_frame()).is_err());
    }
}
