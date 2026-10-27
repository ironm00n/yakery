use std::process::ExitCode;

use clap::ArgGroup;
use tokio::time::Instant;

use super::claim::{Cursor, Scan, answered_in_sent, claim, open};
use private::{now, random::below};
use text::Address;

use crate::{
    address::Topic,
    jmap::wire,
    mail::{Addr, Body, Email, Keyword, Mail, Mark},
    out::{Asked, Outbox, is_receipt},
    prelude::*,
    show::{message, out, reminder, utc},
};

const RECHECK: Duration = Duration::from_secs(150);
const BACKOFF: Duration = Duration::from_secs(5);
// A wait that cannot finish a scan for this long gives up, so whoever runs it hears that mail is not arriving.
const STALE: Duration = Duration::from_secs(300);

// Every open inbox mail is matched on these alone, since Stalwart parses each message for an Email's attachments, and
// a few hundred of those outlast a request.
wire! {
    views {
        Addressed in "Email/get" {
            id: String = "id", thread: String = "threadId", to: Vec<Addr> = "to",
            cc: Vec<Addr> = "cc", mailboxes: HashMap<String, bool> = "mailboxIds",
        }
    }
    objects {}
}

#[derive(clap::Args)]
#[command(group(ArgGroup::new("watched").required(true).multiple(true).args(["topic", "thread"])))]
pub struct Args {
    /// Watch inbox mail to the subaddress <login>+<topic>.
    #[arg(long)]
    pub topic: Vec<Topic>,
    /// Watch inbox mail in the thread of this email.
    #[arg(long, value_name = "EMAIL_ID")]
    pub thread: Vec<String>,
    /// Give up after <n>[smhd], at most 30d; minutes when the unit is left out.
    #[arg(long, value_parser = timeout)]
    pub timeout: Option<Duration>,
}

pub fn timeout(s: &str) -> Result<Duration> {
    let malformed = || anyhow!("timeout \"{s}\" is not <n>[smhd]");
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let scales = [("s", 1), ("", 60), ("m", 60), ("h", 3600), ("d", 86400)];
    let scale = scales.iter().find(|(u, _)| *u == unit).ok_or_else(malformed)?.1;
    let secs = n.parse::<u64>().ok().and_then(|n| n.checked_mul(scale)).ok_or_else(malformed)?;
    ensure!(secs <= 30 * 86400, "timeout \"{s}\" is over 30d");
    Ok(Duration::from_secs(secs))
}

/// To and Cc are matched here rather than by the server, which answers them from its full-text index, built after
/// ingest.
pub fn covers(topics: &[Address], threads: &HashSet<String>, m: &Addressed) -> bool {
    let watched = |a: &Addr| a.email.parse().is_ok_and(|a| topics.contains(&a));
    threads.contains(&m.thread) || m.to.iter().chain(&m.cc).any(watched)
}

struct Watch<'a> {
    mail: &'a Mail,
    inbox: String,
    sent: String,
    topics: Vec<Address>,
    starts: Vec<String>,
    deadline: Option<Instant>,
    cursor: Cursor,
    lost: HashSet<String>,
    started: Instant,
    scanned: Option<Instant>,
}

impl Watch<'_> {
    fn filter(&self) -> Value {
        let rest = [Keyword::Answered, Keyword::Handled].map(|k| json!({ "notKeyword": k.name() }));
        json!({ "operator": "AND", "conditions": [{ "inMailbox": self.inbox }, rest[0], rest[1]] })
    }

    async fn threads(&mut self) -> Result<HashSet<String>> {
        let mut current = vec![];
        for start in &self.starts {
            match self.mail.current(start).await? {
                Some(now) => current.push(now),
                None if self.lost.insert(start.clone()) => {
                    note!("{start} is gone; no longer watching its thread")
                }
                None => {}
            }
        }
        Ok(self.mail.get::<Email>(&current).await?.into_iter().map(|m| m.thread).collect())
    }

    async fn scan(&mut self) -> Result<Scan> {
        let threads = self.threads().await?;
        let (mut cursor, this) = (self.cursor.clone(), &*self);
        let scan = cursor.check(
            async |since| this.mail.changes(since).await,
            async || this.everything().await,
            async |ids| this.triage(&threads, ids).await,
        );
        let scan = scan.await;
        self.cursor = cursor;
        scan
    }

    async fn triage(
        &self,
        threads: &HashSet<String>,
        mut ids: BTreeSet<String>,
    ) -> Result<(Scan, BTreeSet<String>)> {
        ids.extend(self.mail.members(&threads.iter().collect::<Vec<_>>()).await?);
        let (login, inbox) = (self.mail.login(), &self.inbox);
        let all: Vec<Addressed> = self.mail.get(&ids.into_iter().collect::<Vec<_>>()).await?;
        let watched = all
            .into_iter()
            .filter(|m| m.mailboxes.contains_key(inbox) && covers(&self.topics, threads, m));
        let found: Vec<Email> = self.mail.get(&watched.map(|m| m.id).collect::<Vec<_>>()).await?;
        let open: Vec<Email> = found.into_iter().filter(|m| open(login, m)).collect();
        let answered = self.answered_from_sent(&open).await?;
        let (remind, due): (Vec<Email>, Vec<Email>) = open
            .into_iter()
            .filter(|m| !answered.contains(&m.id))
            .partition(|m| m.has(Keyword::Seen));
        let pending = due.iter().map(|m| m.id.clone()).collect();
        Ok((Scan { due, remind }, pending))
    }

    async fn everything(&self) -> Result<(Vec<String>, String)> {
        let (state, mut ids) = (self.mail.state().await?, vec![]);
        if !self.topics.is_empty() {
            ids = self.mail.query(self.filter()).await?;
        }
        Ok((ids, state))
    }

    async fn answered_from_sent(&self, candidates: &[Email]) -> Result<HashSet<String>> {
        let threads: HashSet<&String> = candidates.iter().map(|m| &m.thread).collect();
        let rest = self.mail.members(&threads.into_iter().collect::<Vec<_>>()).await?;
        let rest: Vec<String> =
            rest.into_iter().filter(|i| !candidates.iter().any(|c| c.id == *i)).collect();
        let members: Vec<Email> = self.mail.get(&rest).await?;
        let answered =
            answered_in_sent(self.mail.login(), [&self.inbox, &self.sent], candidates, &members);
        let ids: Vec<String> = answered.iter().cloned().collect();
        let canonical = self.mail.canonical(&ids).await?;
        let marks: Vec<Mark> =
            canonical.into_iter().map(|id| (id, &[Keyword::Answered][..])).collect();
        self.mail.mark(&marks).await?;
        Ok(answered)
    }

    fn remaining(&self) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(Instant::now()))
    }

    async fn pause(&self, pause: Duration) -> bool {
        let left = self.remaining().unwrap_or(Duration::MAX);
        tokio::time::sleep(pause.min(left)).await;
        left > pause
    }

    async fn settle(&mut self, reminded: &mut HashSet<String>) -> Result<Option<Vec<Email>>> {
        let mut pushes = self.mail.pushes().await?;
        loop {
            let mail = self.mail;
            let (claimed, remind) = claim(mail, mail.login(), async || self.scan().await).await;
            let mut fresh: Vec<Email> = remind
                .into_iter()
                .filter(|m| !is_receipt(m) && !reminded.contains(&m.id))
                .collect();
            fresh.sort_by(|a, b| b.received.cmp(&a.received));
            for m in fresh.iter().take(20) {
                note!("{}", reminder(m));
            }
            if fresh.len() > 20 {
                note!("seen but unanswered: {} more", fresh.len() - 20);
            }
            reminded.extend(fresh.into_iter().map(|m| m.id));
            let claimed = claimed?;
            self.scanned = Some(Instant::now());
            if !claimed.is_empty() {
                return Ok(Some(claimed));
            }
            let wait = self.remaining().map_or(RECHECK, |left| left.min(RECHECK));
            if wait.is_zero() {
                return Ok(None);
            }
            match tokio::time::timeout(wait, pushes.next()).await {
                // Every session on the shared inbox rescans on the same push; spreading them keeps
                // Stalwart under its limit.
                Ok(Ok(())) => {
                    let spread = below(250).map_or(0, |n| n as u64 * 12);
                    tokio::time::sleep(Duration::from_millis(spread)).await
                }
                Ok(Err(e)) => return Err(e),
                Err(_) if self.remaining().is_some_and(|left| left.is_zero()) => return Ok(None),
                Err(_) => return Err(Failure::Idle.with("no push in a recheck period")),
            }
        }
    }

    async fn until_due(&mut self) -> Result<Option<Vec<Email>>> {
        let (mut backoff, mut reminded) = (BACKOFF, HashSet::new());
        loop {
            let before = self.scanned;
            let e = match self.settle(&mut reminded).await {
                Ok(wake) => return Ok(wake),
                Err(e) => e,
            };
            if self.scanned != before {
                backoff = BACKOFF;
            }
            let since = self.scanned.unwrap_or(self.started).elapsed();
            if since >= STALE {
                return Err(
                    e.context(format!("no scan finished in {} minutes", STALE.as_secs() / 60))
                );
            }
            let sign_in = |why: String| (backoff, true, format!("{why}; signing in again"));
            let (pause, reconnect, why) = match Failure::of(&e) {
                Failure::Idle => continue,
                Failure::Config | Failure::PasswordRefused => return Err(e),
                Failure::Dropped => {
                    (Duration::from_secs(1), false, format!("{e:#}; reopening the push stream"))
                }
                Failure::Busy => {
                    let why = "stalwart stays over its request limit; checking again";
                    (Duration::from_secs(30), false, why.into())
                }
                Failure::TokenRefused => sign_in("the access token expired".into()),
                Failure::Unprocessed | Failure::Transient => sign_in(format!("{e:#}")),
            };
            // The last attempt comes at the bound, so an outage shorter than it is waited out.
            let pause = pause.min(STALE - since);
            warn!("{why} in {}s", pause.as_secs());
            if !self.pause(pause).await {
                return match self.scanned {
                    Some(_) => Ok(None),
                    None => Err(e.context("the wait timed out before any scan finished")),
                };
            }
            if reconnect {
                backoff *= 2;
                match self.mail.reconnect().await {
                    Err(e) if Failure::of(&e).permanent() => return Err(e),
                    Err(e) => warn!("{e:#}"),
                    Ok(()) => note!("signed in again; waiting"),
                }
            }
        }
    }
}

pub async fn run(args: Args) -> Result<ExitCode> {
    watch(&Mail::open().await?, args).await
}

pub async fn watch(mail: &Mail, args: Args) -> Result<ExitCode> {
    let topics = args.topic.iter().map(|t| mail.login().address().subaddress(t));
    let topics = topics.collect::<Result<Vec<Address>>>()?;
    let deadline =
        args.timeout.map(|t| Instant::now().checked_add(t).context("timeout too far ahead"));
    let (inbox, sent, starts) =
        (mail.mailbox("inbox")?, mail.mailbox("sent")?, args.thread.clone());
    let (deadline, cursor, lost) = (deadline.transpose()?, Cursor::default(), HashSet::new());
    let (started, scanned) = (Instant::now(), None);
    let mut watch =
        Watch { mail, inbox, sent, topics, starts, deadline, cursor, lost, started, scanned };
    for start in &args.thread {
        ensure!(mail.current(start).await?.is_some(), "no email {start}");
    }
    let topics = watch.topics.iter().map(|a| a.to_string().to_lowercase());
    let watching: Vec<String> =
        topics.chain(args.thread.iter().map(|t| format!("the thread of {t}"))).collect();
    note!("waiting for {}", watching.join(", "));
    let wall = args.timeout.map(|t| now().saturating_add(t.as_secs() as i64));
    let Some(mut claimed) = watch.until_due().await? else {
        note!("no mail by {}", utc(wall.unwrap_or_else(now)));
        return Ok(ExitCode::from(2));
    };
    // Claimed already: if printing fails, the mail shows up in the next wait's reminders.
    claimed.sort_by(|a, b| a.received.cmp(&b.received));
    let bodies: Vec<Body> = mail.get(&claimed.iter().map(|m| &m.id).collect::<Vec<_>>()).await?;
    for m in &claimed {
        if let Some(body) = bodies.iter().find(|b| b.id == m.id) {
            out(&format!("{}\n", message(m, Some(body))))?;
        }
    }
    let mut outbox = None;
    for m in &claimed {
        match Asked::of(mail.login(), m) {
            Asked::Nothing => {}
            Asked::Declined(why) => note!("{why}"),
            Asked::Send(receipt) => {
                let outbox = match &outbox {
                    Some(outbox) => outbox,
                    None => outbox.insert(Outbox::open(mail).await?),
                };
                match receipt.send(outbox).await {
                    Ok(()) => note!("sent a receipt for {} to {}", m.id, receipt.to.to[0]),
                    Err(e) => warn!("the receipt for {} failed: {e:#}", m.id),
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
