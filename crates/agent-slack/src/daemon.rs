use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, ensure};
use futures_util::{SinkExt, StreamExt};
use private::Dir;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::Notify,
};
use tokio_tungstenite::tungstenite::Message as Frame;
use yakery_log::{note, warn};

use crate::{
    Channel, Entry, Inbox, Raw, Ts, User, admit,
    api::{Line, Reply, Request, Said},
    channels, chunks,
    slack::Slack,
};

const INBOX: &str = "inbox";
/// chat.postMessage's limit on markdown_text.
pub const MAX_MARKDOWN: usize = 12_000;
/// Slack's pace for posts to one conversation.
pub const PACE: Duration = Duration::from_secs(1);
pub const RECONNECT: Duration = Duration::from_secs(1);

pub struct Daemon {
    slack: Slack,
    dir: Dir,
    list: PathBuf,
    bot: User,
    state: Mutex<State>,
    arrived: Notify,
}

struct State {
    inbox: Inbox,
    waiter: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    envelope_id: Option<String>,
    #[serde(default)]
    payload: Value,
}

type Client = Line<OwnedReadHalf, OwnedWriteHalf>;

/// Frees the one wait slot however the wait ends.
struct Waiting<'a>(&'a Daemon);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.state().waiter = None;
    }
}

impl Daemon {
    pub async fn start(slack: Slack, dir: Dir, list: PathBuf) -> Result<Arc<Daemon>> {
        let bot = slack.bot_user().await.context("asking Slack who the bot is")?;
        let held = dir.read(INBOX)?.map(String::from_utf8).transpose()?.unwrap_or_default();
        let inbox = Inbox::replay(&held).context("replaying the inbox")?;
        let state = Mutex::new(State { inbox, waiter: None });
        Ok(Arc::new(Daemon { slack, dir, list, bot, state, arrived: Notify::new() }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reread every time, so root's edits take effect at once; an unreadable list counts as empty.
    fn listed(&self) -> Vec<Channel> {
        let read = std::fs::read_to_string(&self.list).map_err(anyhow::Error::from);
        read.and_then(|list| channels(&list)).unwrap_or_else(|e| {
            warn!("{}: {e:#}; listening nowhere", self.list.display());
            Vec::new()
        })
    }

    fn target(&self, to: Option<Channel>) -> Result<Channel> {
        let listed = self.listed();
        match to {
            Some(c) if listed.contains(&c) => Ok(c),
            Some(c) => Err(anyhow!("{c} is not on {}", self.list.display())),
            None => listed
                .into_iter()
                .next()
                .with_context(|| format!("{} is empty", self.list.display())),
        }
    }

    /// On disk and synced before the inbox counts it, so an ack sent after this cannot lose it.
    fn record(&self, entry: Entry) -> Result<()> {
        let mut state = self.state();
        if let Entry::Message(m) = &entry
            && state.inbox.holds(&m.key())
        {
            return Ok(());
        }
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        self.dir.append(INBOX, &line)?;
        let arrived = matches!(entry, Entry::Message(_));
        state.inbox.apply(entry);
        drop(state);
        if arrived {
            self.arrived.notify_one();
        }
        Ok(())
    }

    fn store(&self, raw: &Raw, listed: &[Channel]) -> Result<()> {
        admit(raw, listed, &self.bot).map_or(Ok(()), |m| self.record(Entry::Message(m)))
    }

    pub async fn run(self: Arc<Self>, listener: UnixListener) {
        tokio::join!(self.clone().serve(listener), self.socket_mode());
    }

    async fn socket_mode(&self) {
        loop {
            match self.connection().await {
                Ok(()) => note!("Slack closed the connection; reconnecting"),
                Err(e) => warn!("Socket Mode: {e:#}"),
            }
            tokio::time::sleep(RECONNECT).await;
        }
    }

    async fn connection(&self) -> Result<()> {
        let mut socket = self.slack.connect().await?;
        self.catch_up().await;
        while let Some(frame) = socket.next().await {
            let Frame::Text(text) = frame? else { continue };
            let envelope: Envelope = serde_json::from_str(&text)?;
            if envelope.kind == "events_api" {
                match serde_json::from_value(envelope.payload["event"].clone()) {
                    Ok(raw) => self.store(&raw, &self.listed())?,
                    Err(e) => warn!("dropping an event Slack sent malformed: {e}"),
                }
            }
            if let Some(id) = envelope.envelope_id {
                socket.send(Frame::text(json!({ "envelope_id": id }).to_string())).await?;
            }
            if envelope.kind == "disconnect" {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Each listed channel since the newest message held from it, the replies of threads with newer ones included.
    async fn catch_up(&self) {
        let listed = self.listed();
        for channel in &listed {
            let caught = async {
                let Some(since) = self.state().inbox.newest(channel) else {
                    return self.record(Entry::Mark((channel.clone(), Ts::now())));
                };
                let mut found = Vec::new();
                for top in self.slack.history(channel, since).await? {
                    if let (Some(ts), Some(latest)) = (top.ts, top.latest_reply)
                        && latest > since
                    {
                        found.extend(self.slack.replies(channel, ts, Some(since)).await?);
                    }
                    found.push(top);
                }
                found.sort_by_key(|r| r.ts);
                found.iter().try_for_each(|raw| self.store(raw, &listed))
            };
            if let Err(e) = caught.await {
                warn!("catching up on {channel}: {e:#}");
            }
        }
    }

    async fn serve(self: Arc<Self>, listener: UnixListener) {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => drop(tokio::spawn(self.clone().client(stream))),
                Err(e) => warn!("accepting a local client: {e}"),
            }
        }
    }

    async fn client(self: Arc<Self>, stream: UnixStream) {
        let who = stream
            .peer_cred()
            .ok()
            .and_then(|c| c.pid())
            .map_or("a client".into(), |p| format!("pid {p}"));
        let (read, write) = stream.into_split();
        let mut line = Line::new(read, write);
        let served = async {
            let reply = match line.next().await? {
                None => return Ok(()),
                Some(Request::Wait) => return self.wait(who, &mut line).await,
                Some(Request::Send { to, thread, body }) => self.send(to, thread, &body).await,
                Some(Request::Read { to, thread }) => self.read(to, thread).await,
                Some(Request::Taken(_)) => Err(anyhow!("nothing was handed over to take")),
            };
            line.send(&reply.unwrap_or_else(|e| Reply::Refused(format!("{e:#}")))).await
        };
        if let Err(e) = served.await {
            warn!("local client: {e:#}");
        }
    }

    fn claim(&self, who: String) -> std::result::Result<Waiting<'_>, String> {
        let mut state = self.state();
        if let Some(other) = &state.waiter {
            return Err(format!("{other} is waiting already"));
        }
        state.waiter = Some(who);
        Ok(Waiting(self))
    }

    /// At least once: the message is taken only when the client confirms it printed it.
    async fn wait(&self, who: String, line: &mut Client) -> Result<()> {
        let _waiting = match self.claim(who) {
            Ok(waiting) => waiting,
            Err(why) => return line.send(&Reply::Refused(why)).await,
        };
        let message = loop {
            if let Some(m) = self.state().inbox.next().cloned() {
                break m;
            }
            tokio::select! {
                () = self.arrived.notified() => {}
                _ = line.next::<Request>() => return Ok(()),
            }
        };
        line.send(&Reply::Message(message.clone())).await?;
        match line.next().await? {
            Some(Request::Taken(key)) if key == message.key() => {
                self.record(Entry::Taken(key))?;
                line.send(&Reply::Taken).await
            }
            _ => Ok(()),
        }
    }

    /// A body past Slack's limit goes out in pieces, the rest in the first's thread.
    async fn send(&self, to: Option<Channel>, thread: Option<Ts>, body: &str) -> Result<Reply> {
        let channel = self.target(to)?;
        ensure!(!body.trim().is_empty(), "the body is empty");
        let mut first = None;
        for (i, chunk) in chunks(body, MAX_MARKDOWN).into_iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(PACE).await;
            }
            let ts = self.slack.post(&channel, thread.or(first), chunk).await?;
            first.get_or_insert(ts);
        }
        Ok(Reply::Sent((channel, first.context("a body with text has a piece")?)))
    }

    async fn read(&self, to: Option<Channel>, thread: Ts) -> Result<Reply> {
        let channel = self.target(to)?;
        let said = self.slack.replies(&channel, thread, None).await?.into_iter().filter_map(|r| {
            Some(Said {
                ts: r.ts?,
                user: r.user,
                bot_id: r.bot_id,
                text: r.text.unwrap_or_default(),
            })
        });
        Ok(Reply::Thread(said.collect()))
    }
}
