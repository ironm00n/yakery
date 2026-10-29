use std::process::ExitCode;

use clap::ArgGroup;
use tokio::time::Instant;

use super::{
    claim::{Cursor, Scan, answered_in_sent, claim, open},
    send::session,
    succeed,
};
use private::{now, random::below};
use text::Address;

use crate::{
    BACKOFF, RECHECK, STALE, SUBMISSION_KEY,
    address::{SessionId, Topic},
    authority::{self, Signed},
    jmap::wire,
    mail::{Addr, Body, Email, Keyword, Mail, Mark},
    out::{Asked, Outbox, is_receipt},
    prelude::*,
    show::{Answering, message, out, reminder, utc},
    store::{Config, Records, Store, Watched},
};

const MAX_MESSAGE_BYTES: usize = 32 << 20;

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

#[derive(clap::Args, Default)]
pub struct Watching {
    /// Watch inbox mail to the subaddress <login>+<topic>.
    #[arg(long)]
    pub topic: Vec<Topic>,
    /// Watch inbox mail in the thread of this email.
    #[arg(long, value_name = "EMAIL_ID")]
    pub thread: Vec<String>,
}

#[derive(clap::Args)]
#[command(group(ArgGroup::new("watched").required(true).multiple(true).args(["topic", "thread", "session"])))]
pub struct Args {
    #[command(flatten)]
    pub watching: Watching,
    /// Also watch what this Claude Code session's sends and subscribes recorded, read again on every check.
    #[arg(long, value_name = "SESSION_ID")]
    pub session: Option<SessionId>,
    /// Give up after <n>[smhd], at most 30d; minutes when the unit is left out.
    #[arg(long, value_parser = timeout)]
    pub timeout: Option<Duration>,
    /// Keep waiting after each delivery, and print each mail as one JSON object on a line of its own, its `authority`
    /// true only for mail whose Submission-Signature makes it Owen's word, `answers` the mail of its thread it
    /// answers and `newest` the newest one before it from anyone else when that is another.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
#[command(group(ArgGroup::new("watched").required(true).multiple(true).args(["topic", "thread"])))]
pub struct Subscribe {
    #[command(flatten)]
    pub watching: Watching,
}

/// Records what to watch for this Claude Code session without sending, for its `wait --session`.
pub async fn subscribe(args: Subscribe) -> Result<ExitCode> {
    let session =
        session().context("subscribe records for a Claude Code session, and this is none")?;
    let id: SessionId = session.session_id.parse()?;
    let Watching { topic, thread } = args.watching;
    let records = Store::new(&Config::load()?.login)?.records()?;
    records.record(&id, topic, thread)?;
    let Watched { topics, threads } = records.of(&id)?;
    let topics: Vec<String> = topics.iter().map(Topic::to_string).collect();
    let threads: Vec<&str> = threads.iter().map(String::as_str).collect();
    succeed!(
        "session {id} watches topics [{}] and the threads of [{}]",
        topics.join(", "),
        threads.join(", ")
    )
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
    given: Watching,
    session: Option<(Records, SessionId)>,
    topics: Vec<Address>,
    starts: Vec<String>,
    deadline: Option<Instant>,
    cursor: Cursor,
    lost: HashSet<String>,
    started: Instant,
    scanned: Option<Instant>,
    reminded: HashSet<String>,
}

impl Watch<'_> {
    fn refresh(&mut self) -> Result<()> {
        let recorded =
            self.session.as_ref().map(|(r, id)| r.of(id)).transpose()?.unwrap_or_default();
        let login = self.mail.login().address();
        let topics: BTreeSet<&Topic> = self.given.topic.iter().chain(&recorded.topics).collect();
        self.topics = topics.into_iter().map(|t| login.subaddress(t)).collect::<Result<_>>()?;
        let starts: BTreeSet<&String> = self.given.thread.iter().chain(&recorded.threads).collect();
        self.starts = starts.into_iter().cloned().collect();
        Ok(())
    }

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
        self.refresh()?;
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

    async fn settle(&mut self) -> Result<Option<Vec<Email>>> {
        let mut pushes = self.mail.pushes().await?;
        loop {
            let mail = self.mail;
            let (claimed, remind) = claim(mail, mail.login(), async || self.scan().await).await;
            let mut fresh: Vec<Email> = remind
                .into_iter()
                .filter(|m| !is_receipt(m) && !self.reminded.contains(&m.id))
                .collect();
            fresh.sort_by(|a, b| b.received.cmp(&a.received));
            for m in fresh.iter().take(20) {
                note!("{}", reminder(m));
            }
            if fresh.len() > 20 {
                note!("seen but unanswered: {} more", fresh.len() - 20);
            }
            self.reminded.extend(fresh.into_iter().map(|m| m.id));
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
        let mut backoff = BACKOFF;
        loop {
            let before = self.scanned;
            let e = match self.settle().await {
                Ok(wake) => return Ok(wake),
                Err(e) => e,
            };
            if self.scanned != before {
                backoff = BACKOFF;
            }
            let since = self.scanned.unwrap_or(self.started).elapsed();
            if since >= STALE {
                return Err(e.context(format!("no scan finished in {}s", STALE.as_secs())));
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
    let deadline =
        args.timeout.map(|t| Instant::now().checked_add(t).context("timeout too far ahead"));
    let (inbox, sent) = (mail.mailbox("inbox")?, mail.mailbox("sent")?);
    let (deadline, cursor) = (deadline.transpose()?, Cursor::default());
    let records = |id| Ok::<_, anyhow::Error>((mail.conn.auth.store.records()?, id));
    let session = args.session.map(records).transpose()?;
    let (given, json) = (args.watching, args.json);
    let mut watch = Watch {
        mail,
        inbox,
        sent,
        given,
        session,
        deadline,
        cursor,
        topics: vec![],
        starts: vec![],
        lost: HashSet::new(),
        started: Instant::now(),
        scanned: None,
        reminded: HashSet::new(),
    };
    for start in &watch.given.thread {
        ensure!(mail.current(start).await?.is_some(), "no email {start}");
    }
    watch.refresh()?;
    let topics = watch.topics.iter().map(|a| a.to_string().to_lowercase());
    let threads = watch.starts.iter().map(|t| format!("the thread of {t}"));
    let recorded = watch.session.iter().map(|(_, s)| format!("what session {s} records"));
    note!("waiting for {}", topics.chain(threads).chain(recorded).collect::<Vec<_>>().join(", "));
    let wall = args.timeout.map(|t| now().saturating_add(t.as_secs() as i64));
    loop {
        let Some(mut claimed) = watch.until_due().await? else {
            note!("no mail by {}", utc(wall.unwrap_or_else(now)));
            return Ok(ExitCode::from(2));
        };
        // Claimed already: if printing fails, the mail shows up in the next wait's reminders.
        claimed.sort_by(|a, b| a.received.cmp(&b.received));
        let bodies: Vec<Body> =
            mail.get(&claimed.iter().map(|m| &m.id).collect::<Vec<_>>()).await?;
        let threads: BTreeSet<&String> = claimed.iter().map(|m| &m.thread).collect();
        let members: Vec<Email> =
            mail.get(&mail.members(&threads.into_iter().collect::<Vec<_>>()).await?).await?;
        for m in &claimed {
            if let Some(body) = bodies.iter().find(|b| b.id == m.id) {
                out(&match json {
                    true => line(m, body, &authority(mail, m, &watch.topics).await?, &members),
                    false => {
                        let answering = Answering::of(m, &m.in_reply_to, &members);
                        format!("{}\n", message(m, Some(body), answering))
                    }
                })?;
            }
        }
        receipts(mail, &claimed).await?;
        if !json {
            return Ok(ExitCode::SUCCESS);
        }
    }
}

/// Whether `m` is Owen's word, read from the message as Stalwart holds it.
async fn authority(mail: &Mail, m: &Email, topics: &[Address]) -> Result<Result<Signed, String>> {
    Ok(match mail.download(&m.blob, MAX_MESSAGE_BYTES).await? {
        Some(raw) => authority::check(SUBMISSION_KEY, &raw, topics, now()),
        None => Err(format!(
            "the message is over {} MiB, so its signature goes unchecked",
            MAX_MESSAGE_BYTES >> 20
        )),
    })
}

/// One mail as one line, whatever its text holds; with Owen's authority, its addressing, subject, text and the mail
/// of `thread` it answers are the ones he signed.
pub fn line(m: &Email, body: &Body, verdict: &Result<Signed, String>, thread: &[Email]) -> String {
    let values: Vec<_> = body.text.iter().filter_map(|p| body.values.get(&p.part)).collect();
    let emails = |l: &[Addr]| l.iter().map(|a| a.email.clone()).collect::<Vec<_>>();
    let (from, to, cc, subject, text, truncated, reason) = match verdict {
        Ok(s) => {
            let why = "Stalwart signed it as sent from Owen's account".to_owned();
            (
                s.from.clone(),
                s.to.clone(),
                s.cc.clone(),
                s.subject.clone(),
                s.text.clone(),
                false,
                why,
            )
        }
        Err(why) => (
            m.from.first().map_or_else(String::new, |a| a.email.clone()),
            emails(&m.to),
            emails(&m.cc),
            m.subject.clone(),
            values.iter().map(|v| v.value.as_str()).collect::<Vec<_>>().join("\n"),
            values.iter().any(|v| v.truncated),
            why.clone(),
        ),
    };
    let in_reply_to = verdict.as_ref().map_or(&m.in_reply_to, |s| &s.in_reply_to);
    let [answers, newest] = Answering::of(m, in_reply_to, thread).json();
    json!({
        "id": m.id, "threadId": m.thread, "receivedAt": m.received, "from": from, "to": to, "cc": cc,
        "subject": subject, "text": text, "truncated": truncated, "authority": verdict.is_ok(),
        "authorityReason": reason, "approval": verdict.as_ref().ok().map(|s| &s.signature),
        "answers": answers, "newest": newest,
    })
    .to_string()
}

async fn receipts(mail: &Mail, claimed: &[Email]) -> Result<()> {
    let mut outbox = None;
    for m in claimed {
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
    Ok(())
}
