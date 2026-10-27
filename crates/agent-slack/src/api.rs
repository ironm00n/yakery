use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};

use crate::{Channel, Key, Message, Ts, User};

pub const SOCKET: &str = "/run/agent-slack.sock";

/// One JSON line each way; a wait is answered with a message, confirmed with `Taken`, and only then is it taken.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    Wait,
    Taken(Key),
    Send { to: Option<Channel>, thread: Option<Ts>, body: String },
    Read { to: Option<Channel>, thread: Ts },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Message(Message),
    Sent(Key),
    Thread(Vec<Said>),
    Taken,
    Refused(String),
}

/// One message of a thread as read prints it: a person's user id, or the bot id of an app's post.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Said {
    pub ts: Ts,
    pub user: Option<User>,
    pub bot_id: Option<String>,
    pub text: String,
}

pub struct Line<R, W>(pub Lines<BufReader<R>>, pub W);

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Line<R, W> {
    pub fn new(read: R, write: W) -> Self {
        Line(BufReader::new(read).lines(), write)
    }

    pub async fn send(&mut self, value: &impl Serialize) -> Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        self.1.write_all(&line).await?;
        Ok(self.1.flush().await?)
    }

    /// `None` once the other side has closed.
    pub async fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        let Some(line) = self.0.next_line().await? else { return Ok(None) };
        serde_json::from_str(&line).context("a malformed request").map(Some)
    }
}
