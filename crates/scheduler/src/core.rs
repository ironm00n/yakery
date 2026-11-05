//! The core: the only holder of the app passwords and the links. It answers the front with no
//! more than the public page or a link's own page shows, and books under one lock, from checking
//! the link to the last write of the links file.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use jiff::civil::Date;
use jiff::{Timestamp, ToSpan};
use serde_json::Value;
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use crate::Sockets;
use crate::ask::{
    self, AdminAnswer, AdminAsk, Answer, BookAsk, Booked, FrontAsk, LinkAsk, LinkView, Question,
    Refusal, Shown, Week, WeekAsk,
};
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

    /// Each claim goes on its own, so one Stalwart refuses or can't be reached for leaves the others.
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

    /// An error answers Unavailable.
    pub async fn ask<Q: Question>(&self, question: Q) -> Answer<Q::Answer>
    where
        Self: Answers<Q>,
    {
        self.answer(question).await.unwrap_or_else(|e| {
            yakery_log::warn!("{e:#}");
            Err(Refusal::Unavailable)
        })
    }

    pub async fn front(&self, ask: FrontAsk) -> Value {
        match ask {
            FrontAsk::Week(question) => wire(self.ask(question).await),
            FrontAsk::Link(question) => wire(self.ask(question).await),
            FrontAsk::Book(question) => wire(self.ask(question).await),
        }
    }

    async fn free(&self, limits: &Limits, links: &Links) -> Result<Vec<Timestamp>> {
        let busy = self.calendar.busy(reach(self.policy, limits)).await?;
        Ok(slots(self.policy, limits, (self.clock)(), &busy, &links.taken()))
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

fn wire(answer: impl serde::Serialize) -> Value {
    serde_json::to_value(answer).expect("answers serialize")
}

pub trait Answers<Q: Question> {
    async fn answer(&self, question: Q) -> Result<Answer<Q::Answer>>;
}

impl<C: Calendar> Answers<WeekAsk> for Core<C> {
    async fn answer(&self, WeekAsk(n): WeekAsk) -> Result<Answer<Week>> {
        let (now, today) = ((self.clock)(), self.today());
        let monday =
            today - (today.weekday().to_monday_zero_offset() as i64).days() + (7 * n as i64).days();
        let served =
            |monday: Date| days(self.policy, monday, monday).start <= now + self.policy.horizon();
        if !served(monday) {
            return Ok(Err(Refusal::NoSuchWeek));
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
        Ok(Ok(Week { monday, blocks: shown(&open, &busy), open, later: served(monday + 7.days()) }))
    }
}

impl<C: Calendar> Answers<LinkAsk> for Core<C> {
    async fn answer(&self, LinkAsk(token): LinkAsk) -> Result<Answer<LinkView>> {
        let digest = token.digest();
        let links = self.store.lock().await.links().clone();
        let Some(link) = links.live(&digest, self.today()) else {
            return Ok(Err(Refusal::NoSuchLink));
        };
        let state = match &link.state {
            State::Open => Shown::Open(self.free(&link.limits, &links).await?),
            State::Claiming(_) => Shown::Pending,
            State::Booked { booking, .. } => Shown::Booked(booking.start),
        };
        Ok(Ok(LinkView { title: link.limits.title.clone(), duration: link.limits.duration, state }))
    }
}

impl<C: Calendar> Answers<BookAsk> for Core<C> {
    async fn answer(&self, ask: BookAsk) -> Result<Answer<Booked>> {
        let BookAsk { token, start, booker, by } = ask;
        let digest = token.digest();
        let mut store = self.store.lock().await;
        self.send(&mut store, digest).await?;
        let Some(link) = store.links().live(&digest, self.today()).cloned() else {
            return Ok(Err(Refusal::NoSuchLink));
        };
        match link.state {
            State::Open => {}
            State::Claiming(_) => return Ok(Err(Refusal::Pending)),
            State::Booked { .. } => return Ok(Err(Refusal::Used)),
        }
        if !self.free(&link.limits, store.links()).await?.contains(&start) {
            return Ok(Err(Refusal::SlotGone));
        }
        if (self.clock)() > by {
            return Ok(Err(Refusal::Unavailable));
        }
        let booking = Booking { start, booker };
        store.apply(|links| links.claim(digest, booking.clone()))?;
        crash_point("claimed");
        if let Err(e) = self.send(&mut store, digest).await {
            yakery_log::warn!("link {}: {e:#}", digest.id());
            return Ok(Err(Refusal::Pending));
        }
        Ok(match store.links().get(&digest).map(|l| &l.state) {
            Some(State::Booked { .. }) => Ok(Booked {
                start,
                duration: link.limits.duration,
                booker: booking.booker,
                owner: self.calendar.owner().into(),
            }),
            _ => Err(Refusal::SlotGone),
        })
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
    let (front, admin) = (listener(&policy.layout.front.fd)?, listener(&policy.layout.admin.fd)?);
    tokio::task::spawn_local(ask::serve(admin, move |a| core.admin(a)));
    ask::serve(front, move |a| core.front(a)).await;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/core.rs"]
mod tests;
