use std::path::PathBuf;

use agent_proto::{Key, Reply, read_key};
use anyhow::Result;

#[derive(clap::Args)]
pub struct Vm {
    /// The agent VM's vsock CID.
    #[arg(long, default_value_t = 4)]
    pub cid: u32,
    /// The key shared with the VM's daemon.
    #[arg(long)]
    key: PathBuf,
}

impl Vm {
    pub fn key(&self) -> Result<Key> {
        read_key(&self.key)
    }
}

/// How a wake went, read from the daemon's replies as they come: well once the daemon takes the message into a
/// running session, or the session it started exits 0, and not before; a refusal, a start or another exit is not.
#[derive(Default)]
pub struct Outcome {
    pub well: bool,
}

impl Outcome {
    pub fn read(&mut self, reply: &Reply) {
        self.well = match reply {
            Reply::Injected | Reply::Exited { code: Some(0) } => true,
            Reply::Started | Reply::Busy | Reply::Refused { .. } | Reply::Exited { .. } => false,
            Reply::Event(_) | Reply::Dropped { .. } => self.well,
        };
    }
}

#[cfg(test)]
#[path = "../tests/lib.rs"]
mod tests;
