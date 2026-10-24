//! What the scheduler books by, compiled in from the Nix bundle so the core and `scheduler link`
//! can't disagree on it.

use std::sync::LazyLock;

use anyhow::{Context, ensure};
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;
use serde::{Deserialize, Deserializer};
use text::{Address, text};

/// Time kept free between a booking and any other event: 5 minutes at least.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Gap(SignedDuration);

impl Gap {
    pub const FLOOR: Gap = Gap(SignedDuration::from_mins(5));

    pub fn get(self) -> SignedDuration {
        self.0
    }
}

const DAYS: [(&str, Weekday); 7] = [
    ("mon", Weekday::Monday),
    ("tue", Weekday::Tuesday),
    ("wed", Weekday::Wednesday),
    ("thu", Weekday::Thursday),
    ("fri", Weekday::Friday),
    ("sat", Weekday::Saturday),
    ("sun", Weekday::Sunday),
];

/// Bookable hours on one weekday, in the owner's zone: `mon 10:00-18:00`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub day: Weekday,
    pub from: Time,
    pub to: Time,
}

fn window(s: &str) -> Option<Window> {
    let (day, hours) = s.split_once(' ')?;
    let (from, to) = hours.split_once('-')?;
    let day = DAYS.iter().find(|(name, _)| *name == day)?.1;
    Some(Window { day, from: from.parse().ok()?, to: to.parse().ok()? })
}

text! {
    Gap(|s| {
        let gap = Gap(s.parse()?);
        ensure!(gap >= Gap::FLOOR, "a gap under 5 minutes would allow back-to-back events");
        Ok(gap)
    }, |g| g.0);
    Window(|s| {
        let w = window(s).with_context(|| format!("{s:?} is not like \"mon 10:00-18:00\""))?;
        ensure!(w.from < w.to, "{s:?} ends before it starts");
        Ok(w)
    }, |w| {
        let day = DAYS.iter().find(|(_, d)| *d == w.day).unwrap().0;
        format!("{day} {}-{}", w.from.strftime("%H:%M"), w.to.strftime("%H:%M"))
    });
}

#[derive(Debug, Deserialize)]
pub struct Counted {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub owner: Address,
    pub host: String,
    pub stalwart: String,
    #[serde(deserialize_with = "zone")]
    pub zone: TimeZone,
    pub hours: Vec<Window>,
    pub notice: SignedDuration,
    pub horizon_weeks: u8,
    pub gap: Gap,
    pub calendars: Vec<Counted>,
    pub show_calendars: bool,
}

fn zone<'de, D: Deserializer<'de>>(d: D) -> Result<TimeZone, D::Error> {
    jiff::tz::db().get(&String::deserialize(d)?).map_err(serde::de::Error::custom)
}

impl Policy {
    pub fn horizon(&self) -> SignedDuration {
        SignedDuration::from_hours(24 * 7 * self.horizon_weeks as i64)
    }

    pub fn zone_name(&self) -> &str {
        self.zone.iana_name().expect("zones come from the tz database by name")
    }
}

pub static POLICY: LazyLock<Policy> = LazyLock::new(|| {
    serde_json::from_str(env!("SCHEDULER_POLICY")).expect("the bundle's policy parses")
});

#[cfg(test)]
#[path = "../tests/policy.rs"]
pub(crate) mod tests;
