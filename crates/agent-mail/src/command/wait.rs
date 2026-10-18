//! Blocks until watched mail waits on us, claims it with `$seen`, and prints it.
//!
//! What waits is read from server state rather than derived from a scan: inbox mail to a watched topic or in a
//! watched thread that has none of `$seen`, `$answered` and `$handled`, and is not ours. `send --reply` sets
//! `$answered`; a reply sent by another tool counts when a mail of ours in Sent names the mail in In-Reply-To.

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashSet},
    str::FromStr,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use clap::ArgGroup;
use futures_util::StreamExt;
use jmap_client::{
    core::query::Filter,
    email::{Email, Property as P, query},
    mailbox::Role,
};
use tokio::time::Instant;

use crate::{
    address::MessageId,
    address::Topic,
    auth::store::Held,
    auth::{Auth, is_permanent, store::now},
    config::Config,
    identity::Login,
    mail::{AGENT_MAIL, ANSWERED, HANDLED, MDN_SENT, Mail, NeedsSignIn, SEEN, busy, has_keyword},
    mdn::{self, Asked},
    outbox::Outbox,
    random::Draws,
    show::{MAX_BODY_BYTES, addresses, id, message, out, plain, received, utc, warn},
};

const RECHECK: Duration = Duration::from_secs(150);
const BACKOFF_START: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
const TIMEOUT_MAX: Duration = Duration::from_secs(30 * 86400);
const CLAIM_ATTEMPTS: usize = 20;
const REMINDERS_SHOWN: usize = 20;

#[derive(clap::Args)]
#[command(group(ArgGroup::new("watched").required(true).multiple(true).args(["topic", "thread"])))]
pub struct Args {
    /// Watch inbox mail to the subaddress <login>+<topic>.
    #[arg(long)]
    topic: Vec<Topic>,
    /// Watch inbox mail in the thread of this email.
    #[arg(long, value_name = "EMAIL_ID")]
    thread: Vec<String>,
    /// Give up after <n>[smhd], at most 30d; minutes when the unit is left out.
    #[arg(long)]
    timeout: Option<Timeout>,
}

#[derive(Clone, Debug)]
struct Timeout(Duration);

impl FromStr for Timeout {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let malformed = || format!("timeout \"{s}\" is not <n>[smhd]");
        let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (n, unit) = s.split_at(split);
        let scale: u64 = match unit {
            "s" => 1,
            "" | "m" => 60,
            "h" => 3600,
            "d" => 86400,
            _ => bail!(malformed()),
        };
        let secs = n
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_mul(scale))
            .with_context(malformed)?;
        let timeout = Duration::from_secs(secs);
        ensure!(timeout <= TIMEOUT_MAX, "timeout \"{s}\" is over 30d");
        Ok(Timeout(timeout))
    }
}

struct Watched {
    /// Lowercased.
    topics: Vec<String>,
    /// Emails whose threads are watched. Stalwart merges threads after ingest and drops the losing thread id, so
    /// the thread of each is looked up again on every check.
    starts: Vec<String>,
}

impl Watched {
    fn covers(&self, threads: &HashSet<String>, m: &Email) -> bool {
        m.thread_id().is_some_and(|t| threads.contains(t))
            || addresses(m.to())
                .chain(addresses(m.cc()))
                .any(|a| self.topics.contains(&a.to_lowercase()))
    }
}

fn ours(login: &Login, m: &Email) -> bool {
    addresses(m.from()).any(|a| login.owns(a))
}

/// Inbox mail still needing someone: no `$answered` or `$handled`, and not from us.
fn open(login: &Login, m: &Email) -> bool {
    !ours(login, m) && !has_keyword(m, ANSWERED) && !has_keyword(m, HANDLED)
}

/// Candidates that a reply from another tool answers: a mail of ours in Sent, neither a receipt nor tagged
/// `$agentmail` (agent-mail's own replies answer only through `$answered`), whose In-Reply-To names exactly one
/// id that exactly one inbox mail of the thread carries, since senders choose their Message-IDs.
/// `members` holds the rest of the candidates' threads.
fn answered_in_sent(
    login: &Login,
    inbox: &str,
    sent: &str,
    candidates: &[Email],
    members: &[Email],
) -> HashSet<String> {
    let thread_inbox: Vec<&Email> = candidates
        .iter()
        .chain(members)
        .filter(|m| m.mailbox_ids().contains(&inbox))
        .collect();
    let carriers = |thread: Option<&str>, reference: &MessageId| -> Vec<&str> {
        thread_inbox
            .iter()
            .filter(|m| m.thread_id() == thread)
            .filter(|m| {
                MessageId::all_valid(m.message_id().unwrap_or_default()).contains(reference)
            })
            .map(|m| id(m))
            .collect()
    };
    let mut answered = HashSet::new();
    for r in members {
        let other_tool = r.mailbox_ids().contains(&sent)
            && ours(login, r)
            && !mdn::is_receipt(r)
            && !has_keyword(r, AGENT_MAIL);
        let references = MessageId::all_valid(r.in_reply_to().unwrap_or_default());
        if let (true, [reference]) = (other_tool, references.as_slice())
            && let [only] = carriers(r.thread_id(), reference).as_slice()
        {
            answered.insert((*only).to_owned());
        }
    }
    let candidate_ids: HashSet<&str> = candidates.iter().map(id).collect();
    answered.retain(|a| candidate_ids.contains(a.as_str()));
    answered
}

fn triage_properties() -> Vec<P> {
    [
        P::ThreadId,
        P::ReceivedAt,
        P::From,
        P::To,
        P::Cc,
        P::Subject,
        P::Keywords,
        P::MailboxIds,
        P::MessageId,
    ]
    .into_iter()
    .chain(mdn::properties())
    .collect()
}

/// The server side of a claim, so the race between sessions can be tested.
trait Claims {
    /// Watched inbox mail with none of the three keywords, minus mail answered from Sent.
    async fn candidates(&self) -> Result<Vec<Email>>;
    /// The current id of each, once per email: after a thread merge an old id and a new one name the same
    /// email, and two updates to one email in a single Email/set fail it whole.
    async fn canonical(&self, ids: &[String]) -> Result<Vec<String>>;
    /// Makes the session's token good before a lock is taken, so nothing signs in while holding it.
    async fn ready(&self) -> Result<()>;
    /// Held while a claim re-reads and marks mail, so the waits of one host claim one at a time.
    async fn exclusive(&self) -> Result<Held>;
    /// The emails as they are now, with the account's Email state. Under the lock: never signs in.
    async fn snapshot(&self, ids: &[String]) -> Result<(String, Vec<Email>)>;
    /// Adds keywords if the state is unchanged; `false` when another change came first. Under the lock: never
    /// signs in.
    async fn mark_if(&self, state: &str, marks: &[(&str, Vec<&str>)]) -> Result<bool>;
}

/// Claims the waiting mail with `$seen`, and `$mdnsent` where a receipt is asked for; only what this session
/// claimed is returned. Receipts that reach us are claimed silently and not returned.
///
/// Exclusive between the waits of one host, which re-read and mark under the host's claims lock. Not across hosts:
/// Stalwart checks `ifInState` against the state it loaded when the request began, and adding a keyword that is
/// already set still reports the email updated, so waits on two hosts can both claim the same mail.
async fn claim(server: &impl Claims, login: &Login) -> Result<Vec<Email>> {
    for _ in 0..CLAIM_ATTEMPTS {
        let candidates = server.candidates().await?;
        let ids: Vec<String> = candidates.iter().map(|m| id(m).to_owned()).collect();
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let ids = server.canonical(&ids).await?;
        server.ready().await?;
        match claim_locked(server, login, &ids).await {
            Ok(Some(claimed)) => return Ok(claimed),
            Ok(None) => {}
            Err(e) if e.is::<NeedsSignIn>() => {}
            Err(e) => return Err(e),
        }
    }
    bail!("other changes to the inbox kept beating the claim, {CLAIM_ATTEMPTS} times in a row")
}

fn deliverable(claimed: Vec<Email>) -> Vec<Email> {
    claimed
        .into_iter()
        .filter(|m| !mdn::is_receipt(m))
        .collect()
}

/// `None` when another change came first. A mark can land even though its call failed: whatever now carries
/// `$seen` that lacked it in the snapshot is taken as claimed, so it is printed rather than lost.
async fn claim_locked(
    server: &impl Claims,
    login: &Login,
    ids: &[String],
) -> Result<Option<Vec<Email>>> {
    let _held = server.exclusive().await?;
    let (state, fresh) = server.snapshot(ids).await?;
    let fresh: Vec<Email> = fresh
        .into_iter()
        .filter(|m| open(login, m) && !has_keyword(m, SEEN))
        .collect();
    if fresh.is_empty() {
        return Ok(None);
    }
    let marks: Vec<(&str, Vec<&str>)> = fresh
        .iter()
        .map(|m| match Asked::of(login, m) {
            Asked::Nothing => (id(m), vec![SEEN]),
            _ if mdn::is_receipt(m) => (id(m), vec![SEEN]),
            _ => (id(m), vec![SEEN, MDN_SENT]),
        })
        .collect();
    match server.mark_if(&state, &marks).await {
        Ok(true) => Ok(Some(deliverable(fresh))),
        Ok(false) => Ok(None),
        Err(e) if e.is::<NeedsSignIn>() => Err(e),
        Err(e) => {
            let fresh_ids: Vec<String> = fresh.iter().map(|m| id(m).to_owned()).collect();
            let Ok((_, now)) = server.snapshot(&fresh_ids).await else {
                return Err(e);
            };
            let landed: Vec<Email> = fresh
                .into_iter()
                .filter(|m| now.iter().any(|n| id(n) == id(m) && has_keyword(n, SEEN)))
                .collect();
            if landed.is_empty() {
                return Err(e);
            }
            warn(&format!(
                "claiming reported {e:#}, but {} mail now carry $seen; delivering them",
                landed.len()
            ));
            Ok(Some(deliverable(landed)))
        }
    }
}

/// Watched inbox mail from a check, sorted by `$seen`.
#[derive(Default)]
struct Scan {
    due: Vec<Email>,
    remind: Vec<Email>,
}

/// Where the incremental checks stand: the Email state the last whole check read, and the mail it left due. It
/// moves only when a whole check succeeds, so a failure anywhere replays the same changes, and a full check that
/// fails is redone in full.
#[derive(Default)]
struct Cursor(RefCell<Position>);

#[derive(Clone, Default)]
struct Position {
    since: Option<String>,
    pending: BTreeSet<String>,
}

trait ChangeSource {
    /// Created and updated ids since `since`, and the new state; `None` when only a full check will do.
    async fn changes(&self, since: &str) -> Result<Option<(Vec<String>, String)>>;
    /// Every candidate id, and the state read before them.
    async fn everything(&self) -> Result<(Vec<String>, String)>;
}

impl Cursor {
    /// Runs `check` over what changed since the last check and what that check left due; `check` returns what is
    /// due now.
    async fn check<T>(
        &self,
        source: &impl ChangeSource,
        check: impl AsyncFnOnce(BTreeSet<String>) -> Result<(T, BTreeSet<String>)>,
    ) -> Result<T> {
        let at = self.0.borrow().clone();
        let changed = match &at.since {
            Some(since) => source.changes(since).await?,
            None => None,
        };
        let (ids, state) = match changed {
            Some(changed) => changed,
            None => source.everything().await?,
        };
        let (found, due) = check(ids.into_iter().chain(at.pending).collect()).await?;
        *self.0.borrow_mut() = Position {
            since: Some(state),
            pending: due,
        };
        Ok(found)
    }
}

impl ChangeSource for Watch<'_> {
    async fn changes(&self, since: &str) -> Result<Option<(Vec<String>, String)>> {
        self.mail.changes(since).await
    }

    async fn everything(&self) -> Result<(Vec<String>, String)> {
        let state = self.mail.email_state().await?;
        let mut ids = vec![];
        if !self.watched.topics.is_empty() {
            ids.extend(self.mail.query(|| self.filter(false)).await?);
            ids.extend(self.mail.query(|| self.filter(true)).await?);
        }
        Ok((ids, state))
    }
}

struct Watch<'a> {
    mail: &'a Mail,
    inbox: String,
    sent: String,
    watched: Watched,
    deadline: Option<Instant>,
    cursor: Cursor,
    reminders: RefCell<Vec<Email>>,
    reminded: RefCell<HashSet<String>>,
    lost: RefCell<HashSet<String>>,
}

impl Watch<'_> {
    fn filter(&self, seen: bool) -> Filter<query::Filter> {
        let keyword = if seen {
            query::Filter::has_keyword(SEEN)
        } else {
            query::Filter::not_keyword(SEEN)
        };
        Filter::and([
            query::Filter::in_mailbox(&self.inbox),
            keyword,
            query::Filter::not_keyword(ANSWERED),
            query::Filter::not_keyword(HANDLED),
        ])
    }

    /// The current threads of the watched emails. A start email that is gone is a warning, once.
    async fn threads(&self) -> Result<HashSet<String>> {
        let mut current = vec![];
        for start in &self.watched.starts {
            match self.mail.current(start).await? {
                Some(now) => current.push(now),
                None => {
                    if self.lost.borrow_mut().insert(start.clone()) {
                        warn(&format!("{start} is gone; no longer watching its thread"));
                    }
                }
            }
        }
        let starts = self.mail.emails(&current, &[P::ThreadId], |_| {}).await?;
        Ok(starts
            .iter()
            .filter_map(Email::thread_id)
            .map(str::to_owned)
            .collect())
    }

    /// Every open inbox mail on a full check, afterwards only what changed since and what was due last time.
    async fn scan(&self) -> Result<Scan> {
        let threads = self.threads().await?;
        self.cursor
            .check(self, async |ids| {
                let scan = self.triage(&threads, ids).await?;
                let due = scan.due.iter().map(|m| id(m).to_owned()).collect();
                Ok((scan, due))
            })
            .await
    }

    /// Only keyword and mailbox filters go to the server; To and Cc are matched here, since the server answers
    /// those from its full-text index, which is built after ingest.
    async fn triage(&self, threads: &HashSet<String>, mut ids: BTreeSet<String>) -> Result<Scan> {
        let login = &self.mail.auth.login;
        let thread_ids: Vec<String> = threads.iter().cloned().collect();
        ids.extend(
            self.mail
                .threads(&thread_ids)
                .await?
                .iter()
                .flat_map(|t| t.email_ids().iter().cloned()),
        );
        let ids: Vec<String> = ids.into_iter().collect();
        let open_mail: Vec<Email> = self
            .mail
            .emails(&ids, &triage_properties(), |_| {})
            .await?
            .into_iter()
            .filter(|m| {
                m.mailbox_ids().contains(&self.inbox.as_str())
                    && self.watched.covers(threads, m)
                    && open(login, m)
            })
            .collect();
        let answered = self.answered_from_sent(&open_mail).await?;
        let (remind, due): (Vec<Email>, Vec<Email>) = open_mail
            .into_iter()
            .filter(|m| !answered.contains(id(m)))
            .partition(|m| has_keyword(m, SEEN));
        Ok(Scan { due, remind })
    }

    async fn answered_from_sent(&self, candidates: &[Email]) -> Result<HashSet<String>> {
        let thread_ids: BTreeSet<String> = candidates
            .iter()
            .filter_map(Email::thread_id)
            .map(str::to_owned)
            .collect();
        let thread_ids: Vec<String> = thread_ids.into_iter().collect();
        let known: HashSet<&str> = candidates.iter().map(id).collect();
        let rest: Vec<String> = self
            .mail
            .threads(&thread_ids)
            .await?
            .iter()
            .flat_map(|t| t.email_ids().iter().cloned())
            .filter(|i| !known.contains(i.as_str()))
            .collect();
        let properties = [
            P::ThreadId,
            P::From,
            P::MailboxIds,
            P::InReplyTo,
            P::MessageId,
            P::Keywords,
        ]
        .into_iter()
        .chain(mdn::properties());
        let members = self
            .mail
            .emails(&rest, &properties.collect::<Vec<_>>(), |_| {})
            .await?;
        let answered = answered_in_sent(
            &self.mail.auth.login,
            &self.inbox,
            &self.sent,
            candidates,
            &members,
        );
        let marks: Vec<(&str, Vec<&str>)> = answered
            .iter()
            .map(|a| (a.as_str(), vec![ANSWERED]))
            .collect();
        self.mail.mark(&marks).await?;
        Ok(answered)
    }

    /// Seen mail still open that this process has not reminded of yet, one line each.
    fn remind(&self) {
        let mut fresh: Vec<Email> = {
            let reminded = self.reminded.borrow();
            self.reminders
                .borrow()
                .iter()
                .filter(|m| !mdn::is_receipt(m) && !reminded.contains(id(m)))
                .cloned()
                .collect()
        };
        fresh.sort_by_key(|m| std::cmp::Reverse(received(m)));
        for m in fresh.iter().take(REMINDERS_SHOWN) {
            warn(&reminder(m));
        }
        if let more @ 1.. = fresh.len().saturating_sub(REMINDERS_SHOWN) {
            warn(&format!("seen but unanswered: {more} more"));
        }
        self.reminded
            .borrow_mut()
            .extend(fresh.iter().map(|m| id(m).to_owned()));
        self.reminders.borrow_mut().clear();
    }

    fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
    }

    /// Sleeps at most until the deadline; `false` when the deadline came first.
    async fn pause(&self, pause: Duration) -> bool {
        match self.remaining() {
            Some(left) if left <= pause => {
                tokio::time::sleep(left).await;
                false
            }
            _ => {
                tokio::time::sleep(pause).await;
                true
            }
        }
    }

    /// Checks once the stream is open, so mail that lands while it opens is not missed. Requests run to the end;
    /// the deadline only bounds the waiting between them.
    async fn settle(&self) -> Result<Wake, Interruption> {
        let session = self.mail.session();
        let mut pushes = session.pushes().await.map_err(Interruption::Failed)?;
        loop {
            let claimed = claim(self, &self.mail.auth.login).await;
            self.remind();
            let claimed = claimed.map_err(Interruption::Failed)?;
            if !claimed.is_empty() {
                return Ok(Wake::Due(claimed));
            }
            let wait = self.remaining().map_or(RECHECK, |left| left.min(RECHECK));
            if wait.is_zero() {
                return Ok(Wake::TimedOut);
            }
            match tokio::time::timeout(wait, pushes.next()).await {
                Ok(Some(Ok(_))) => spread().await,
                Ok(Some(Err(e))) => return Err(Interruption::Dropped(e.to_string())),
                Ok(None) => return Err(Interruption::Dropped("the push stream closed".into())),
                Err(_) if self.remaining().is_some_and(|left| left.is_zero()) => {
                    return Ok(Wake::TimedOut);
                }
                Err(_) => return Err(Interruption::Idle),
            }
        }
    }

    async fn until_due(&self) -> Result<Wake> {
        let mut backoff = BACKOFF_START;
        loop {
            let pause = match self.settle().await {
                Ok(wake) => return Ok(wake),
                Err(Interruption::Idle) => continue,
                Err(Interruption::Dropped(why)) => {
                    warn(&format!("{why}; reopening the push stream"));
                    Duration::from_secs(1)
                }
                Err(Interruption::Failed(e)) if is_permanent(&e) => return Err(e),
                Err(Interruption::Failed(e)) if busy(&e) => {
                    warn("stalwart stays over its request limit; checking again in 30s");
                    Duration::from_secs(30)
                }
                Err(Interruption::Failed(e)) => {
                    warn(&format!("{e:#}; trying again in {}s", backoff.as_secs()));
                    let pause = backoff;
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                    if !self.pause(pause).await {
                        return Ok(Wake::TimedOut);
                    }
                    match self.mail.reconnect().await {
                        Err(e) if is_permanent(&e) => return Err(e),
                        Err(e) => warn(&format!("{e:#}")),
                        Ok(()) => {}
                    }
                    continue;
                }
            };
            if !self.pause(pause).await {
                return Ok(Wake::TimedOut);
            }
        }
    }

    /// The mail is claimed already: if printing fails, it shows up in the next wait's reminders.
    async fn deliver(&self, claimed: Vec<Email>) -> Result<()> {
        let ids: Vec<String> = claimed.iter().map(|m| id(m).to_owned()).collect();
        let shown = [
            P::ReceivedAt,
            P::From,
            P::To,
            P::Subject,
            P::TextBody,
            P::BodyValues,
            P::Attachments,
        ];
        let mut bodies = self
            .mail
            .emails(&ids, &shown, |a| {
                a.fetch_text_body_values(true)
                    .max_body_value_bytes(MAX_BODY_BYTES);
            })
            .await?;
        bodies.sort_by_key(received);
        for m in &bodies {
            out(&format!("{}\n", message(m)))?;
        }
        let login = &self.mail.auth.login;
        let mut outbox = None;
        for m in &claimed {
            match Asked::of(login, m) {
                Asked::Nothing => {}
                Asked::Declined(why) => warn(&why),
                Asked::Send(receipt) => {
                    let outbox = match &outbox {
                        Some(outbox) => outbox,
                        None => outbox.insert(Outbox::open(self.mail).await?),
                    };
                    match receipt.send(self.mail, outbox, now()).await {
                        Ok(()) => {
                            warn(&format!("sent a receipt for {} to {}", id(m), receipt.to()))
                        }
                        Err(e) => warn(&format!("the receipt for {} failed: {e:#}", id(m))),
                    }
                }
            }
        }
        Ok(())
    }
}

impl Claims for Watch<'_> {
    async fn candidates(&self) -> Result<Vec<Email>> {
        let scan = self.scan().await?;
        let mut reminders = self.reminders.borrow_mut();
        for m in scan.remind {
            if !reminders.iter().any(|r| id(r) == id(&m)) {
                reminders.push(m);
            }
        }
        Ok(scan.due)
    }

    async fn canonical(&self, ids: &[String]) -> Result<Vec<String>> {
        self.mail.canonical(ids).await
    }

    async fn ready(&self) -> Result<()> {
        self.mail.ensure_fresh().await
    }

    async fn exclusive(&self) -> Result<Held> {
        self.mail.auth.claims_lock().await
    }

    async fn snapshot(&self, ids: &[String]) -> Result<(String, Vec<Email>)> {
        let session = self.mail.session();
        let ids = &ids[..ids
            .len()
            .min(session.max_objects_in_get())
            .min(session.max_objects_in_set())];
        let mut responses = self
            .mail
            .call_locked(|r| {
                r.get_email()
                    .ids(ids.iter().cloned())
                    .properties(triage_properties());
            })
            .await?;
        let mut got = responses.next()?.unwrap_get_email()?;
        Ok((got.take_state(), got.take_list()))
    }

    async fn mark_if(&self, state: &str, marks: &[(&str, Vec<&str>)]) -> Result<bool> {
        self.mail.mark_if(state, marks).await
    }
}

/// Every session waiting on the shared inbox rescans on the same push; spreading the rescans over a few seconds
/// keeps them under Stalwart's limit on concurrent requests.
async fn spread() {
    let millis = Draws::default().below(250).map_or(0, |n| n as u64 * 12);
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

/// The subject as a JSON string, with the line separators serde_json leaves raw escaped too.
fn reminder(m: &Email) -> String {
    let json = serde_json::to_string(m.subject().unwrap_or_default()).unwrap_or_default();
    let subject: String = json
        .chars()
        .map(|c| match c {
            '\u{2028}' | '\u{2029}' | '\u{85}' | '\u{7f}' => format!("\\u{:04x}", u32::from(c)),
            c => c.to_string(),
        })
        .collect();
    format!("seen but unanswered: {} {subject}", plain(id(m)))
}

enum Wake {
    Due(Vec<Email>),
    TimedOut,
}

enum Interruption {
    /// No push in a recheck period; the stream may have died without a word.
    Idle,
    Dropped(String),
    Failed(anyhow::Error),
}

pub enum Waited {
    Delivered,
    TimedOut,
}

pub async fn run(args: Args) -> Result<Waited> {
    let mail = Mail::connect(Auth::new(Config::load()?)?).await?;
    let login = &mail.auth.login;
    let session = mail.session();
    let watch = Watch {
        inbox: session.mailbox(Role::Inbox)?.to_owned(),
        sent: session.mailbox(Role::Sent)?.to_owned(),
        watched: Watched {
            topics: args
                .topic
                .iter()
                .map(|t| login.topic_address(t).to_lowercase())
                .collect(),
            starts: args.thread.clone(),
        },
        deadline: match &args.timeout {
            Some(timeout) => Some(
                Instant::now()
                    .checked_add(timeout.0)
                    .context("timeout too far ahead")?,
            ),
            None => None,
        },
        mail: &mail,
        cursor: Cursor::default(),
        reminders: RefCell::new(vec![]),
        reminded: RefCell::new(HashSet::new()),
        lost: RefCell::new(HashSet::new()),
    };
    for start in &args.thread {
        ensure!(mail.current(start).await?.is_some(), "no email {start}");
    }
    let watching: Vec<String> = watch
        .watched
        .topics
        .iter()
        .cloned()
        .chain(args.thread.iter().map(|t| format!("the thread of {t}")))
        .collect();
    warn(&format!("waiting for {}", watching.join(", ")));
    let wall = args
        .timeout
        .as_ref()
        .map(|t| now().saturating_add(t.0.as_secs() as i64));
    match watch.until_due().await? {
        Wake::Due(claimed) => {
            watch.deliver(claimed).await?;
            Ok(Waited::Delivered)
        }
        Wake::TimedOut => {
            warn(&format!("no mail by {}", utc(wall.unwrap_or_else(now))));
            Ok(Waited::TimedOut)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use serde_json::{Value, json};

    use super::*;

    const T0: i64 = 1_792_000_000;
    const HIM: &str = "ironmoon@ironmoon.dev";
    const POOL: &str = "agent+pool@ironmoon.dev";

    fn mail_json(
        id: &str,
        thread: &str,
        secs: i64,
        from: &str,
        to: &str,
        keywords: &[&str],
    ) -> Value {
        let keywords: serde_json::Map<String, Value> = keywords
            .iter()
            .map(|k| (k.to_string(), json!(true)))
            .collect();
        json!({
            "id": id, "threadId": thread, "receivedAt": utc(T0 + secs), "keywords": keywords,
            "messageId": [format!("{id}@ironmoon.dev")],
            "from": [{ "email": from }], "to": [{ "email": to }], "mailboxIds": { "inbox": true },
            "header:Content-Type": " text/plain",
        })
    }

    fn mail(id: &str, thread: &str, secs: i64, from: &str, to: &str, keywords: &[&str]) -> Email {
        serde_json::from_value(mail_json(id, thread, secs, from, to, keywords)).unwrap()
    }

    /// A mail of ours in Sent replying to `answers`.
    fn sent_json(id: &str, thread: &str, secs: i64, answers: &str) -> Value {
        let mut m = mail_json(id, thread, secs, POOL, HIM, &[SEEN]);
        m["mailboxIds"] = json!({ "sent": true });
        m["inReplyTo"] = json!([format!("{answers}@ironmoon.dev")]);
        m
    }

    fn sent(id: &str, thread: &str, secs: i64, answers: &str) -> Email {
        serde_json::from_value(sent_json(id, thread, secs, answers)).unwrap()
    }

    fn login() -> Login {
        "agent@ironmoon.dev".parse().unwrap()
    }

    fn ids(list: &[Email]) -> Vec<String> {
        let mut ids: Vec<String> = list.iter().map(|m| id(m).to_owned()).collect();
        ids.sort();
        ids
    }

    #[test]
    fn a_reply_answers_the_mail_it_replies_to_and_no_other() {
        let candidates = [
            mail("a1", "A", 10, HIM, POOL, &[]),
            mail("a2", "A", 20, HIM, POOL, &[]),
            mail("b1", "B", 20, HIM, POOL, &[]),
        ];
        let answered = answered_in_sent(
            &login(),
            "inbox",
            "sent",
            &candidates,
            &[sent("r", "A", 30, "a1")],
        );
        assert_eq!(answered, HashSet::from(["a1".to_owned()]));
    }

    #[test]
    fn a_past_reply_does_not_answer_the_newer_mail_it_skipped() {
        let candidates = [mail("M", "T", 20, HIM, POOL, &[])];
        let members = [mail("P", "T", 10, HIM, POOL, &[]), sent("R", "T", 30, "P")];
        assert!(answered_in_sent(&login(), "inbox", "sent", &candidates, &members).is_empty());
    }

    #[test]
    fn only_our_mail_in_sent_answers_and_receipts_do_not() {
        let candidates = [mail("a1", "A", 20, HIM, POOL, &[])];
        let mut draft: Value = sent_json("d", "A", 30, "a1");
        draft["mailboxIds"] = json!({ "drafts": true });
        let mut receipt: Value = sent_json("r", "A", 30, "a1");
        receipt["header:Content-Type"] =
            json!(" multipart/report; report-type=disposition-notification; boundary=b");
        let mut theirs: Value = sent_json("t", "A", 30, "a1");
        theirs["from"] = json!([{ "email": HIM }]);
        let others: Vec<Email> = [draft, receipt, theirs]
            .into_iter()
            .map(|m| serde_json::from_value(m).unwrap())
            .collect();
        assert!(answered_in_sent(&login(), "inbox", "sent", &candidates, &others).is_empty());
    }

    #[test]
    fn agent_mails_own_replies_do_not_answer_through_sent() {
        let candidates = [mail("a1", "A", 10, HIM, POOL, &[])];
        let mut ours: Value = sent_json("r", "A", 30, "a1");
        ours["keywords"] = json!({ "$seen": true, "$agentmail": true });
        let members = [serde_json::from_value(ours).unwrap()];
        assert!(answered_in_sent(&login(), "inbox", "sent", &candidates, &members).is_empty());
    }

    #[test]
    fn a_reused_or_ambiguous_message_id_answers_nothing() {
        let mut twin = mail_json("a2", "A", 20, HIM, POOL, &[]);
        twin["messageId"] = json!(["a1@ironmoon.dev"]);
        let candidates = [
            mail("a1", "A", 10, HIM, POOL, &[]),
            serde_json::from_value(twin).unwrap(),
        ];
        let reused = answered_in_sent(
            &login(),
            "inbox",
            "sent",
            &candidates,
            &[sent("r", "A", 30, "a1")],
        );
        assert!(reused.is_empty(), "{reused:?}");
        let candidates = [mail("b1", "B", 10, HIM, POOL, &[])];
        let mut two: Value = sent_json("r", "B", 30, "b1");
        two["inReplyTo"] = json!(["b1@ironmoon.dev", "x@ironmoon.dev"]);
        let members = [serde_json::from_value(two).unwrap()];
        assert!(answered_in_sent(&login(), "inbox", "sent", &candidates, &members).is_empty());
    }

    #[test]
    fn mail_from_any_of_our_subaddresses_is_not_open() {
        assert!(!open(
            &login(),
            &mail("o1", "O", 10, "agent+other@ironmoon.dev", POOL, &[])
        ));
        assert!(open(&login(), &mail("h1", "H", 10, HIM, POOL, &[])));
        assert!(!open(&login(), &mail("h2", "H", 10, HIM, POOL, &[HANDLED])));
        assert!(!open(
            &login(),
            &mail("h3", "H", 10, HIM, POOL, &[ANSWERED])
        ));
    }

    #[test]
    fn watched_means_addressed_to_a_topic_or_in_a_thread() {
        let watched = Watched {
            topics: vec![POOL.to_owned()],
            starts: vec!["s".to_owned()],
        };
        let threads = HashSet::from(["T".to_owned()]);
        assert!(watched.covers(
            &threads,
            &mail("topic", "X", 10, HIM, "Agent+Pool@ironmoon.dev", &[])
        ));
        assert!(watched.covers(
            &threads,
            &mail("thread", "T", 20, HIM, "agent@ironmoon.dev", &[])
        ));
        assert!(!watched.covers(
            &threads,
            &mail("neither", "Y", 30, HIM, "xagent+pool@ironmoon.dev", &[])
        ));
    }

    /// An inbox shared with another session, which may claim mail between our snapshot and our mark.
    struct Shared {
        mail: RefCell<HashMap<String, Value>>,
        state: RefCell<u32>,
        rival_claims: RefCell<Option<String>>,
        attempts: RefCell<u32>,
        /// The next mark lands, but its reply is lost.
        lose_reply: RefCell<bool>,
        /// Old ids, from before a thread merge, that name the same email as a current one.
        stale: RefCell<HashMap<String, String>>,
    }

    impl Shared {
        fn new(mail: Vec<Value>, rival_claims: Option<&str>) -> Shared {
            Shared {
                mail: RefCell::new(
                    mail.into_iter()
                        .map(|m| (m["id"].as_str().unwrap().to_owned(), m))
                        .collect(),
                ),
                state: RefCell::new(0),
                rival_claims: RefCell::new(rival_claims.map(str::to_owned)),
                attempts: RefCell::new(0),
                lose_reply: RefCell::new(false),
                stale: RefCell::new(HashMap::new()),
            }
        }

        fn keywords(&self, id: &str) -> Vec<String> {
            let mail = self.mail.borrow();
            let mut keywords: Vec<String> = mail[id]["keywords"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            keywords.sort();
            keywords
        }

        fn add(&self, id: &str, keyword: &str) {
            let document = self.document(id);
            self.mail.borrow_mut().get_mut(&document).unwrap()["keywords"][keyword] = json!(true);
            *self.state.borrow_mut() += 1;
        }

        fn document(&self, id: &str) -> String {
            self.stale
                .borrow()
                .get(id)
                .map_or(id, String::as_str)
                .to_owned()
        }

        /// The email as fetched under `id`, which may be a stale one.
        fn under(&self, id: &str) -> Email {
            let mut m = self.mail.borrow()[&self.document(id)].clone();
            m["id"] = json!(id);
            serde_json::from_value(m).unwrap()
        }
    }

    impl Claims for Shared {
        async fn ready(&self) -> Result<()> {
            Ok(())
        }

        async fn canonical(&self, ids: &[String]) -> Result<Vec<String>> {
            let current: BTreeSet<String> = ids.iter().map(|i| self.document(i)).collect();
            Ok(current.into_iter().collect())
        }

        async fn exclusive(&self) -> Result<Held> {
            Ok(Held::nothing())
        }

        /// Stale ids too, as the ids a check left due can hold one.
        async fn candidates(&self) -> Result<Vec<Email>> {
            let login = login();
            let current: Vec<String> = self.mail.borrow().keys().cloned().collect();
            let stale: Vec<String> = self.stale.borrow().keys().cloned().collect();
            Ok(current
                .iter()
                .chain(&stale)
                .map(|i| self.under(i))
                .filter(|m| open(&login, m) && !has_keyword(m, SEEN))
                .collect())
        }

        async fn snapshot(&self, ids: &[String]) -> Result<(String, Vec<Email>)> {
            let list = ids.iter().map(|i| self.under(i)).collect();
            Ok((self.state.borrow().to_string(), list))
        }

        async fn mark_if(&self, state: &str, marks: &[(&str, Vec<&str>)]) -> Result<bool> {
            *self.attempts.borrow_mut() += 1;
            let documents: HashSet<String> =
                marks.iter().map(|(id, _)| self.document(id)).collect();
            ensure!(documents.len() == marks.len(), "two updates to one email");
            if let Some(rival) = self.rival_claims.borrow_mut().take() {
                self.add(&rival, SEEN);
            }
            if state != self.state.borrow().to_string() {
                return Ok(false);
            }
            for (id, keywords) in marks {
                for keyword in keywords {
                    self.add(id, keyword);
                }
            }
            if self.lose_reply.replace(false) {
                bail!("the reply timed out");
            }
            Ok(true)
        }
    }

    #[tokio::test]
    async fn a_mail_another_session_claims_first_is_not_delivered_twice() {
        let shared = Shared::new(
            vec![
                mail_json("a1", "A", 10, HIM, POOL, &[]),
                mail_json("b1", "B", 20, HIM, POOL, &[]),
            ],
            Some("a1"),
        );
        let claimed = claim(&shared, &login()).await.unwrap();
        assert_eq!(ids(&claimed), ["b1"]);
        assert_eq!(*shared.attempts.borrow(), 2);
        assert_eq!(shared.keywords("b1"), [SEEN]);
    }

    #[tokio::test]
    async fn a_claim_that_lands_but_errors_is_still_delivered() {
        let shared = Shared::new(vec![mail_json("a1", "A", 10, HIM, POOL, &[])], None);
        *shared.lose_reply.borrow_mut() = true;
        let claimed = claim(&shared, &login()).await.unwrap();
        assert_eq!(ids(&claimed), ["a1"]);
        assert_eq!(shared.keywords("a1"), [SEEN]);
    }

    #[tokio::test]
    async fn receipts_are_claimed_silently_and_receipt_requests_get_mdnsent() {
        let mut receipt = mail_json("r1", "A", 10, HIM, POOL, &[]);
        receipt["header:Content-Type"] =
            json!(" multipart/report; report-type=disposition-notification; boundary=b");
        let mut asks = mail_json("q1", "B", 20, HIM, POOL, &[]);
        asks["header:Disposition-Notification-To:asAddresses"] = json!([{ "email": HIM }]);
        let shared = Shared::new(vec![receipt, asks], None);
        let claimed = claim(&shared, &login()).await.unwrap();
        assert_eq!(ids(&claimed), ["q1"]);
        assert_eq!(shared.keywords("r1"), [SEEN]);
        assert_eq!(shared.keywords("q1"), [MDN_SENT, SEEN]);
        assert!(claim(&shared, &login()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_id_and_the_current_one_claim_one_email_once() {
        let shared = Shared::new(vec![mail_json("a2", "A", 10, HIM, POOL, &[])], None);
        shared
            .stale
            .borrow_mut()
            .insert("a1".to_owned(), "a2".to_owned());
        let claimed = claim(&shared, &login()).await.unwrap();
        assert_eq!(ids(&claimed), ["a2"]);
        assert_eq!(*shared.attempts.borrow(), 1);
        assert_eq!(shared.keywords("a2"), [SEEN]);
    }

    /// Email states `s0`, `s1`, ...; `changes` maps a state to the ids changed after it.
    struct Log {
        changes: HashMap<&'static str, Vec<&'static str>>,
        head: &'static str,
        full_checks: RefCell<u32>,
    }

    impl ChangeSource for Log {
        async fn changes(&self, since: &str) -> Result<Option<(Vec<String>, String)>> {
            Ok(self.changes.get(since).map(|ids| {
                let ids = ids.iter().map(|&i| i.to_owned()).collect();
                (ids, self.head.to_owned())
            }))
        }

        async fn everything(&self) -> Result<(Vec<String>, String)> {
            *self.full_checks.borrow_mut() += 1;
            Ok((vec!["old".to_owned()], "s0".to_owned()))
        }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|&i| i.to_owned()).collect()
    }

    /// Checks once and returns the ids the check saw: `Ok` when it found `due` due, `Err` when its get failed.
    async fn check(
        cursor: &Cursor,
        log: &Log,
        due: Option<&[&str]>,
    ) -> std::result::Result<BTreeSet<String>, BTreeSet<String>> {
        let mut seen = BTreeSet::new();
        let checked = cursor
            .check(log, async |ids| {
                seen = ids;
                Ok(((), set(due.context("the get failed")?)))
            })
            .await;
        match checked {
            Ok(()) => Ok(seen),
            Err(_) => Err(seen),
        }
    }

    #[tokio::test]
    async fn a_check_that_fails_after_changes_leaves_their_ids_due() {
        let log = Log {
            changes: HashMap::from([("s0", vec!["m1"]), ("s1", vec![])]),
            head: "s1",
            full_checks: RefCell::new(0),
        };
        let cursor = Cursor::default();
        assert_eq!(check(&cursor, &log, Some(&[])).await, Ok(set(&["old"])));
        assert_eq!(check(&cursor, &log, None).await, Err(set(&["m1"])));
        assert_eq!(check(&cursor, &log, Some(&["m1"])).await, Ok(set(&["m1"])));
        assert_eq!(check(&cursor, &log, Some(&[])).await, Ok(set(&["m1"])));
        assert_eq!(check(&cursor, &log, Some(&[])).await, Ok(set(&[])));
        assert_eq!(*log.full_checks.borrow(), 1);
    }

    #[tokio::test]
    async fn a_full_check_that_fails_is_redone_in_full() {
        let log = Log {
            changes: HashMap::from([("s0", vec![])]),
            head: "s0",
            full_checks: RefCell::new(0),
        };
        let cursor = Cursor::default();
        assert_eq!(check(&cursor, &log, None).await, Err(set(&["old"])));
        assert_eq!(check(&cursor, &log, Some(&[])).await, Ok(set(&["old"])));
        assert_eq!(check(&cursor, &log, Some(&[])).await, Ok(set(&[])));
        assert_eq!(*log.full_checks.borrow(), 2);
    }

    #[test]
    fn a_subject_cannot_forge_another_reminder() {
        let mut m = mail_json("m1", "A", 10, HIM, POOL, &[SEEN]);
        m["subject"] = json!("x\", m42 \"urgent\nseen but unanswered: m43 \"fake\"");
        let line = reminder(&serde_json::from_value(m).unwrap());
        assert_eq!(
            line,
            r#"seen but unanswered: m1 "x\", m42 \"urgent\nseen but unanswered: m43 \"fake\"""#
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn unicode_line_separators_in_a_subject_are_escaped() {
        let mut m = mail_json("m1", "A", 10, HIM, POOL, &[SEEN]);
        m["subject"] = json!("a\u{2028}b\u{2029}c\u{85}d\u{7f}e");
        let line = reminder(&serde_json::from_value(m).unwrap());
        assert_eq!(
            line,
            r#"seen but unanswered: m1 "a\u2028b\u2029c\u0085d\u007fe""#
        );
    }

    #[test]
    fn timeouts() {
        let secs = |s: &str| s.parse::<Timeout>().map(|t| t.0.as_secs());
        assert_eq!(secs("90s").unwrap(), 90);
        assert_eq!(secs("2").unwrap(), 120);
        assert_eq!(secs("2h").unwrap(), 7200);
        assert_eq!(secs("30d").unwrap(), 30 * 86400);
        for bad in [
            "",
            "m",
            "1x",
            "1.5h",
            "-1m",
            "1mm",
            "31d",
            "99999999999999999999d",
            "18446744073709551615m",
        ] {
            assert!(secs(bad).is_err(), "{bad}");
        }
    }
}
