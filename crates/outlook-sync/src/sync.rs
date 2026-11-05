//! One loop that does all the work, woken by notifications, lifecycle events, Stalwart's push, a
//! sign-in and a daily timer: send the owner's edits in the mirror to Outlook, when writing back
//! is on; keep the subscription alive; and bring the mirror up to Graph by a delta run, or by a
//! fresh listing over a new window, which also drops what Graph no longer lists.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use private::{Dir, Secret};
use tokio::sync::mpsc;

use crate::caldav::Mirror;
use crate::event::{Change, file};
use crate::graph::{Delta, Graph, Refused};
use crate::policy::Policy;
use crate::state::{self, Subscription, Tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wake {
    Changed,
    /// The mirror may hold an edit of the owner's.
    Local,
    Daily,
    Missed,
    Reauthorize,
    /// Graph refused an edit of the owner's to one of its events, so a fresh listing puts
    /// Outlook's version back.
    Refused,
    Removed,
    SignedIn,
}

const DAILY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after a failed run the loop tries again, should no notification come first.
const RETRY: Duration = Duration::from_secs(5 * 60);

/// What the server and the loop share: the subscription notifications are checked against, and
/// whether a subscribe of ours is waiting on Graph's validation request.
pub struct Shared {
    pub subscription: RefCell<Option<Subscription>>,
    pub validating: Cell<bool>,
    pub wake: mpsc::UnboundedSender<Wake>,
}

impl Shared {
    pub fn ours(&self, id: &str, client_state: &str) -> bool {
        self.subscription
            .borrow()
            .as_ref()
            .is_some_and(|s| s.id == id && s.client_state == Secret::new(client_state))
    }
}

pub struct Sync {
    pub graph: Graph,
    pub mirror: Mirror,
    pub state: Dir,
    pub policy: &'static Policy,
    pub shared: std::rc::Rc<Shared>,
}

impl Sync {
    pub async fn run(&self, mut wakes: mpsc::UnboundedReceiver<Wake>) {
        let mut daily = tokio::time::interval(DAILY);
        let mut retry: Option<tokio::time::Instant> = None;
        loop {
            let first = tokio::select! {
                Some(wake) = wakes.recv() => wake,
                _ = daily.tick() => Wake::Daily,
                () = sleep_until(retry) => Wake::Changed,
            };
            let mut woken = BTreeSet::from([first]);
            while let Ok(wake) = wakes.try_recv() {
                woken.insert(wake);
            }
            retry = match self.handle(&woken).await {
                Ok(()) => None,
                Err(e) => {
                    yakery_log::warn!(
                        "{woken:?}: {e:#}; trying again in {} minutes",
                        RETRY.as_secs() / 60
                    );
                    Some(tokio::time::Instant::now() + RETRY)
                }
            };
        }
    }

    async fn handle(&self, woken: &BTreeSet<Wake>) -> Result<()> {
        let Tokens { refresh } = self.state.load(state::TOKENS)?;
        if refresh.is_none() {
            yakery_log::warn!("not signed in; run `outlook-sync login`");
            return Ok(());
        }
        // his edits go first, so a delta can't write over one before it is sent
        if self.policy.write {
            self.edits().await?;
        }
        if woken.iter().all(|w| *w == Wake::Local) {
            return Ok(());
        }
        let fresh = woken.contains(&Wake::Removed) || self.shared.subscription.borrow().is_none();
        self.subscription(fresh, woken.contains(&Wake::Reauthorize)).await?;
        let full =
            woken.iter().any(|w| !matches!(w, Wake::Changed | Wake::Local | Wake::Reauthorize));
        self.mirror(full).await
    }

    /// Sends Outlook what the owner changed in the mirror since the last look: an edited
    /// occurrence as an update, a deleted one as a delete, and an event he made as a create, whose
    /// local copy then goes, so the delta brings back Outlook's. An edit of Outlook's event that
    /// can't go is overwritten by Outlook's version; an event of his that can't go stays.
    async fn edits(&self) -> Result<()> {
        let mut written: state::Written = self.state.load(state::WRITTEN)?;
        let (entries, token) = self.mirror.changes(written.token.as_deref()).await?;
        for entry in entries {
            let copy = written.files.get(&entry.file).cloned();
            match (copy, entry.gone) {
                (Some(copy), true) => {
                    if let Err(refused) = self.graph.remove(&copy.id).await? {
                        self.put_back(&entry.file, refused);
                    }
                    written.files.remove(&entry.file);
                }
                (None, true) => {}
                (Some(copy), false) if entry.etag.as_deref() == Some(copy.etag.as_str()) => {}
                (copy, false) => {
                    let Some((ics, etag)) = self.mirror.get(&entry.file).await? else { continue };
                    let sent = match crate::local::graph(&ics) {
                        Err(e) => Err(Refused(format!("{e:#}"))),
                        Ok(event) => match &copy {
                            Some(copy) => self.graph.update(&copy.id, event).await?,
                            None => {
                                self.graph
                                    .create(event, entry.file.trim_end_matches(".ics"))
                                    .await?
                            }
                        },
                    };
                    match (copy, sent) {
                        (Some(copy), sent) => {
                            if let Err(refused) = sent {
                                self.put_back(&entry.file, refused);
                            }
                            written.files.insert(entry.file, state::Copy { etag, ..copy });
                        }
                        (None, Ok(())) => self.mirror.delete(&entry.file).await?,
                        (None, Err(Refused(why))) => {
                            yakery_log::warn!("{} stays unsent: {why}", entry.file)
                        }
                    }
                }
            }
            self.state.save(state::WRITTEN, &written)?;
        }
        written.token = Some(token);
        self.state.save(state::WRITTEN, &written)
    }

    /// Logs an edit of one of Outlook's events that didn't go, and asks for the fresh listing that
    /// puts Outlook's version back.
    fn put_back(&self, file: &str, Refused(why): Refused) {
        yakery_log::warn!("{file}: {why}; Outlook's version comes back");
        let _ = self.shared.wake.send(Wake::Refused);
    }

    async fn subscription(&self, fresh: bool, reauthorize: bool) -> Result<()> {
        let held = self.shared.subscription.borrow().clone();
        let kept = match held {
            Some(mut s) if !fresh => {
                let due = s.expires < Timestamp::now() + days(2) || reauthorize;
                let alive = !reauthorize || self.graph.reauthorize(&s).await?;
                match alive && (!due || self.graph.renew(&mut s).await?) {
                    true => Some(s),
                    false => None,
                }
            }
            _ => None,
        };
        let subscription = match kept {
            Some(s) => s,
            None => {
                self.shared.validating.set(true);
                let made = self.graph.subscribe(&private::random::token(32)?).await;
                self.shared.validating.set(false);
                made?
            }
        };
        self.state.save(state::SUBSCRIPTION, &Some(&subscription))?;
        *self.shared.subscription.borrow_mut() = Some(subscription);
        Ok(())
    }

    async fn mirror(&self, full: bool) -> Result<()> {
        let held: state::Delta = self.state.load(state::DELTA)?;
        let now = Timestamp::now();
        let start = match (&held.link, full) {
            (Some(link), false) => link.clone(),
            _ => {
                let (from, to) = window(self.policy, now);
                Graph::listing(from, to)
            }
        };
        let mut seen = BTreeSet::new();
        let mut written: state::Written = self.state.load(state::WRITTEN)?;
        let mut url = start;
        let link = loop {
            let page = match self.graph.page(&url).await? {
                Delta::Page(page) => page,
                Delta::Expired if !full => return Box::pin(self.mirror(true)).await,
                Delta::Expired => anyhow::bail!("graph refused a fresh listing as expired"),
            };
            for change in page.changes {
                self.apply(change, &mut seen, &mut written, now).await?;
            }
            self.state.save(state::WRITTEN, &written)?;
            match (page.next, page.delta) {
                (Some(next), _) => url = next,
                (None, Some(delta)) => break delta,
                (None, None) => anyhow::bail!("graph delta: a last page with no delta link"),
            }
        };
        if full {
            // what the owner made and the sync hasn't sent yet isn't Graph's to drop
            let gone: Vec<String> = self
                .mirror
                .files()
                .await?
                .difference(&seen)
                .filter(|f| !self.policy.write || written.files.contains_key(*f))
                .cloned()
                .collect();
            for file in gone {
                self.mirror.delete(&file).await?;
                written.files.remove(&file);
            }
            self.state.save(state::WRITTEN, &written)?;
        }
        self.state.save(state::DELTA, &state::Delta { link: Some(link) })
    }

    async fn apply(
        &self,
        change: Change,
        seen: &mut BTreeSet<String>,
        written: &mut state::Written,
        now: Timestamp,
    ) -> Result<()> {
        let (id, ics) = match change {
            Change::Removed { id, .. } => (id, None),
            Change::Present(event) => {
                let ics = event.ics(self.policy.host(), now)?;
                (event.id, ics)
            }
        };
        let name = file(&id);
        let Some(ics) = ics else {
            self.mirror.delete(&name).await?;
            written.files.remove(&name);
            return Ok(());
        };
        // with writing back on, only over the version the sync wrote: a newer one is the owner's,
        // and his edit goes to Outlook before Outlook's version comes back
        let over = written.files.get(&name).filter(|_| self.policy.write).map(|c| c.etag.clone());
        match self.mirror.put(&name, ics, over.as_deref()).await? {
            Some(etag) => {
                written.files.insert(name.clone(), state::Copy { id, etag });
            }
            None => {
                let _ = self.shared.wake.send(Wake::Local);
            }
        }
        seen.insert(name);
        Ok(())
    }
}

/// Days as 24 hours each: a timestamp has no calendar to add days in.
fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(24 * n)
}

/// The span a fresh listing asks Graph for.
fn window(policy: &Policy, now: Timestamp) -> (Timestamp, Timestamp) {
    (now - days(policy.days_back), now + days(policy.days_ahead))
}

#[cfg(test)]
#[path = "../tests/sync.rs"]
mod tests;

async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
