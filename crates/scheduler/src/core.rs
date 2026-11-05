//! The core: the only holder of the app passwords and the links. It answers the front with no
//! more than the public page or a link's own page shows, and books under one lock, from checking
//! the link to the last write of the links file.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use jiff::civil::Date;
use jiff::{Timestamp, ToSpan};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use crate::Sockets;
use crate::ask::{self, AdminAnswer, AdminAsk, FrontAnswer, FrontAsk, LinkView, Refusal, Shown};
use crate::checked::Checked;
use crate::link::{Booker, Booking, Digest, Limits, Links, QUIET, State};
use crate::policy::Policy;
use crate::slots::{Period, days, open, reach, shown, slots};
use crate::stalwart::{Calendar, Stalwart, event};
use crate::store::Store;

const CACHED: i64 = 60;

type Weeks = BTreeMap<Date, (Timestamp, Vec<Period>)>;

pub struct Core<C> {
    calendar: C,
    store: Mutex<Store>,
    policy: &'static Policy,
    clock: Box<dyn Fn() -> Timestamp>,
    weeks: RefCell<Weeks>,
}

fn crash_point(_at: &str) {
    #[cfg(feature = "crash-points")]
    if std::env::var("SCHEDULER_CRASH").as_deref() == Ok(_at)
        && let Some(state) = std::env::var_os("STATE_DIRECTORY")
        && std::fs::File::create_new(std::path::Path::new(&state).join(format!("crashed-{_at}")))
            .is_ok()
    {
        std::process::abort();
    }
}

impl<C: Calendar> Core<C> {
    pub fn new(
        calendar: C,
        store: Store,
        policy: &'static Policy,
        clock: impl Fn() -> Timestamp + 'static,
    ) -> Core<C> {
        let (store, clock) = (Mutex::new(store), Box::new(clock));
        Core { calendar, store, policy, clock, weeks: RefCell::default() }
    }

    fn today(&self) -> Date {
        (self.clock)().to_zoned(self.policy.zone.clone()).date()
    }

    /// Sends every claim that is due, each on its own, so a link Stalwart refuses or can't be
    /// reached for leaves the others; at start, before the core answers anything, and then
    /// every [`QUIET`].
    pub async fn reconcile(&self) {
        let mut store = self.store.lock().await;
        if let Err(e) = store.apply(|links| Ok(links.expire(self.today()))) {
            yakery_log::warn!("expiring links: {e:#}");
        }
        for digest in store.links().claiming() {
            if let Err(e) = self.send(&mut store, digest).await {
                yakery_log::warn!("link {}: {e:#}", digest.id());
            }
        }
    }

    /// Sends the link's create if it is claiming and due, and settles it on Stalwart's answer.
    async fn send(&self, store: &mut Store, digest: Digest) -> Result<()> {
        let Some((next, limits, claim)) = store.links().sending(digest, (self.clock)()) else {
            return Ok(());
        };
        store.apply(|_| Ok(next))?;
        let event = event(self.policy, self.calendar.owner(), digest, &limits, &claim.booking);
        let created = self.calendar.create(event).await?;
        crash_point("created");
        store.apply(|links| Ok(links.settle(digest, created)))
    }

    pub async fn front(&self, ask: FrontAsk) -> FrontAnswer {
        let answer = match ask {
            FrontAsk::Week(n) => self.week(n).await,
            FrontAsk::Link(token) => self.link(token.digest()).await,
            FrontAsk::Book { token, start, booker, by } => {
                self.book(token.digest(), start, booker, by).await
            }
        };
        answer.unwrap_or_else(|e| {
            yakery_log::warn!("{e:#}");
            FrontAnswer::Refused(Refusal::Unavailable)
        })
    }

    async fn week(&self, n: u8) -> Result<FrontAnswer> {
        let (now, today) = ((self.clock)(), self.today());
        let monday =
            today - (today.weekday().to_monday_zero_offset() as i64).days() + (7 * n as i64).days();
        let served =
            |monday: Date| days(self.policy, monday, monday).start <= now + self.policy.horizon();
        if !served(monday) {
            return Ok(FrontAnswer::Refused(Refusal::NoSuchWeek));
        }
        let fresh = self
            .weeks
            .borrow()
            .get(&monday)
            .filter(|(at, _)| now < *at + CACHED.seconds())
            .map(|(_, b)| b.clone());
        let busy = match fresh {
            Some(busy) => busy,
            None => {
                let busy = self.calendar.busy(days(self.policy, monday, monday + 6.days())).await?;
                self.weeks.borrow_mut().insert(monday, (now, busy.clone()));
                busy
            }
        };
        let open = open(self.policy, monday, now);
        Ok(FrontAnswer::Week {
            monday,
            blocks: shown(&open, &busy),
            open,
            later: served(monday + 7.days()),
        })
    }

    async fn free(&self, limits: &Limits, links: &Links) -> Result<Vec<Timestamp>> {
        let busy = self.calendar.busy(reach(self.policy, limits)).await?;
        Ok(slots(self.policy, limits, (self.clock)(), &busy, &links.taken()))
    }

    async fn link(&self, digest: Digest) -> Result<FrontAnswer> {
        let links = self.store.lock().await.links().clone();
        let Some(link) = links.live(&digest, self.today()) else {
            return Ok(FrontAnswer::Refused(Refusal::NoSuchLink));
        };
        let state = match &link.state {
            State::Open => Shown::Open(self.free(&link.limits, &links).await?),
            State::Claiming(_) => Shown::Pending,
            State::Booked { booking, .. } => Shown::Booked(booking.start),
        };
        Ok(FrontAnswer::Link(LinkView {
            title: link.limits.title.clone(),
            duration: link.limits.duration,
            state,
        }))
    }

    async fn book(
        &self,
        digest: Digest,
        start: Timestamp,
        booker: Booker,
        by: Timestamp,
    ) -> Result<FrontAnswer> {
        let mut store = self.store.lock().await;
        self.send(&mut store, digest).await?;
        let Some(link) = store.links().live(&digest, self.today()).cloned() else {
            return Ok(FrontAnswer::Refused(Refusal::NoSuchLink));
        };
        match link.state {
            State::Open => {}
            State::Claiming(_) => return Ok(FrontAnswer::Refused(Refusal::Pending)),
            State::Booked { .. } => return Ok(FrontAnswer::Refused(Refusal::Used)),
        }
        if !self.free(&link.limits, store.links()).await?.contains(&start) {
            return Ok(FrontAnswer::Refused(Refusal::SlotGone));
        }
        if (self.clock)() > by {
            return Ok(FrontAnswer::Refused(Refusal::Unavailable));
        }
        let booking = Booking { start, booker };
        store.apply(|links| links.claim(digest, booking.clone()))?;
        crash_point("claimed");
        if let Err(e) = self.send(&mut store, digest).await {
            yakery_log::warn!("link {}: {e:#}", digest.id());
            return Ok(FrontAnswer::Refused(Refusal::Pending));
        }
        Ok(match store.links().get(&digest).map(|l| &l.state) {
            Some(State::Booked { .. }) => FrontAnswer::Booked {
                start,
                duration: link.limits.duration,
                booker: booking.booker,
                owner: self.calendar.owner().into(),
            },
            _ => FrontAnswer::Refused(Refusal::SlotGone),
        })
    }

    pub async fn admin(&self, ask: AdminAsk) -> AdminAnswer {
        let mut store = self.store.lock().await;
        let changed = match ask {
            AdminAsk::Issue { digest, limits } => {
                store.apply(|links| links.issue(digest, Checked::new(limits)?))
            }
            AdminAsk::Revoke(id) => store.apply(|links| links.revoke(id)),
            AdminAsk::List => Ok(()),
        };
        changed.map_err(|e| format!("{e:#}"))?;
        Ok(store
            .links()
            .iter()
            .map(|(d, l)| (d.id(), l.limits.clone().into_inner(), l.state.to_string()))
            .collect())
    }
}

pub async fn run(policy: &'static Policy, mut sockets: Sockets) -> Result<()> {
    let credentials = PathBuf::from(
        std::env::var_os("CREDENTIALS_DIRECTORY").context("the app password, by LoadCredential")?,
    );
    let state = PathBuf::from(std::env::var_os("STATE_DIRECTORY").context("StateDirectory")?);
    let secret = credentials.join(policy.account.address.to_string());
    let stalwart = Stalwart::connect(policy, &secret).await?;
    let core: &'static Core<Stalwart> =
        Box::leak(Box::new(Core::new(stalwart, Store::open(&state)?, policy, Timestamp::now)));
    core.reconcile().await;
    tokio::task::spawn_local(async move {
        let mut every = tokio::time::interval(QUIET.unsigned_abs());
        loop {
            every.tick().await;
            core.reconcile().await;
        }
    });
    let mut listener = |name: &str| -> Result<UnixListener> {
        let listener = std::os::unix::net::UnixListener::from(sockets.take(name)?);
        listener.set_nonblocking(true)?;
        Ok(UnixListener::from_std(listener)?)
    };
    let (front, admin) = (listener("front")?, listener("admin")?);
    tokio::task::spawn_local(ask::serve(admin, move |a| core.admin(a)));
    ask::serve(front, move |a| core.front(a)).await;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/core.rs"]
mod tests;
