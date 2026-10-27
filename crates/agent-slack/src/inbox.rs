use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use text::text;

/// Slack's ids: a kind letter, then capitals and digits.
fn id(s: &str, kinds: &str, what: &str) -> Result<String> {
    let ok = s.len() >= 2
        && s.starts_with(|c| kinds.contains(c))
        && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    ensure!(ok, "{} is not a {what} id", s.escape_debug());
    Ok(s.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Channel(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct User(String);

/// A message's timestamp, which is also its id within its channel and, for a thread's first message, the thread's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ts(u64, u32);

text! {
    Channel(|s| id(s, "CGD", "channel").map(Channel), |c| c.0);
    User(|s| id(s, "UWB", "user").map(User), |u| u.0);
    Ts(|s| match s.split_once('.') {
        Some((secs, micros)) if micros.len() == 6 && [secs, micros].iter().all(|d| digits(d)) => {
            Ok(Ts(secs.parse()?, micros.parse()?))
        }
        _ => bail!("{} is not a Slack timestamp", s.escape_debug()),
    }, |t| format!("{}.{:06}", t.0, t.1));
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 19 && s.chars().all(|c| c.is_ascii_digit())
}

impl Ts {
    pub fn now() -> Ts {
        let since = std::time::UNIX_EPOCH.elapsed().unwrap_or_default();
        Ts(since.as_secs(), since.subsec_micros())
    }
}

pub type Key = (Channel, Ts);

/// A message from a person in a listed channel, as the inbox holds it and wait prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub channel: Channel,
    pub ts: Ts,
    pub thread_ts: Option<Ts>,
    pub user: User,
    pub text: String,
}

impl Message {
    pub fn key(&self) -> Key {
        (self.channel.clone(), self.ts)
    }
}

/// A message as Slack's events and history carry it; history leaves out the channel.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Raw {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub subtype: Option<String>,
    pub channel: Option<Channel>,
    pub user: Option<User>,
    pub bot_id: Option<String>,
    pub text: Option<String>,
    pub ts: Option<Ts>,
    pub thread_ts: Option<Ts>,
    pub reply_count: Option<u64>,
    pub latest_reply: Option<Ts>,
}

/// The checks: a plain new message or thread reply, in a listed channel, from a person other than the bot.
pub fn admit(raw: &Raw, listed: &[Channel], bot: &User) -> Option<Message> {
    let plain = matches!(raw.subtype.as_deref(), None | Some("thread_broadcast"));
    let user = raw.user.as_ref().filter(|u| *u != bot && raw.bot_id.is_none())?;
    let channel = raw.channel.as_ref().filter(|c| listed.contains(c))?;
    let ts = raw.ts?;
    (raw.kind.as_deref() == Some("message") && plain).then(|| Message {
        channel: channel.clone(),
        ts,
        thread_ts: raw.thread_ts.filter(|t| Some(*t) != raw.ts),
        user: user.clone(),
        text: raw.text.clone().unwrap_or_default(),
    })
}

/// The channel list: one id per line, blank lines skipped; the first is where send posts by default.
pub fn channels(list: &str) -> Result<Vec<Channel>> {
    list.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::parse).collect()
}

/// One line of the inbox file. `Mark` is where catch-up starts for a channel the inbox holds nothing from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    Message(Message),
    Taken(Key),
    Mark(Key),
}

#[derive(Debug, Default)]
pub struct Inbox {
    held: Vec<Message>,
    keys: HashSet<Key>,
    taken: HashSet<Key>,
    newest: HashMap<Channel, Ts>,
}

impl Inbox {
    pub fn replay(file: &str) -> Result<Inbox> {
        let mut inbox = Inbox::default();
        for line in file.lines() {
            inbox.apply(serde_json::from_str(line)?);
        }
        Ok(inbox)
    }

    pub fn apply(&mut self, entry: Entry) {
        let (channel, ts) = match entry {
            Entry::Message(m) => {
                let key = m.key();
                if self.keys.insert(key.clone()) {
                    self.held.push(m);
                }
                key
            }
            Entry::Taken(key) => {
                self.taken.insert(key);
                return;
            }
            Entry::Mark(key) => key,
        };
        let newest = self.newest.entry(channel).or_insert(ts);
        *newest = ts.max(*newest);
    }

    pub fn holds(&self, key: &Key) -> bool {
        self.keys.contains(key)
    }

    pub fn next(&self) -> Option<&Message> {
        self.held.iter().find(|m| !self.taken.contains(&m.key()))
    }

    pub fn newest(&self, channel: &Channel) -> Option<Ts> {
        self.newest.get(channel).copied()
    }
}

/// `body` in pieces of at most `max` characters, each cut after its last newline when it has one.
pub fn chunks(body: &str, max: usize) -> Vec<&str> {
    let mut rest = body;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let end = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
        let cut = match rest[..end].rfind('\n') {
            Some(nl) if end < rest.len() && nl > 0 => nl + 1,
            _ => end,
        };
        out.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    out
}

/// Slack escapes these three in message text.
pub fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}
