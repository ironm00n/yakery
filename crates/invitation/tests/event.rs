use std::sync::LazyLock;

use invitation::{At, Details, Invitation, Length, Person, Zone, mailto};
use serde_json::json;

static NEW_YORK: LazyLock<Zone> = LazyLock::new(|| Zone::named("America/New_York").unwrap());

fn at(utc: &str) -> At<'static> {
    At { when: utc.parse().unwrap(), zone: &NEW_YORK }
}

#[test]
fn an_agents_invitation_names_it_organizer_only_and_him_invited() {
    let start = at("2026-11-06T20:00:00Z");
    let invited = [Person { address: "owner@example.org", name: None }];
    let event = Invitation {
        uid: "u1@example.org",
        title: "Review the sync page",
        description: None,
        start,
        duration: "30m".parse().unwrap(),
        location: None,
        organizer: "agent+scheduler@example.org",
        organizer_attends: false,
        invited: &invited,
    }
    .event();
    assert_eq!(
        event,
        json!({
            "@type": "Event",
            "uid": "u1@example.org",
            "title": "Review the sync page",
            "start": "2026-11-06T15:00:00",
            "timeZone": "America/New_York",
            "duration": "PT30M",
            "organizerCalendarAddress": "mailto:agent+scheduler@example.org",
            "participants": {
                "organizer": {"calendarAddress": "mailto:agent+scheduler@example.org", "roles": {"owner": true}},
                "invited0": {"calendarAddress": "mailto:owner@example.org", "roles": {"attendee": true}, "expectReply": true},
            },
        })
    );
}

#[test]
fn a_booked_meeting_has_its_organizer_attending_and_the_invitee_named() {
    let start = at("2026-10-27T15:00:00Z");
    let invited = [Person { address: "ada@example.org", name: Some("Ada Lovelace") }];
    let event = Invitation {
        uid: "u2@example.org",
        title: "Research chat with Ada Lovelace",
        description: Some("hi"),
        start,
        duration: "30m".parse().unwrap(),
        location: Some("Room 101"),
        organizer: "owner@example.org",
        organizer_attends: true,
        invited: &invited,
    }
    .event();
    assert_eq!(
        event["participants"]["organizer"],
        json!({"calendarAddress": "mailto:owner@example.org", "roles": {"owner": true, "attendee": true}, "participationStatus": "accepted"})
    );
    assert_eq!(event["participants"]["invited0"]["name"], "Ada Lovelace");
    assert_eq!(event["description"], "hi");
    assert_eq!(event["locations"], json!({"place": {"name": "Room 101"}}));
}

#[test]
fn only_a_named_zone_of_the_database_is_a_zone() {
    assert_eq!(NEW_YORK.name(), "America/New_York");
    assert!(Zone::named("-05:00").is_err());
    assert!("Nowhere/Else".parse::<Zone>().is_err());
}

#[test]
fn a_length_is_above_zero_in_whole_seconds() {
    assert_eq!("90m".parse::<Length>().unwrap().to_string(), "PT1H30M");
    for refused in ["0s", "-30m", "30m 500ms", "PT0.5S"] {
        let e = refused.parse::<Length>().unwrap_err().to_string();
        assert_eq!(e, format!("{refused} isn't a length above zero in whole seconds"));
    }
}

#[test]
fn a_change_sets_only_the_parts_given() {
    let start = at("2026-11-06T21:30:00Z");
    let change = Details { start: Some(start), location: Some("Room 101"), ..Details::default() };
    assert_eq!(
        serde_json::Value::Object(change.properties()),
        json!({"start": "2026-11-06T16:30:00", "timeZone": "America/New_York", "locations": {"place": {"name": "Room 101"}}})
    );
    assert!(Details::default().properties().is_empty());
}

#[test]
fn a_start_in_an_hour_the_zone_repeats_is_written_in_utc_whichever_instance_it_is() {
    let start = |utc: &str| {
        serde_json::Value::Object(
            Details { start: Some(at(utc)), ..Details::default() }.properties(),
        )
    };
    assert_eq!(
        start("2026-11-01T05:30:00Z"),
        json!({"start": "2026-11-01T05:30:00", "timeZone": "Etc/UTC"})
    );
    assert_eq!(
        start("2026-11-01T06:30:00Z"),
        json!({"start": "2026-11-01T06:30:00", "timeZone": "Etc/UTC"})
    );
    assert_eq!(
        start("2026-11-01T04:30:00Z"),
        json!({"start": "2026-11-01T00:30:00", "timeZone": "America/New_York"})
    );
}

#[test]
fn a_calendar_address_keeps_its_local_part_as_one_part() {
    assert_eq!(mailto("owner@example.org"), "mailto:owner@example.org");
    assert_eq!(mailto("agent+pool@example.org"), "mailto:agent+pool@example.org");
    assert_eq!(mailto("x%40evil.com#@example.org"), "mailto:x%2540evil.com%23@example.org");
    assert_eq!(mailto("a=b?c@example.org"), "mailto:a%3Db%3Fc@example.org");
}
