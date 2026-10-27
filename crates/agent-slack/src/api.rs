use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines},
    time::timeout,
};

use crate::{Channel, Key, Message, Ts, User};

pub const SOCKET: &str = "/run/agent-slack.sock";
/// How long a client waits for agent-slackd to take up its request, which it does before any work, so a daemon that
/// is not running fails the client rather than leave it queued on systemd's socket.
pub const ANSWER: Duration = Duration::from_secs(60);

/// One JSON line each way; every request is answered `Accepted` first. A wait is then answered with a message,
/// confirmed with `Taken`, and only then is it taken.
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
    Accepted,
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

    /// The reply to `request` after the daemon's `Accepted`, which must come within `answer`.
    pub async fn request(&mut self, request: &Request, answer: Duration) -> Result<Reply> {
        self.send(request).await?;
        let Ok(accepted) = timeout(answer, self.next()).await else {
            bail!("agent-slackd took up no request in {answer:?}; is agent-slackd.service running?")
        };
        ensure!(matches!(accepted?, Some(Reply::Accepted)), "agent-slackd answered out of turn");
        self.next().await?.context("agent-slackd closed the connection")
    }
}
