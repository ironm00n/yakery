//! One loop that does all the work, woken by notifications, lifecycle events, a sign-in and a
//! daily timer: keep the subscription alive, and bring the mirror up to Graph by a delta run, or
//! by a fresh listing over a new window, which also drops what Graph no longer lists.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use private::{Dir, Secret};
use tokio::sync::mpsc;

use crate::caldav::Mirror;
use crate::event::{Change, file};
use crate::graph::{Delta, Graph};
use crate::policy::Policy;
use crate::state::{self, Subscription, Tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wake {
    Changed,
    Daily,
    Missed,
    Reauthorize,
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
        let fresh = woken.contains(&Wake::Removed) || self.shared.subscription.borrow().is_none();
        self.subscription(fresh, woken.contains(&Wake::Reauthorize)).await?;
        let full = woken.iter().any(|w| *w != Wake::Changed && *w != Wake::Reauthorize);
        self.mirror(full).await
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
        let mut url = start;
        let link = loop {
            let page = match self.graph.page(&url).await? {
                Delta::Page(page) => page,
                Delta::Expired if !full => return Box::pin(self.mirror(true)).await,
                Delta::Expired => anyhow::bail!("graph refused a fresh listing as expired"),
            };
            for change in page.changes {
                self.apply(change, &mut seen, now).await?;
            }
            match (page.next, page.delta) {
                (Some(next), _) => url = next,
                (None, Some(delta)) => break delta,
                (None, None) => anyhow::bail!("graph delta: a last page with no delta link"),
            }
        };
        if full {
            for gone in self.mirror.files().await?.difference(&seen) {
                self.mirror.delete(gone).await?;
            }
        }
        self.state.save(state::DELTA, &state::Delta { link: Some(link) })
    }

    async fn apply(
        &self,
        change: Change,
        seen: &mut BTreeSet<String>,
        now: Timestamp,
    ) -> Result<()> {
        match change {
            Change::Removed { id, .. } => self.mirror.delete(&file(&id)).await,
            Change::Present(event) => match event.ics(self.policy.host(), now)? {
                Some(ics) => {
                    let name = file(&event.id);
                    self.mirror.put(&name, ics).await?;
                    seen.insert(name);
                    Ok(())
                }
                None => self.mirror.delete(&file(&event.id)).await,
            },
        }
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
