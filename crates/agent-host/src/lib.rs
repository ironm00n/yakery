use std::path::PathBuf;

use agent_proto::{Key, read_key};
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
