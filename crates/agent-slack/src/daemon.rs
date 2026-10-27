use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
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
    time::{Instant, timeout},
};
use tokio_tungstenite::tungstenite::Message as Frame;
use yakery_log::{fail, note, warn};

use crate::{
    Channel, Entry, Inbox, Raw, Ts, User, admit,
    api::{Line, Reply, Request, Said},
    channels, chunks,
    slack::{Slack, TIMEOUT},
};

const INBOX: &str = "inbox";
/// chat.postMessage's limit on markdown_text.
pub const MAX_MARKDOWN: usize = 12_000;
/// Slack's pace for posts to one conversation.
pub const PACE: Duration = Duration::from_secs(1);

/// How long the daemon may go with no live connection before it refuses every request, so whichever the session runs
/// next hears that Slack is not reaching it; how often a waiting request looks again; the pause between connections;
/// and how long a connection may stay silent before it is pinged, and as long again before it is dropped.
#[derive(Clone, Copy)]
pub struct Bounds {
    pub down: Duration,
    pub recheck: Duration,
    pub reconnect: Duration,
    pub ping: Duration,
}

pub const BOUNDS: Bounds = Bounds {
    down: Duration::from_secs(120),
    recheck: Duration::from_secs(5),
    reconnect: Duration::from_secs(1),
    ping: Duration::from_secs(30),
};

pub struct Daemon {
    slack: Slack,
    dir: Dir,
    list: PathBuf,
    bot: User,
    bounds: Bounds,
    state: Mutex<State>,
    arrived: Notify,
}

struct State {
    inbox: Inbox,
    waiter: Option<String>,
    /// Since when no connection has been live, that is open and caught up, and the last reason; `None` while one is.
    down: Option<(Instant, String)>,
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
    pub async fn start(
        slack: Slack,
        dir: Dir,
        list: PathBuf,
        bounds: Bounds,
    ) -> Result<Arc<Daemon>> {
        let bot = slack.bot_user().await.context("asking Slack who the bot is")?;
        let held = dir.read(INBOX)?.map(String::from_utf8).transpose()?.unwrap_or_default();
        let inbox = Inbox::replay(&held).context("replaying the inbox")?;
        let down = Some((Instant::now(), "not connected yet".into()));
        let state = Mutex::new(State { inbox, waiter: None, down });
        Ok(Arc::new(Daemon { slack, dir, list, bot, bounds, state, arrived: Notify::new() }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reread every time, so root's edits take effect at once.
    fn listed(&self) -> Result<Vec<Channel>> {
        let list = std::fs::read_to_string(&self.list).map_err(anyhow::Error::from);
        let listed = list.and_then(|list| channels(&list)).and_then(|listed| {
            ensure!(!listed.is_empty(), "it lists no channel");
            Ok(listed)
        });
        listed.with_context(|| format!("the channel list {}", self.list.display()))
    }

    /// The listed channels, unless the daemon cannot hear Slack.
    fn hearing(&self) -> Result<Vec<Channel>> {
        if let Some((since, why)) = &self.state().down
            && since.elapsed() >= self.bounds.down
        {
            let secs = since.elapsed().as_secs();
            bail!("agent-slackd has had no live Socket Mode connection for {secs}s: {why}");
        }
        self.listed()
    }

    fn target(&self, to: Option<Channel>) -> Result<Channel> {
        let listed = self.hearing()?;
        match to {
            Some(c) if listed.contains(&c) => Ok(c),
            Some(c) => Err(anyhow!("{c} is not on {}", self.list.display())),
            None => listed.into_iter().next().context("no channel is listed"),
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

    /// Ends only when accepting a local client fails, so systemd restarts the daemon with the socket's queue intact.
    pub async fn run(self: Arc<Self>, listener: UnixListener) -> Result<()> {
        tokio::select! {
            served = self.clone().serve(listener) => served,
            never = self.socket_mode() => match never {},
        }
    }

    /// Records the connection down for `why`, from now unless it was down already; whether it was live, and for how
    /// long it has been down.
    fn went_down(&self, why: String) -> (bool, Duration) {
        let mut state = self.state();
        let live = state.down.is_none();
        let since = state.down.as_ref().map_or_else(Instant::now, |d| d.0);
        state.down = Some((since, why));
        (live, since.elapsed())
    }

    async fn socket_mode(&self) -> Infallible {
        let mut told = false;
        loop {
            let ended = self.connection().await;
            let why =
                ended.as_ref().map_or_else(|e| format!("{e:#}"), |()| "Slack closed it".into());
            let (live, down) = self.went_down(why.clone());
            told &= !live;
            match ended {
                Ok(()) => note!("Socket Mode: {why}; reconnecting"),
                Err(_) if down >= self.bounds.down && !told => {
                    told = true;
                    fail!(
                        "Socket Mode: down for {}s, so every request is refused: {why}",
                        down.as_secs()
                    )
                }
                Err(_) => warn!("Socket Mode: {why}"),
            }
            tokio::time::sleep(self.bounds.reconnect).await;
        }
    }

    async fn connection(&self) -> Result<()> {
        let listed = self.listed()?;
        let opened = timeout(TIMEOUT, self.slack.connect()).await;
        let mut socket =
            opened.with_context(|| format!("no connection in {}s", TIMEOUT.as_secs()))??;
        self.catch_up(&listed).await?;
        self.state().down = None;
        let mut pinged = false;
        loop {
            let frame = match timeout(self.bounds.ping, socket.next()).await {
                Ok(Some(frame)) => frame?,
                Ok(None) => return Ok(()),
                Err(_) if !pinged => {
                    socket.send(Frame::Ping(Default::default())).await?;
                    pinged = true;
                    continue;
                }
                Err(_) => bail!("Slack answered no ping in {}s", self.bounds.ping.as_secs()),
            };
            pinged = false;
            let Frame::Text(text) = frame else { continue };
            let envelope: Envelope = serde_json::from_str(&text)?;
            if envelope.kind == "events_api" {
                match serde_json::from_value(envelope.payload["event"].clone()) {
                    Ok(raw) => self.store(&raw, &self.listed()?)?,
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
    }

    /// Each listed channel since the newest message held from it, the replies of threads with newer ones included.
    async fn catch_up(&self, listed: &[Channel]) -> Result<()> {
        for channel in listed {
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
                found.iter().try_for_each(|raw| self.store(raw, listed))
            };
            caught.await.with_context(|| format!("catching up on {channel}"))?;
        }
        Ok(())
    }

    async fn serve(self: Arc<Self>, listener: UnixListener) -> Result<()> {
        loop {
            let (stream, _) = listener.accept().await.context("accepting a local client")?;
            drop(tokio::spawn(self.clone().client(stream)));
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

    /// At least once: the message is taken only when the client confirms it printed it. Messages held already are
    /// handed over first; with none, a daemon that cannot hear Slack refuses the wait.
    async fn wait(&self, who: String, line: &mut Client) -> Result<()> {
        let _waiting = match self.claim(who) {
            Ok(waiting) => waiting,
            Err(why) => return line.send(&Reply::Refused(why)).await,
        };
        let message = loop {
            if let Some(m) = self.state().inbox.next().cloned() {
                break m;
            }
            if let Err(e) = self.hearing() {
                return line.send(&Reply::Refused(format!("{e:#}"))).await;
            }
            tokio::select! {
                () = self.arrived.notified() => {}
                () = tokio::time::sleep(self.bounds.recheck) => {}
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
