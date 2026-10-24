use std::cell::{Cell, RefCell};
use std::rc::Rc;

use jiff::SignedDuration;
use serde_json::Value;

use super::*;
use crate::checked::Checked;
use crate::link::tests::issue;
use crate::link::{Created, Form, Issue, Token};
use crate::policy::Gap;
use crate::policy::tests::{fixture, leaked};

/// Stalwart as the core sees it: the busy time overlapping what is asked, and creates that
/// refuse a UID it holds. `lose` drops the next create's answer, with the request (`true`) or
/// after it landed (`false`), as a timeout or a crash would; `refuse` refuses the next create.
#[derive(Default)]
struct Fake {
    busy: Vec<(Period, usize)>,
    events: RefCell<Vec<Value>>,
    lose: Cell<Option<bool>>,
    refuse: Cell<bool>,
}

impl Calendar for &Fake {
    async fn busy(&self, period: Period) -> Result<Vec<(Period, usize)>> {
        Ok(self.busy.iter().filter(|(b, _)| b.overlaps(period)).cloned().collect())
    }

    async fn create(&self, event: Value) -> Result<Created> {
        if self.refuse.take() {
            return Ok(Created::Refused("forbidden".into()));
        }
        let lost = self.lose.take();
        anyhow::ensure!(lost != Some(true), "lost before Stalwart");
        let taken = self.events.borrow().iter().any(|e| e["uid"] == event["uid"]);
        if !taken {
            self.events.borrow_mut().push(event);
        }
        anyhow::ensure!(lost.is_none(), "lost after Stalwart");
        Ok(if taken { Created::UidTaken } else { Created::Event("e".into()) })
    }
}

macro_rules! answers {
    ($answer:expr, $pattern:pat $(if $guard:expr)?) => {
        let answer = $answer;
        assert!(matches!(answer, $pattern $(if $guard)?), "not {}", stringify!($pattern));
    };
}

fn token(c: char) -> Token {
    Token::parse(&c.to_string().repeat(64)).unwrap()
}

fn booker() -> Booker {
    let form = Form { name: "Ada".into(), address: "ada@example.org".into(), note: "".into() };
    Checked::new(form).unwrap()
}

fn at(local: &str) -> Timestamp {
    fixture().zone.to_timestamp(local.parse().unwrap()).unwrap()
}

fn monday(time: &str) -> Timestamp {
    at(&format!("2026-10-26T{time}"))
}

/// A links file and a clock, which every core a test starts over them shares.
struct Setup {
    dir: PathBuf,
    clock: Rc<Cell<Timestamp>>,
}

impl Setup {
    fn new(name: &str) -> Setup {
        let dir = std::env::temp_dir().join(format!("scheduler-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Setup { dir, clock: Rc::new(Cell::new(at("2026-10-23T09:00"))) }
    }

    /// A core as one started afresh would be.
    fn core<'f>(&self, fake: &'f Fake, policy: Policy) -> Core<&'f Fake> {
        let (store, clock) = (Store::open(&self.dir).unwrap(), self.clock.clone());
        Core::new(fake, store, leaked(policy), move || clock.get())
    }

    fn later(&self, by: SignedDuration) {
        self.clock.set(self.clock.get() + by);
    }
}

async fn issued(core: &Core<&Fake>, c: char, issue: Issue) {
    core.admin(AdminAsk::Issue { digest: token(c).digest(), limits: issue }).await.unwrap();
}

async fn book_by(core: &Core<&Fake>, c: char, start: Timestamp, by: Timestamp) -> FrontAnswer {
    core.front(FrontAsk::Book { token: token(c), start, booker: booker(), by }).await
}

async fn book(core: &Core<&Fake>, c: char, start: Timestamp) -> FrontAnswer {
    book_by(core, c, start, (core.clock)() + ask::CLAIMED_WITHIN).await
}

async fn shown(core: &Core<&Fake>, c: char) -> Shown {
    let FrontAnswer::Link(view) = core.front(FrontAsk::Link(token(c))).await else { panic!() };
    view.state
}

fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(test)
}

const WEEK: (&str, &str) = ("2026-10-26", "2026-10-30");

#[test]
fn a_link_books_one_event_and_then_shows_only_its_time() {
    run(async {
        let (setup, fake) = (Setup::new("once"), Fake::default());
        let core = setup.core(&fake, fixture());
        issued(&core, 'a', issue(WEEK.0, WEEK.1)).await;
        answers!(book(&core, 'a', monday("10:00")).await, FrontAnswer::Booked { .. });
        answers!(book(&core, 'a', monday("11:00")).await, FrontAnswer::Refused(Refusal::Used));
        let uid = format!("{}@schedule.example.test", token('a').digest());
        assert_eq!(fake.events.borrow().len(), 1);
        assert_eq!(fake.events.borrow()[0]["uid"], uid);
        answers!(shown(&core, 'a').await, Shown::Booked(start) if start == monday("10:00"));
    })
}

#[test]
fn a_slot_off_the_link_or_taken_by_another_link_is_refused() {
    run(async {
        let (setup, fake) = (Setup::new("taken"), Fake::default());
        let core = setup.core(&fake, fixture());
        issued(&core, 'a', issue(WEEK.0, WEEK.1)).await;
        issued(&core, 'b', issue(WEEK.0, WEEK.1)).await;
        answers!(book(&core, 'a', monday("10:05")).await, FrontAnswer::Refused(Refusal::SlotGone));
        answers!(book(&core, 'a', monday("10:00")).await, FrontAnswer::Booked { .. });
        answers!(book(&core, 'b', monday("10:30")).await, FrontAnswer::Refused(Refusal::SlotGone));
        answers!(book(&core, 'b', monday("10:45")).await, FrontAnswer::Booked { .. });
        answers!(
            book(&core, 'c', monday("12:00")).await,
            FrontAnswer::Refused(Refusal::NoSuchLink)
        );
    })
}

#[test]
fn a_booking_taken_after_its_deadline_claims_nothing() {
    run(async {
        let (setup, fake) = (Setup::new("late"), Fake::default());
        let core = setup.core(&fake, fixture());
        issued(&core, 'a', issue(WEEK.0, WEEK.1)).await;
        let by = (core.clock)();
        setup.later(SignedDuration::from_secs(1));
        answers!(
            book_by(&core, 'a', monday("10:00"), by).await,
            FrontAnswer::Refused(Refusal::Unavailable)
        );
        answers!(shown(&core, 'a').await, Shown::Open(_));
        core.reconcile().await;
        assert!(fake.events.borrow().is_empty());
    })
}

#[test]
fn a_create_lost_on_the_way_is_sent_again_once_its_send_is_quiet() {
    run(async {
        let (setup, fake) = (Setup::new("before"), Fake::default());
        let core = setup.core(&fake, fixture());
        issued(&core, 'a', issue(WEEK.0, WEEK.1)).await;
        fake.lose.set(Some(true));
        answers!(book(&core, 'a', monday("10:00")).await, FrontAnswer::Refused(Refusal::Pending));
        answers!(book(&core, 'a', monday("11:00")).await, FrontAnswer::Refused(Refusal::Pending));
        core.reconcile().await;
        assert!(fake.events.borrow().is_empty(), "the first send may still be in flight");
        setup.later(QUIET);
        core.reconcile().await;
        assert_eq!(fake.events.borrow().len(), 1);
        assert_eq!(fake.events.borrow()[0]["start"], "2026-10-26T10:00:00", "the slot first asked");
        answers!(shown(&core, 'a').await, Shown::Booked(_));
    })
}

#[test]
fn a_create_whose_answer_was_lost_is_booked_by_its_taken_uid_after_a_restart() {
    run(async {
        let (setup, fake) = (Setup::new("after"), Fake::default());
        issued(&setup.core(&fake, fixture()), 'a', issue(WEEK.0, WEEK.1)).await;
        fake.lose.set(Some(false));
        let first = setup.core(&fake, fixture());
        answers!(book(&first, 'a', monday("10:00")).await, FrontAnswer::Refused(Refusal::Pending));
        drop(first);
        setup.later(QUIET);
        let core = setup.core(&fake, fixture());
        core.reconcile().await;
        assert_eq!(fake.events.borrow().len(), 1);
        let links = core.store.lock().await.links().clone();
        answers!(
            &links.get(&token('a').digest()).unwrap().state,
            State::Booked { event: None, .. }
        );
    })
}

#[test]
fn a_refusal_reopens_a_link_only_when_it_answers_the_one_send() {
    run(async {
        let (setup, fake) = (Setup::new("refused"), Fake::default());
        let core = setup.core(&fake, fixture());
        issued(&core, 'a', issue(WEEK.0, WEEK.1)).await;
        fake.refuse.set(true);
        answers!(book(&core, 'a', monday("10:00")).await, FrontAnswer::Refused(Refusal::SlotGone));
        answers!(shown(&core, 'a').await, Shown::Open(_));
        fake.lose.set(Some(false));
        answers!(book(&core, 'a', monday("10:00")).await, FrontAnswer::Refused(Refusal::Pending));
        setup.later(QUIET);
        fake.refuse.set(true);
        core.reconcile().await;
        answers!(shown(&core, 'a').await, Shown::Pending);
        setup.later(QUIET);
        core.reconcile().await;
        answers!(shown(&core, 'a').await, Shown::Booked(_));
        assert_eq!(fake.events.borrow().len(), 1);
    })
}

#[test]
fn busy_time_is_read_as_far_as_the_policy_gap_reaches() {
    run(async {
        let busy = vec![(Period { start: at("2026-10-31T00:10"), end: at("2026-10-31T00:40") }, 0)];
        let (setup, fake) = (Setup::new("reach"), Fake { busy, ..Fake::default() });
        let gap: Gap = "PT30M".parse().unwrap();
        let core = setup.core(&fake, Policy { gap, ..fixture() });
        let hours = vec!["fri 20:00-23:45".parse().unwrap()];
        issued(&core, 'a', Issue { hours, ..issue("2026-10-30", "2026-10-30") }).await;
        let Shown::Open(slots) = shown(&core, 'a').await else { panic!() };
        assert_eq!(slots.last(), Some(&at("2026-10-30T23:00")));
    })
}

#[test]
fn the_week_drops_labels_unless_shown() {
    run(async {
        let busy = vec![(Period { start: monday("09:00"), end: monday("11:00") }, 1)];
        let (setup, fake) = (Setup::new("week"), Fake { busy, ..Fake::default() });
        let hour = Period { start: monday("10:00"), end: monday("11:00") };
        for (show, label) in [(true, Some("Personal".to_string())), (false, None)] {
            let core = setup.core(&fake, Policy { show_calendars: show, ..fixture() });
            let FrontAnswer::Week { blocks, .. } = core.front(FrontAsk::Week(1)).await else {
                panic!()
            };
            assert_eq!(blocks, [(hour, label)]);
            answers!(
                core.front(FrontAsk::Week(5)).await,
                FrontAnswer::Refused(Refusal::NoSuchWeek)
            );
        }
    })
}
