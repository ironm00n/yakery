//! When a booking may start, and what the public week shows: pure functions of the policy, a
//! link's limits, the clock and busy time.

use jiff::civil::{Date, DateTime, DateTimeRound, Time};
use jiff::{RoundMode, SignedDuration, Timestamp, ToSpan, Unit};
use serde::{Deserialize, Serialize};

use crate::link::Limits;
use crate::policy::{Gap, Policy, Window};

const GRID_MINUTES: i64 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Period {
    pub start: Timestamp,
    pub end: Timestamp,
}

impl Period {
    pub fn starting(start: Timestamp, length: SignedDuration) -> Period {
        Period { start, end: start + length }
    }

    pub fn padded(self, by: SignedDuration) -> Period {
        Period { start: self.start - by, end: self.end + by }
    }

    pub fn overlaps(self, other: Period) -> bool {
        self.start < other.end && other.start < self.end
    }

    fn clip(self, to: Period) -> Option<Period> {
        let clipped = Period { start: self.start.max(to.start), end: self.end.min(to.end) };
        (clipped.start < clipped.end).then_some(clipped)
    }
}

/// A booking the core already holds; a new one keeps the larger of their two gaps from it.
#[derive(Clone, Copy, Debug)]
pub struct Taken {
    pub period: Period,
    pub gap: Gap,
}

fn windows(
    hours: &[Window],
    from: Date,
    until: Date,
) -> impl Iterator<Item = (DateTime, DateTime)> {
    from.series(1.day()).take_while(move |day| *day <= until).flat_map(move |day| {
        hours
            .iter()
            .filter(move |w| w.day == day.weekday())
            .map(move |w| (day.to_datetime(w.from), day.to_datetime(w.to)))
    })
}

/// The gap a link keeps: its own, or the policy's when that is larger.
fn gap(policy: &Policy, limits: &Limits) -> Gap {
    limits.gap.max(policy.gap)
}

/// What a link's busy time is read across: its days, midnight to midnight, widened by its gap.
pub fn reach(policy: &Policy, limits: &Limits) -> Period {
    days(policy, limits.from, limits.until).padded(gap(policy, limits).get())
}

/// The grid's starts in one window that are unambiguous in the owner's zone and end, in real
/// time, inside it, as they don't across the hour the clocks skip.
fn starts(
    policy: &Policy,
    (from, to): (DateTime, DateTime),
    length: SignedDuration,
) -> impl Iterator<Item = Timestamp> {
    let grid =
        DateTimeRound::new().smallest(Unit::Minute).increment(GRID_MINUTES).mode(RoundMode::Ceil);
    let end = policy.zone.to_timestamp(to).ok();
    let civil = from.round(grid).ok().into_iter();
    civil
        .flat_map(move |first| first.series(GRID_MINUTES.minutes()).take_while(move |s| *s < to))
        .filter_map(|s| policy.zone.to_ambiguous_timestamp(s).unambiguous().ok())
        .filter(move |&start| end.is_some_and(|end| start + length <= end))
}

/// Every start the link may book: in its hours and window, past the notice, and the gap clear
/// of busy time and of what is taken.
pub fn slots(
    policy: &Policy,
    limits: &Limits,
    now: Timestamp,
    busy: &[Period],
    taken: &[Taken],
) -> Vec<Timestamp> {
    let (length, gap) = (limits.duration, gap(policy, limits));
    let clear = |slot: Period| {
        busy.iter()
            .map(|b| b.padded(gap.get()))
            .chain(taken.iter().map(|t| t.period.padded(t.gap.max(gap).get())))
            .all(|b| !b.overlaps(slot))
    };
    let hours = if limits.hours.is_empty() { &policy.hours } else { &limits.hours };
    windows(hours, limits.from, limits.until)
        .flat_map(|window| starts(policy, window, length))
        .filter(|&start| start >= now + policy.notice && clear(Period::starting(start, length)))
        .collect()
}

fn midnight(policy: &Policy, day: Date) -> Timestamp {
    policy
        .zone
        .to_timestamp(day.to_datetime(Time::midnight()))
        .expect("the scheduler's dates are in range")
}

/// The days `from..=until` in the owner's zone, midnight to midnight.
pub fn days(policy: &Policy, from: Date, until: Date) -> Period {
    Period { start: midnight(policy, from), end: midnight(policy, until + 1.day()) }
}

/// The week's bookable hours from `monday`, inside the horizon.
pub fn open(policy: &Policy, monday: Date, now: Timestamp) -> Vec<Period> {
    let horizon = Period { start: now, end: now + policy.horizon() };
    windows(&policy.hours, monday, monday + 6.days())
        .filter_map(|(from, to)| {
            let window = Period {
                start: policy.zone.to_timestamp(from).ok()?,
                end: policy.zone.to_timestamp(to).ok()?,
            };
            window.clip(horizon)
        })
        .collect()
}

/// Busy time as the public page may show it: clipped to `open`.
pub fn shown<L: Copy>(open: &[Period], busy: &[(Period, L)]) -> Vec<(Period, L)> {
    busy.iter()
        .flat_map(|&(b, label)| open.iter().filter_map(move |w| Some((b.clip(*w)?, label))))
        .collect()
}

/// Splits a read into pieces of at most a week, as Stalwart fails an answer with too many
/// instances in it.
pub fn weeks(period: Period) -> impl Iterator<Item = Period> {
    let week = SignedDuration::from_hours(24 * 7);
    std::iter::successors(Some(period.start), move |&s| Some(s + week))
        .take_while(move |&s| s < period.end)
        .map(move |start| Period { start, end: (start + week).min(period.end) })
}

#[cfg(test)]
#[path = "../tests/slots.rs"]
mod tests;
