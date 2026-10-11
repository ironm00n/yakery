//! Every page, as maud markup: no script anywhere, a `<dialog open>` the server sends for the
//! booking form, and the viewer's zone carried in `?tz=`.

use std::collections::BTreeSet;
use std::iter;
use std::sync::LazyLock;

use jiff::civil::{Date, Time, TimeRound};
use jiff::tz::TimeZone;
use jiff::{RoundMode, SignedDuration, Timestamp, ToSpan, Unit, Zoned};
use maud::{DOCTYPE, Markup, html};
use text::Address;

use crate::ask::{LinkView, Shown};
use crate::link::{Booker, Form, MOST_NAME, MOST_NOTE};
use crate::policy::POLICY;
use crate::slots::{Period, days};

static ZONES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut zones: Vec<String> = jiff::tz::db().available().map(|z| z.to_string()).collect();
    zones.sort();
    zones
});

const STYLE: &str = "body{font:16px/1.4 system-ui,sans-serif;max-width:60rem;margin:auto;padding:1rem}\
table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccc;padding:.1rem .3rem;font-size:.8rem}\
.busy{background:#c66;color:#fff}.closed{background:#eee}.slots a{display:inline-block;margin:.2rem}";

/// Who is looking, in which zone; `tz` is set only when they chose one.
pub struct Viewer {
    pub zone: TimeZone,
    pub tz: Option<String>,
}

impl Viewer {
    pub fn new(tz: Option<&str>) -> Viewer {
        match tz.and_then(|tz| jiff::tz::db().get(tz).ok().map(|zone| (tz, zone))) {
            Some((tz, zone)) => Viewer { zone, tz: Some(tz.into()) },
            None => Viewer { zone: POLICY.zone.clone(), tz: None },
        }
    }

    fn local(&self, t: Timestamp) -> Zoned {
        t.to_zoned(self.zone.clone())
    }

    fn zone_name(&self) -> &str {
        self.tz.as_deref().unwrap_or(POLICY.zone_name())
    }

    fn query(&self, pairs: &[(&str, String)]) -> String {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
        query.extend_pairs(self.tz.iter().map(|tz| ("tz", tz)));
        format!("?{}", query.finish())
    }

    fn time(&self, t: Timestamp) -> String {
        self.local(t).strftime("%H:%M").to_string()
    }

    fn when(&self, start: Timestamp, length: SignedDuration) -> String {
        format!(
            "{}–{} ({})",
            self.local(start).strftime("%A %-d %B, %H:%M"),
            self.time(start + length),
            self.zone_name()
        )
    }
}

fn page(viewer: &Viewer, title: &str, keep: &[(&str, String)], body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (STYLE) }
            }
            body {
                form method="get" {
                    "Times are in " (viewer.zone_name()) ". "
                    @for (name, value) in keep { input type="hidden" name=(name) value=(value); }
                    input name="tz" list="zones" value=(viewer.zone_name()) aria-label="Time zone";
                    datalist id="zones" { @for zone in ZONES.iter() { option value=(zone) {} } }
                    button { "Change zone" }
                }
                h1 { (title) }
                (body)
            }
        }
    }
}

pub fn message(viewer: &Viewer, title: &str, text: &str) -> Markup {
    page(viewer, title, &[], html! { p { (text) } })
}

pub fn week(
    viewer: &Viewer,
    n: u8,
    monday: Date,
    blocks: &[(Period, Option<String>)],
    open: &[Period],
) -> Markup {
    let half = SignedDuration::from_mins(30);
    let row = TimeRound::new().smallest(Unit::Minute).increment(30).mode(RoundMode::Trunc);
    let starts = |p: Period| {
        iter::successors(Some(p.start), move |t| Some(*t + half)).take_while(move |t| *t < p.end)
    };
    let rows: BTreeSet<Time> = open
        .iter()
        .flat_map(|p| starts(*p))
        .filter_map(|t| viewer.local(t).time().round(row).ok())
        .collect();
    let midnight = |day: Date| days(&POLICY, day, day).start;
    let days: Vec<Date> =
        (0..7).map(|d| viewer.local(midnight(monday + d.days())).date()).collect();
    let cell = |day: Date, row: Time| {
        Some(Period::starting(viewer.zone.to_timestamp(day.to_datetime(row)).ok()?, half))
    };
    let nav = |to: u8, text: &str| html! { a href=(viewer.query(&[("week", to.to_string())])) { (text) } " " };
    let body = html! {
        p {
            @if n > 0 { (nav(n - 1, "Earlier week")) }
            @if n < POLICY.horizon_weeks { (nav(n + 1, "Later week")) }
        }
        table {
            thead { tr { th {} @for day in &days { th { (day.strftime("%a %-d %b")) } } } }
            tbody {
                @for row in &rows {
                    tr {
                        th { (row.strftime("%H:%M")) }
                        @for day in &days {
                            @let slot = cell(*day, *row);
                            @match slot.and_then(|s| blocks.iter().find(|(b, _)| b.overlaps(s))) {
                                Some((_, label)) => td.busy { (label.as_deref().unwrap_or("busy")) },
                                None if slot.is_some_and(|s| open.iter().any(|o| o.overlaps(s))) => td {},
                                None => td.closed {},
                            }
                        }
                    }
                }
            }
        }
    };
    page(viewer, "When I'm busy", &[("week", n.to_string())], body)
}

/// A link's page; with `chosen`, the booking form for that slot in a `<dialog open>`.
pub fn link(
    viewer: &Viewer,
    view: &LinkView,
    chosen: Option<Timestamp>,
    form: &Form,
    problem: &str,
) -> Markup {
    let title = view.title.as_deref().unwrap_or("Book a meeting");
    let body = match &view.state {
        Shown::Booked(start) => {
            html! { p { "Booked for " (viewer.when(*start, view.duration)) "." } }
        }
        Shown::Pending => html! { p { "This link is being booked. Try again in a minute." } },
        Shown::Open(slots) if slots.is_empty() => {
            html! { p { "No times are free on this link any more." } }
        }
        Shown::Open(slots) => html! {
            @if !problem.is_empty() { p { strong { (problem) } } }
            p { (view.duration.as_mins()) " minutes. Choose a time:" }
            @for (day, starts) in by_day(viewer, slots) {
                h2 { (day.strftime("%A %-d %B")) }
                p.slots { @for s in starts { a href=(viewer.query(&[("slot", s.as_second().to_string())])) { (viewer.time(s)) } } }
            }
            @if let Some(start) = chosen.filter(|c| slots.contains(c)) {
                dialog open {
                    h2 { (viewer.when(start, view.duration)) }
                    form method="post" {
                        input type="hidden" name="slot" value=(start.as_second());
                        @if let Some(tz) = &viewer.tz { input type="hidden" name="tz" value=(tz); }
                        p { label { "Name " input name="name" required maxlength=(MOST_NAME) value=(form.name); } }
                        p { label { "Email " input name="address" type="email" required maxlength=(Address::MOST) value=(form.address); } }
                        p { label { "Note " textarea name="note" maxlength=(MOST_NOTE) { (form.note) } } }
                        button { "Book" }
                    }
                    form method="dialog" { button { "Back" } }
                }
            }
        },
    };
    page(viewer, title, &[], body)
}

fn by_day(viewer: &Viewer, slots: &[Timestamp]) -> Vec<(Date, Vec<Timestamp>)> {
    slots
        .chunk_by(|a, b| viewer.local(*a).date() == viewer.local(*b).date())
        .map(|c| (viewer.local(c[0]).date(), c.to_vec()))
        .collect()
}

pub fn booked(
    viewer: &Viewer,
    start: Timestamp,
    length: SignedDuration,
    booker: &Booker,
) -> Markup {
    let body = html! {
        p { (viewer.when(start, length)) }
        p { (booker.name) ", an invitation from " (POLICY.owner) " should reach " (booker.address) "." }
    };
    page(viewer, "Booked", &[], body)
}

#[cfg(test)]
#[path = "../tests/pages.rs"]
mod tests;
