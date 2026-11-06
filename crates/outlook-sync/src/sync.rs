use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use private::{Dir, Secret};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::event::Change;
use crate::file::FileName;
use crate::graph::{Delta, Graph, Refused};
use crate::ledger::{Edit, Entry, Ledger, Mirror, Mirrored, Put};
use crate::local;
use crate::policy::Policy;
use crate::state::{self, Subscription, Tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wake {
    Changed,
    Edited,
    Daily,
    Missed,
    Reauthorize,
    PutBack,
    Removed,
    SignedIn,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// The subscription kept and the mirror brought up to Graph.
    pub graph: bool,
    pub fresh_subscription: bool,
    pub reauthorize: bool,
    pub listing: bool,
}

impl Wake {
    pub fn needs(self) -> Needs {
        let graph = Needs { graph: true, ..Needs::default() };
        match self {
            Wake::Edited => Needs::default(),
            Wake::Changed => graph,
            Wake::Reauthorize => Needs { reauthorize: true, ..graph },
            Wake::Removed => Needs { fresh_subscription: true, listing: true, ..graph },
            Wake::Daily | Wake::Missed | Wake::PutBack | Wake::SignedIn => {
                Needs { listing: true, ..graph }
            }
        }
    }

    /// New credentials may be what the failed run lacked.
    fn waits_out_backoff(self) -> bool {
        self != Wake::SignedIn
    }
}

impl std::ops::BitOr for Needs {
    type Output = Needs;
    fn bitor(self, other: Needs) -> Needs {
        Needs {
            graph: self.graph || other.graph,
            fresh_subscription: self.fresh_subscription || other.fresh_subscription,
            reauthorize: self.reauthorize || other.reauthorize,
            listing: self.listing || other.listing,
        }
    }
}

pub fn needs<'a>(wakes: impl IntoIterator<Item = &'a Wake>) -> Needs {
    wakes.into_iter().fold(Needs::default(), |n, w| n | w.needs())
}

const DAILY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(5 * 60);
/// A subscription is renewed at the first run this close to its end: a daily run, retried, lands in it.
const RENEWAL: SignedDuration = SignedDuration::from_secs(2 * DAILY.as_secs() as i64);

pub struct Shared {
    pub subscription: RefCell<Option<Subscription>>,
    /// A subscribe of ours is waiting on Graph's validation request to the notification URL.
    pub validating: Cell<bool>,
    pub wake: mpsc::UnboundedSender<Wake>,
}

impl Shared {
    async fn validating<T>(&self, subscribe: impl Future<Output = T>) -> T {
        self.validating.set(true);
        let made = subscribe.await;
        self.validating.set(false);
        made
    }

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
    pub state: std::rc::Rc<Dir>,
    pub policy: &'static Policy,
    pub shared: std::rc::Rc<Shared>,
}

impl Sync {
    pub async fn run(&self, mut wakes: mpsc::UnboundedReceiver<Wake>) {
        let mut daily = tokio::time::interval(DAILY);
        let mut retry: Option<tokio::time::Instant> = None;
        let mut pending = BTreeSet::new();
        loop {
            let mut woken = BTreeSet::new();
            tokio::select! {
                Some(wake) = wakes.recv() => { woken.insert(wake); }
                _ = daily.tick() => { woken.insert(Wake::Daily); }
                () = sleep_until(retry) => { retry = None; }
            }
            while let Ok(wake) = wakes.try_recv() {
                woken.insert(wake);
            }
            let backing_off = retry.is_some() && woken.iter().all(|w| w.waits_out_backoff());
            pending.extend(woken);
            if backing_off {
                continue;
            }
            retry = match self.handle(needs(&pending)).await {
                Ok(()) => {
                    pending.clear();
                    None
                }
                Err(e) => {
                    yakery_log::warn!(
                        "{pending:?}: {e:#}; trying again in {} minutes",
                        RETRY.as_secs() / 60
                    );
                    Some(tokio::time::Instant::now() + RETRY)
                }
            };
        }
    }

    async fn handle(&self, needs: Needs) -> Result<()> {
        let Tokens { refresh } = self.state.load(state::TOKENS)?;
        if refresh.is_none() {
            yakery_log::warn!("not signed in; run `outlook-sync login`");
            return Ok(());
        }
        let mut ledger = Ledger::open(&self.mirror, &self.state)?;
        // a listing first records Outlook's files, so the owner's edits of them are told from the sync's
        let listed = self.policy.write && !ledger.looked();
        if listed {
            self.mirror(&mut ledger, true).await?;
        }
        let edited = match self.policy.write {
            true => self.edits(&mut ledger).await,
            false => Ok(()),
        };
        if !needs.graph {
            return edited;
        }
        let fresh = needs.fresh_subscription || self.shared.subscription.borrow().is_none();
        let subscribed = self.subscription(fresh, needs.reauthorize).await;
        let mirrored = self.mirror(&mut ledger, needs.listing && !listed).await;
        every([edited, subscribed, mirrored])
    }

    /// A created event's local file goes once sent, so the delta brings it back under its Graph id.
    async fn edits(&self, ledger: &mut Ledger<'_>) -> Result<()> {
        let (planned, token) = ledger.changes().await?;
        for (Entry { file, .. }, edit) in planned {
            match edit {
                Edit::Unchanged => {}
                Edit::Unsettled => {
                    self.put_back(&file, Refused("a write of the sync's was cut off".into()))
                }
                Edit::Deleted(mirrored) => {
                    if let Err(refused) = self.graph.remove(&mirrored.id).await? {
                        self.put_back(&file, refused);
                    }
                    ledger.forget(&file)?;
                }
                Edit::Edited(mirrored) => {
                    let Some((ics, etag)) = self.mirror.get(&file).await? else { continue };
                    let taken = match local::graph(&ics) {
                        Err(e) => Err(Refused(format!("{e:#}"))),
                        Ok(event) => match local::moved(&event, &mirrored.fields) {
                            moved if moved.is_empty() => Ok(local::fields(&event)),
                            moved => {
                                (self.graph.update(&mirrored.id, Value::Object(moved)).await?)
                                    .map(|()| local::fields(&event))
                            }
                        },
                    };
                    let fields = taken.unwrap_or_else(|refused| {
                        self.put_back(&file, refused);
                        mirrored.fields.clone()
                    });
                    let mirrored = Mirrored { etag: Some(etag), fields, content: None, ..mirrored };
                    ledger.record(file, mirrored)?;
                }
                Edit::New => {
                    let Some((ics, _)) = self.mirror.get(&file).await? else { continue };
                    match local::owned(&file, &ics) {
                        Ok(None) => {
                            self.put_back(&file, Refused("the sync's file, unrecorded".into()))
                        }
                        Ok(Some(event)) => match self.graph.create(event).await? {
                            Ok(()) => ledger.delete([file]).await?,
                            Err(Refused(why)) => yakery_log::warn!("{file} stays unsent: {why}"),
                        },
                        Err(e) => yakery_log::warn!("{file} stays unsent: {e:#}"),
                    }
                }
            }
        }
        ledger.looked_up_to(token)
    }

    fn put_back(&self, file: &FileName, Refused(why): Refused) {
        yakery_log::warn!("{file}: {why}; Outlook's version comes back");
        let _ = self.shared.wake.send(Wake::PutBack);
    }

    async fn subscription(&self, fresh: bool, reauthorize: bool) -> Result<()> {
        let held = self.shared.subscription.borrow().clone();
        let kept = match held {
            Some(mut s) if !fresh => {
                let due = s.expires < Timestamp::now() + RENEWAL || reauthorize;
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
                let client_state = private::random::token(32)?;
                self.shared.validating(self.graph.subscribe(&client_state)).await?
            }
        };
        self.state.save(state::SUBSCRIPTION, &Some(&subscription))?;
        *self.shared.subscription.borrow_mut() = Some(subscription);
        Ok(())
    }

    async fn mirror(&self, ledger: &mut Ledger<'_>, full: bool) -> Result<()> {
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
        let mut url = start;
        let link = loop {
            let page = match self.graph.page(&url).await? {
                Delta::Page(page) => page,
                Delta::Expired if !full => return Box::pin(self.mirror(ledger, true)).await,
                Delta::Expired => anyhow::bail!("graph refused a fresh listing as expired"),
            };
            let (puts, gone) = sort(page.changes, now)?;
            seen.extend(puts.iter().map(|p| p.file.clone()));
            ledger.delete(gone).await?;
            if !ledger.write(puts, self.policy.write).await?.is_empty() {
                let _ = self.shared.wake.send(Wake::Edited);
            }
            match (page.next, page.delta) {
                (Some(next), _) => url = next,
                (None, Some(delta)) => break delta,
                (None, None) => anyhow::bail!("graph delta: a last page with no delta link"),
            }
        };
        if full {
            ledger.sweep(&seen, self.policy.write).await?;
        }
        self.state.save(state::DELTA, &state::Delta { link: Some(link) })
    }
}

fn sort(changes: Vec<Change>, now: Timestamp) -> Result<(Vec<Put>, Vec<FileName>)> {
    let (mut puts, mut gone) = (vec![], vec![]);
    for change in changes {
        match change {
            Change::Removed { id } => gone.push(FileName::of(&id)),
            Change::Present(event) => match event.ics(now)? {
                Some(ics) => puts.push(Put { file: FileName::of(&event.id), id: event.id, ics }),
                None => gone.push(FileName::of(&event.id)),
            },
        }
    }
    Ok((puts, gone))
}

/// Each part's failure, so one part's can't keep the others from running.
fn every<const N: usize>(parts: [Result<()>; N]) -> Result<()> {
    let failed: Vec<String> =
        parts.into_iter().filter_map(Result::err).map(|e| format!("{e:#}")).collect();
    anyhow::ensure!(failed.is_empty(), "{}", failed.join("; "));
    Ok(())
}

fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(24 * n)
}

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
