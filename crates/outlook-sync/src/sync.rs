use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use private::{Dir, Secret};
use tokio::sync::mpsc;

use crate::caldav::{Entry, Mirror};
use crate::event::{Change, file};
use crate::graph::{Delta, Graph, Refused};
use crate::ledger::{Edit, Ledger, Mirrored};
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

pub struct Shared {
    pub subscription: RefCell<Option<Subscription>>,
    /// A subscribe of ours is waiting on Graph's validation request to the notification URL.
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
            tokio::select! {
                Some(wake) = wakes.recv() => { pending.insert(wake); }
                _ = daily.tick() => { pending.insert(Wake::Daily); }
                () = sleep_until(retry) => {}
            }
            while let Ok(wake) = wakes.try_recv() {
                pending.insert(wake);
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
        let mut ledger = Ledger::open(&self.mirror, &self.state, self.policy.public.host())?;
        // a listing first records Outlook's files, so his edits of them are told from the sync's
        let listed = self.policy.write && !ledger.looked();
        if listed {
            self.mirror(&mut ledger, true).await?;
        }
        if self.policy.write {
            self.edits(&mut ledger).await?;
        }
        if !needs.graph {
            return Ok(());
        }
        let fresh = needs.fresh_subscription || self.shared.subscription.borrow().is_none();
        self.subscription(fresh, needs.reauthorize).await?;
        self.mirror(&mut ledger, needs.listing && !listed).await
    }

    /// A created event's local file goes once sent, so the delta brings it back under its Graph id.
    async fn edits(&self, ledger: &mut Ledger<'_>) -> Result<()> {
        let (planned, token) = ledger.changes().await?;
        for (Entry { file, .. }, edit) in planned {
            match edit {
                Edit::Unchanged => {}
                Edit::Deleted(mirrored) => {
                    if let Err(refused) = self.graph.remove(&mirrored.id).await? {
                        self.put_back(&file, refused);
                    }
                    ledger.forget(&file)?;
                }
                Edit::Edited(mirrored) => {
                    let Some((ics, etag)) = ledger.get(&file).await? else { continue };
                    let sent = match local::graph(&ics) {
                        Err(e) => Err(Refused(format!("{e:#}"))),
                        Ok(event) => self.graph.update(&mirrored.id, event).await?,
                    };
                    if let Err(refused) = sent {
                        self.put_back(&file, refused);
                    }
                    ledger.record(file, Mirrored { etag, ..mirrored })?;
                }
                Edit::New => {
                    let Some((ics, _)) = ledger.get(&file).await? else { continue };
                    match local::owned(&file, &ics, self.policy.public.host()) {
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

    fn put_back(&self, file: &str, Refused(why): Refused) {
        yakery_log::warn!("{file}: {why}; Outlook's version comes back");
        let _ = self.shared.wake.send(Wake::PutBack);
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
            for change in page.changes {
                self.apply(change, &mut seen, ledger, now).await?;
            }
            ledger.save()?;
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

    async fn apply(
        &self,
        change: Change,
        seen: &mut BTreeSet<String>,
        ledger: &mut Ledger<'_>,
        now: Timestamp,
    ) -> Result<()> {
        let (id, ics) = match change {
            Change::Removed { id, .. } => (id, None),
            Change::Present(event) => {
                let ics = event.ics(self.policy.public.host(), now)?;
                (event.id, ics)
            }
        };
        let name = file(&id);
        let Some(ics) = ics else { return ledger.delete([name]).await };
        if !ledger.write(&name, id, ics, self.policy.write).await? {
            let _ = self.shared.wake.send(Wake::Edited);
        }
        seen.insert(name);
        Ok(())
    }
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
