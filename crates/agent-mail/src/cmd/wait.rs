use std::{borrow::Borrow, process::ExitCode};

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
    BACKOFF, RECHECK, REOPEN, STALE, SUBMISSION_KEY,
    address::{SessionId, Topic},
    authority::{self, Word},
    jmap::MAIL,
    listeners::{Leave, Listeners},
    mail::{Addr, Body, Current, Email, Keyword, Mail, Mark, names},
    out::{Asked, Outbox, is_receipt},
    prelude::*,
    show::{Answering, message, out, reminder, utc},
    store::{Config, Records, Store, Watched},
};

const UNSCANNED: &str = "the wait timed out before any scan finished";

macro_rules! exits {
    (timeout $timeout:literal, deferred $deferred:literal) => {
        const TIMED_OUT: u8 = $timeout;
        pub const DEFERRED: u8 = $deferred;
        pub const WAIT_HELP: &str = concat!(
            "Wait for watched mail from others that is unseen, unanswered and not done; claim it as seen, print it, and ",
            "send any read receipt it asks for. Exits ", $timeout, " at --timeout, but fails once no scan has finished ",
            "for 5 minutes (as built) or at a --timeout no scan finished before. Seen mail still unanswered is listed on ",
            "stderr. Without --json it exits ", $deferred, " at once when the mail-listener mod already waits for this ",
            "Claude Code session, recording the topics and threads given for that wait"
        );
    };
}

exits!(timeout 2, deferred 3);

// Every open inbox mail is matched on these alone, since Stalwart parses each message for an Email's attachments, and
// a few hundred of those outlast a request.
::jmap::view! {
    under MAIL {
        Addressed in "Email/get" {
            id: String = "id", thread: String = "threadId", to: Vec<Addr> = "to",
            cc: Vec<Addr> = "cc", mailboxes: HashMap<String, bool> = "mailboxIds",
        }
    }
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

#[derive(clap::Args)]
pub struct Subscriptions {
    #[arg(long, value_name = "SESSION_ID")]
    pub session: Option<SessionId>,
    /// Print the record as one JSON object: its `topics`, and its `threads` as the mail ids that name them.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
#[command(group(ArgGroup::new("watched").required(true).multiple(true).args(["topic", "thread"])))]
pub struct Unsubscribe {
    #[command(flatten)]
    pub watching: Watching,
    #[arg(long, value_name = "SESSION_ID")]
    pub session: Option<SessionId>,
}

/// Records `given` for `id`'s `wait --session` and says what that session now watches.
fn record(records: &Records, id: &SessionId, given: &Watching) -> Result<String> {
    records.record(id, given.topic.clone(), given.thread.clone())?;
    watches(records, id)
}

pub fn listed(Watched { topics, threads }: &Watched) -> String {
    let topics: Vec<String> = topics.iter().map(Topic::to_string).collect();
    let threads: Vec<&str> = threads.iter().map(String::as_str).collect();
    format!("topics [{}] and the threads of [{}]", topics.join(", "), threads.join(", "))
}

/// What `id`'s sends and subscribes recorded for its `wait --session`.
pub fn watches(records: &Records, id: &SessionId) -> Result<String> {
    Ok(format!("session {id} watches {}", listed(&records.of(id)?)))
}

/// Takes `given` out of `id`'s record, warning of what it did not hold, and says what that session still watches.
pub fn forget(records: &Records, id: &SessionId, given: Watching) -> Result<String> {
    let absent = records.forget(id, given.topic, given.thread)?;
    if !absent.topics.is_empty() || !absent.threads.is_empty() {
        warn!("session {id} did not watch {}", listed(&absent));
    }
    watches(records, id)
}

/// The session `--session` names, else this one.
fn target(named: Option<SessionId>, command: &str) -> Result<SessionId> {
    let this =
        || format!("without --session, {command} this Claude Code session's, and this is none");
    match named {
        Some(id) => Ok(id),
        None => session().with_context(this)?.session_id.parse(),
    }
}

/// Records what to watch for this Claude Code session without sending, for its `wait --session`.
pub async fn subscribe(args: Subscribe) -> Result<ExitCode> {
    let session =
        session().context("subscribe records for a Claude Code session, and this is none")?;
    let id: SessionId = session.session_id.parse()?;
    succeed!("{}", record(&Store::new(&Config::load()?.login)?.records()?, &id, &args.watching)?)
}

/// Says what a Claude Code session's `wait --session` watches, this one's unless --session names another.
pub async fn subscriptions(args: Subscriptions) -> Result<ExitCode> {
    let id = target(args.session, "subscriptions lists")?;
    succeed!("{}", subscribed(&Store::new(&Config::load()?.login)?.records()?, &id, args.json)?)
}

/// What `subscriptions` prints for `id`: its record as JSON, or in words.
pub fn subscribed(records: &Records, id: &SessionId, json: bool) -> Result<String> {
    if json { Ok(serde_json::to_string(&records.of(id)?)?) } else { watches(records, id) }
}

/// Takes topics and threads out of what a Claude Code session's `wait --session` watches, this one's unless --session
/// names another.
pub async fn unsubscribe(args: Unsubscribe) -> Result<ExitCode> {
    let id = target(args.session, "unsubscribe edits")?;
    succeed!("{}", forget(&Store::new(&Config::load()?.login)?.records()?, &id, args.watching)?)
}

/// While `session`'s mail-listener mod is live, records `given` for its wait and says so: that wait hands the session
/// all its mail, so a second one would only race it.
pub fn deferred(
    store: &Store,
    session: &SessionId,
    given: &Watching,
    now: i64,
) -> Result<Option<String>> {
    let Some(listener) = store.listeners()?.listening(session, now)? else { return Ok(None) };
    let watches = record(&store.records()?, session, given)?;
    Ok(Some(format!(
        "the mail-listener mod (pid {}) already waits for this session and hands its mail in as prompts; {watches}",
        listener.pid
    )))
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

/// At least `least`, and up to `spread` more, drawn at random.
fn jittered(least: Duration, spread: Duration) -> Duration {
    let ms = spread.as_millis().max(1) as usize;
    least + Duration::from_millis(below(ms).unwrap_or(0) as u64)
}

pub fn covers(topics: &[Address], threads: &HashSet<String>, m: &Addressed) -> bool {
    threads.contains(&m.thread) || names(topics, m.to.iter().chain(&m.cc))
}

struct Watch<'a> {
    mail: &'a Mail,
    inbox: String,
    sent: String,
    given: Watching,
    session: Option<(Records, SessionId)>,
    /// A listener's: where it beats for its session after each scan.
    beat: Option<(Listeners, SessionId)>,
    /// Any other wait's: the listeners it leaves Owen's word to.
    yields: Option<Listeners>,
    topics: Vec<Address>,
    starts: Vec<String>,
    deadline: Option<Instant>,
    /// Kept with the topics and threads it checked for: a whole check reads the watched threads' members, so a set
    /// that changed starts over in full.
    cursor: Cursor<(Vec<Address>, Vec<String>)>,
    /// Each watched mail's thread, resolved once, since a mail never changes thread.
    thread_of: HashMap<String, String>,
    lost: HashSet<String>,
    started: Instant,
    scanned: Option<Instant>,
    reminded: HashSet<String>,
    /// A listener's: each mail is shown with whether it has Owen's authority.
    json: bool,
}

/// What showing claimed mail needs from the server, fetched before the claim.
#[derive(Default)]
struct Ready {
    bodies: Vec<Body>,
    members: Vec<Email>,
    verdicts: HashMap<String, Result<Word, String>>,
}

impl Ready {
    async fn fetch(mail: &Mail, ids: &[Current], topics: &[Address], json: bool) -> Result<Ready> {
        let (bodies, emails): (Vec<Body>, Vec<Email>) =
            (mail.get(ids).await?, mail.get(ids).await?);
        let threads: BTreeSet<&String> = emails.iter().map(|m| &m.thread).collect();
        let members =
            mail.get(&mail.members(&threads.into_iter().collect::<Vec<_>>()).await?).await?;
        let mut verdicts = HashMap::new();
        for m in emails.iter().filter(|_| json) {
            verdicts.insert(m.id.clone(), authority(mail, m, topics).await?);
        }
        Ok(Ready { bodies, members, verdicts })
    }
}

impl Watch<'_> {
    /// `due` less the mail with Owen's authority that a live listener watches: that listener hands it over as his.
    async fn unyielded(&self, due: Vec<Email>) -> Result<Vec<Email>> {
        let Some(listeners) = &self.yields else { return Ok(due) };
        let watched = listeners.watched(now())?;
        if watched.is_empty() {
            return Ok(due);
        }
        let mut kept = vec![];
        for m in due {
            if !names(&watched, m.to.iter().chain(&m.cc))
                || authority(self.mail, &m, &watched).await?.is_err()
            {
                kept.push(m);
            }
        }
        Ok(kept)
    }

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
        let unresolved: Vec<String> = (self.starts.iter())
            .filter(|s| !self.thread_of.contains_key(*s) && !self.lost.contains(*s))
            .cloned()
            .collect();
        let mut current = HashMap::new();
        for (start, now) in unresolved.iter().zip(self.mail.currents(&unresolved).await?) {
            match now {
                Some(now) => {
                    current.insert(now.as_str().to_owned(), start.clone());
                }
                None => {
                    self.lost.insert(start.clone());
                    note!("{start} is gone; no longer watching its thread")
                }
            }
        }
        let ids: Vec<&String> = current.keys().collect();
        for m in self.mail.get::<Addressed>(&ids).await? {
            if let Some(start) = current.get(&m.id) {
                self.thread_of.insert(start.clone(), m.thread);
            }
        }
        Ok(self.starts.iter().filter_map(|s| self.thread_of.get(s).cloned()).collect())
    }

    async fn scan(&mut self) -> Result<Scan> {
        self.refresh()?;
        let threads = self.threads().await?;
        let (mut cursor, this) = (self.cursor.clone(), &*self);
        let scan = cursor.check(
            (this.topics.clone(), this.starts.clone()),
            async |since| this.mail.changes(since).await,
            async || this.everything().await,
            async |ids, whole| this.triage(&threads, ids, whole).await,
        );
        let scan = scan.await;
        self.cursor = cursor;
        scan
    }

    /// Only a whole check reads the watched threads' members: a reply after the cursor's state is among the changes.
    async fn triage(
        &self,
        threads: &HashSet<String>,
        mut ids: BTreeSet<String>,
        whole: bool,
    ) -> Result<(Scan, BTreeSet<String>)> {
        if whole {
            ids.extend(self.mail.members(&threads.iter().collect::<Vec<_>>()).await?);
        }
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
        // Mail left to a listener stays pending, so it is claimed here once that listener is gone.
        let pending = due.iter().map(|m| m.id.clone()).collect();
        let due = self.unyielded(due).await?;
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

    async fn settle(&mut self) -> Result<Option<(Vec<Email>, Ready)>> {
        let mut pushes = self.mail.pushes().await?;
        loop {
            let (mail, topics, json) = (self.mail, self.topics.clone(), self.json);
            let ready = async |ids: &[Current]| Ready::fetch(mail, ids, &topics, json).await;
            let (claimed, remind) =
                claim(mail, mail.login(), async || self.scan().await, ready).await;
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
            if let Some((listeners, session)) = &self.beat
                && let Err(e) = listeners.beat(session, &self.topics, &self.starts, now())
            {
                warn!(
                    "{e:#}; this listener's scan goes unrecorded, so other waits may claim Owen's word"
                );
            }
            if !claimed.0.is_empty() {
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

    async fn until_due(&mut self) -> Result<Option<(Vec<Email>, Ready)>> {
        let (mut backoff, mut reopen) = (BACKOFF, REOPEN);
        loop {
            let before = self.scanned;
            let e = match self.settle().await {
                Ok(wake) => return Ok(wake),
                Err(e) => e,
            };
            if self.scanned != before {
                (backoff, reopen) = (BACKOFF, REOPEN);
            }
            let since = self.scanned.unwrap_or(self.started).elapsed();
            if since >= STALE {
                return Err(e.context(format!("no scan finished in {}s", STALE.as_secs())));
            }
            let sign_in = |why: String| {
                (jittered(backoff, backoff), true, format!("{why}; signing in again"))
            };
            let (pause, reconnect, why) = match Failure::of(&e) {
                Failure::Idle => continue,
                Failure::Config | Failure::PasswordRefused => return Err(e),
                // Every wait on a host loses its stream when Stalwart restarts; drawn apart, their scans don't
                // arrive together, and the spread doubles while the drops come in a row.
                Failure::Dropped => {
                    let pause = jittered(Duration::from_secs(1), reopen);
                    reopen = (reopen * 2).min(STALE);
                    (pause, false, format!("{e:#}; reopening the push stream"))
                }
                Failure::Busy(after) => {
                    let why = "stalwart stays over its request limit; checking again";
                    let least = after.unwrap_or_default().max(Duration::from_secs(30));
                    (jittered(least, REOPEN), false, why.into())
                }
                Failure::TokenRefused => sign_in("the access token expired".into()),
                Failure::Unprocessed | Failure::Transient => sign_in(format!("{e:#}")),
            };
            // The last attempt comes at the bound, so an outage shorter than it is waited out.
            let pause = pause.min(STALE - since);
            warn!("{why} in {:.1}s", pause.as_secs_f64());
            if !self.pause(pause).await {
                return match self.scanned {
                    Some(_) => Ok(None),
                    None => Err(e.context(UNSCANNED)),
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
    let caller = session().filter(|_| !args.json).and_then(|s| s.session_id.parse().ok());
    // Its mod watches what this session records, so only a wait on the same or less stands down.
    if let Some(caller) = caller.filter(|c| args.session.as_ref().is_none_or(|s| s == c))
        && let Some(line) =
            deferred(&Store::new(&Config::load()?.login)?, &caller, &args.watching, now())?
    {
        out(&line)?;
        return Ok(ExitCode::from(DEFERRED));
    }
    watch(Mail::open(), args).await
}

/// The wait on the mail `opening` opens, whose deadline covers the opening: signing in, and the auth lock.
pub async fn watch<M: Borrow<Mail>>(
    opening: impl Future<Output = Result<M>>,
    args: Args,
) -> Result<ExitCode> {
    let deadline =
        args.timeout.map(|t| Instant::now().checked_add(t).context("timeout too far ahead"));
    let (deadline, cursor) = (deadline.transpose()?, Cursor::default());
    let wall = args.timeout.map(|t| now().saturating_add(t.as_secs() as i64));
    let opened = match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, opening)
            .await
            .map_err(|_| anyhow!("opening the mail took all of --timeout").context(UNSCANNED))?,
        None => opening.await,
    }?;
    let mail = opened.borrow();
    let (inbox, sent) = (mail.mailbox("inbox")?, mail.mailbox("sent")?);
    let store = &mail.conn.auth.store;
    let listening = args.session.clone().filter(|_| args.json);
    let _leave =
        listening.clone().map(|id| anyhow::Ok(Leave(store.listeners()?, id))).transpose()?;
    let beat = listening.map(|id| anyhow::Ok((store.listeners()?, id))).transpose()?;
    let yields = (!args.json).then(|| store.listeners()).transpose()?;
    let records = |id| anyhow::Ok((store.records()?, id));
    let session = args.session.map(records).transpose()?;
    let (given, json) = (args.watching, args.json);
    let mut watch = Watch {
        mail,
        inbox,
        sent,
        given,
        session,
        beat,
        yields,
        deadline,
        cursor,
        topics: vec![],
        starts: vec![],
        thread_of: HashMap::new(),
        lost: HashSet::new(),
        started: Instant::now(),
        scanned: None,
        reminded: HashSet::new(),
        json,
    };
    for start in &watch.given.thread {
        ensure!(mail.current(start).await?.is_some(), "no email {start}");
    }
    watch.refresh()?;
    let topics = watch.topics.iter().map(|a| a.to_string().to_lowercase());
    let threads = watch.starts.iter().map(|t| format!("the thread of {t}"));
    let recorded = watch.session.iter().map(|(_, s)| format!("what session {s} records"));
    note!("waiting for {}", topics.chain(threads).chain(recorded).collect::<Vec<_>>().join(", "));
    loop {
        let Some((mut claimed, mut ready)) = watch.until_due().await? else {
            note!("no mail by {}", utc(wall.unwrap_or_else(now)));
            return Ok(ExitCode::from(TIMED_OUT));
        };
        claimed.sort_by(|a, b| a.received.cmp(&b.received));
        let unchecked = || Err("its authority was not checked".to_owned());
        for m in &claimed {
            if let Some(body) = ready.bodies.iter().find(|b| b.id == m.id) {
                out(&match json {
                    true => {
                        let verdict = ready.verdicts.remove(&m.id).unwrap_or_else(unchecked);
                        line(m, body, &verdict, &ready.members)
                    }
                    false => {
                        let answering = Answering::of(m, &m.in_reply_to, &ready.members);
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
async fn authority(mail: &Mail, m: &Email, topics: &[Address]) -> Result<Result<Word, String>> {
    Ok(mail.message(m).await?.and_then(|raw| authority::check(SUBMISSION_KEY, &raw, topics, now())))
}

/// One mail as one line, whatever its text holds; with Owen's authority, its addressing, subject, text and the mail
/// of `thread` it answers are the ones he signed.
pub fn line(m: &Email, body: &Body, verdict: &Result<Word, String>, thread: &[Email]) -> String {
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
