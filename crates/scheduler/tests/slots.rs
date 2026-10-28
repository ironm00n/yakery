use super::*;
use crate::checked::Checked;
use crate::link::Issue;
use crate::policy::tests::fixture;

fn at(local: &str) -> Timestamp {
    fixture().zone.to_timestamp(local.parse().unwrap()).unwrap()
}

fn limits(from: &str, until: &str, minutes: i64, gap: i64, hours: &[&str]) -> Limits {
    let issue = Issue {
        from: from.parse().unwrap(),
        until: until.parse().unwrap(),
        duration: SignedDuration::from_mins(minutes),
        gap: format!("PT{gap}M").parse().unwrap(),
        hours: hours.iter().map(|h| h.parse().unwrap()).collect(),
        title: None,
        location: None,
    };
    Checked::new(issue).unwrap()
}

fn busy(from: &str, to: &str) -> Period {
    Period { start: at(from), end: at(to) }
}

const MONDAY: &str = "2026-10-26";

fn free(limits: &Limits, busy: &[Period], taken: &[Taken]) -> Vec<Timestamp> {
    slots(&fixture(), limits, at("2026-10-23T09:00"), busy, taken)
}

#[test]
fn an_event_five_minutes_away_keeps_the_slot_and_four_drops_it() {
    let hour = limits(MONDAY, MONDAY, 60, 5, &["mon 11:00-12:00"]);
    assert_eq!(
        free(&hour, &[busy("2026-10-26T10:00", "2026-10-26T10:55")], &[]),
        [at("2026-10-26T11:00")]
    );
    assert_eq!(free(&hour, &[busy("2026-10-26T10:00", "2026-10-26T10:56")], &[]), []);
    assert_eq!(
        free(&hour, &[busy("2026-10-26T12:05", "2026-10-26T13:00")], &[]),
        [at("2026-10-26T11:00")]
    );
    assert_eq!(free(&hour, &[busy("2026-10-26T12:04", "2026-10-26T13:00")], &[]), []);
}

#[test]
fn bookings_keep_the_larger_gap_and_never_touch() {
    let short = limits(MONDAY, MONDAY, 30, 5, &["mon 11:00-12:00"]);
    let booked = |gap| Taken {
        period: busy("2026-10-26T10:30", "2026-10-26T11:00"),
        gap: format!("PT{gap}M").parse().unwrap(),
    };
    assert_eq!(free(&short, &[], &[booked(5)]), [at("2026-10-26T11:15"), at("2026-10-26T11:30")]);
    assert_eq!(free(&short, &[], &[booked(30)]), [at("2026-10-26T11:30")]);
}

#[test]
fn starts_sit_on_the_grid_inside_the_window() {
    let odd = limits(MONDAY, MONDAY, 30, 5, &["mon 10:05-11:00"]);
    assert_eq!(free(&odd, &[], &[]), [at("2026-10-26T10:15"), at("2026-10-26T10:30")]);
}

#[test]
fn nothing_inside_the_notice() {
    let day = limits(MONDAY, MONDAY, 60, 5, &["mon 10:00-12:00"]);
    let now = at("2026-10-26T06:30");
    assert_eq!(slots(&fixture(), &day, now, &[], &[]).first(), Some(&at("2026-10-26T10:30")));
}

#[test]
fn the_policy_hours_apply_unless_the_link_has_its_own() {
    let week = limits(MONDAY, "2026-11-01", 480, 5, &[]);
    assert_eq!(free(&week, &[], &[]).len(), 5);
}

#[test]
fn starts_the_clocks_repeat_or_skip_are_not_offered() {
    let back = limits("2026-11-01", "2026-11-01", 30, 5, &["sun 00:30-03:00"]);
    let starts: Vec<String> = free(&back, &[], &[])
        .iter()
        .map(|t| t.to_zoned(fixture().zone).strftime("%H:%M").to_string())
        .collect();
    assert_eq!(starts, ["00:30", "00:45", "02:00", "02:15", "02:30"]);
    let forward = limits("2027-03-14", "2027-03-14", 30, 5, &["sun 01:00-04:00"]);
    let starts: Vec<String> = slots(&fixture(), &forward, at("2027-03-01T00:00"), &[], &[])
        .iter()
        .map(|t| t.to_zoned(fixture().zone).strftime("%H:%M").to_string())
        .collect();
    assert_eq!(starts, ["01:00", "01:15", "01:30", "01:45", "03:00", "03:15", "03:30"]);
}

#[test]
fn the_public_week_shows_busy_time_inside_hours_and_horizon_only() {
    let now = at("2026-10-26T09:00");
    let week = open(&fixture(), MONDAY.parse().unwrap(), now);
    assert_eq!(week.len(), 5);
    let blocks = shown(
        &week,
        &[
            (busy("2026-10-26T08:00", "2026-10-26T11:00"), 0),
            (busy("2026-10-25T08:00", "2026-10-25T11:00"), 1),
        ],
    );
    assert_eq!(blocks, [(busy("2026-10-26T10:00", "2026-10-26T11:00"), 0)]);
    let far = open(&fixture(), "2026-11-23".parse().unwrap(), at("2026-10-28T12:00"));
    assert_eq!(
        (far.len(), far[2].end),
        (3, at("2026-11-25T11:00")),
        "the horizon ends four weeks on, inside Wednesday's hours"
    );
}

#[test]
fn reads_split_into_weeks() {
    let eight = Period { start: at("2026-10-26T00:00"), end: at("2026-12-21T00:00") };
    let pieces: Vec<Period> = weeks(eight).collect();
    assert_eq!(
        (pieces.len(), pieces[0].start, pieces.last().unwrap().end),
        (9, eight.start, eight.end)
    );
}

#[test]
fn a_booking_ends_inside_its_window_in_real_time() {
    let forward = limits("2027-03-14", "2027-03-14", 90, 5, &["sun 00:00-03:00"]);
    let starts: Vec<String> = slots(&fixture(), &forward, at("2027-03-01T00:00"), &[], &[])
        .iter()
        .map(|t| t.to_zoned(fixture().zone).strftime("%H:%M").to_string())
        .collect();
    assert_eq!(starts, ["00:00", "00:15", "00:30"], "00:45 would end at 03:15 EDT");
}
