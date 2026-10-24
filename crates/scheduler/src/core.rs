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
use crate::link::{Booker, Booking, Digest, Limits, Link, Links, QUIET, State};
use crate::policy::Policy;
use crate::slots::{Period, days, open, reach, shown, slots};
use crate::stalwart::{Calendar, Stalwart, event};
use crate::store::Store;

const CACHED: i64 = 60;

type Weeks = BTreeMap<Date, (Timestamp, Vec<(Period, usize)>)>;

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
        && std::fs::File::create_new(format!("/var/lib/scheduler/crashed-{_at}")).is_ok()
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
        let now = (self.clock)();
        let Some(Link { limits, state: State::Claiming(claim) }) =
            store.links().get(&digest).cloned()
        else {
            return Ok(());
        };
        if !claim.due(now) {
            return Ok(());
        }
        store.apply(|links| links.sending(digest, now))?;
        let created =
            self.calendar.create(event(self.policy, digest, &limits, &claim.booking)).await?;
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
        if days(self.policy, monday, monday).start > now + self.policy.horizon() {
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
        let label =
            |i: usize| self.policy.show_calendars.then(|| self.policy.calendars[i].label.clone());
        let open = open(self.policy, monday, now);
        let blocks = shown(&open, &busy).into_iter().map(|(p, i)| (p, label(i))).collect();
        Ok(FrontAnswer::Week { monday, blocks, open })
    }

    async fn free(&self, limits: &Limits, links: &Links) -> Result<Vec<Timestamp>> {
        let busy = self.calendar.busy(reach(self.policy, limits)).await?;
        let busy: Vec<Period> = busy.into_iter().map(|(p, _)| p).collect();
        Ok(slots(self.policy, limits, (self.clock)(), &busy, &links.taken()))
    }

    async fn link(&self, digest: Digest) -> Result<FrontAnswer> {
        let links = self.store.lock().await.links().clone();
        let Some(link) = links.get(&digest).filter(|l| l.limits.until >= self.today()) else {
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
        let Some(link) =
            store.links().get(&digest).filter(|l| l.limits.until >= self.today()).cloned()
        else {
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
            },
            _ => FrontAnswer::Refused(Refusal::SlotGone),
        })
    }

    pub async fn admin(&self, ask: AdminAsk) -> AdminAnswer {
        let mut store = self.store.lock().await;
        let changed = store.apply(|links| match ask {
            AdminAsk::Issue { digest, limits } => links.issue(digest, Checked::new(limits)?),
            AdminAsk::Revoke(id) => links.revoke(id),
            AdminAsk::List => Ok(links.clone()),
        });
        changed.map_err(|e| format!("{e:#}"))?;
        let state = |s: &State| match s {
            State::Open => "open".to_string(),
            State::Claiming(c) => format!("claiming {}, sent {} times", c.booking.start, c.sends),
            State::Booked { booking, .. } => {
                format!("booked {} by {}", booking.start, booking.booker.address)
            }
        };
        Ok(store
            .links()
            .iter()
            .map(|(d, l)| (d.id(), l.limits.clone().into_inner(), state(&l.state)))
            .collect())
    }
}

pub async fn run(policy: &'static Policy, mut sockets: Sockets) -> Result<()> {
    let credentials = PathBuf::from(
        std::env::var_os("CREDENTIALS_DIRECTORY")
            .context("the app passwords, by LoadCredential")?,
    );
    let state = PathBuf::from(std::env::var_os("STATE_DIRECTORY").context("StateDirectory")?);
    let stalwart = Stalwart::connect(policy, &credentials).await?;
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
