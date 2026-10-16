//! What the host and the VM's root daemon say to each other over vsock. A
//! control connection carries one [`Request`], then [`Reply`] lines until the
//! daemon closes it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub const HOST_CID: u32 = 2;
pub const CONTROL_PORT: u32 = 6000;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Hand `message` to the agent: a new session if it has none running, or
    /// a `next` user message into the one that is.
    Wake { agent: String, message: String },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Started,
    Injected,
    Refused {
        reason: String,
    },
    /// One stream-json line from the session, as the client wrote it.
    Event(Box<RawValue>),
    Exited {
        code: Option<i32>,
    },
}

impl Reply {
    pub fn line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("replies serialize");
        line.push('\n');
        line
    }
}
