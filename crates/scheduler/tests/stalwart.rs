use serde_json::json;

use super::*;
use crate::checked::Checked;
use crate::link::{Form, Issue, Token};
use crate::policy::tests::fixture;

#[test]
fn creates_read_as_stalwart_0_16_26_answers_them() {
    // recorded by scheduler/proto/vm/stalwart.nix
    let created = json!({"accountId": "b", "created": {"b": {"id": "u"}}, "newState": "sba", "oldState": "saq"});
    let again = json!({"accountId": "b", "newState": "sba", "notCreated": {"b": {
        "description": "An event with UID 00…@schedule.example.test already exists.", "properties": ["uid"], "type": "invalidProperties"}}});
    let elsewhere = json!({"accountId": "b", "notCreated": {"b": {
        "description": "You are not allowed to add calendar events to calendar c.", "type": "forbidden"}}});
    assert_eq!(super::created(&created), Created::Event("u".into()));
    assert_eq!(super::created(&again), Created::UidTaken);
    assert!(matches!(super::created(&elsewhere), Created::Refused(_)));
}

#[test]
fn the_event_names_the_owner_as_organizer_and_the_booker_as_attendee() {
    let token = Token::parse(&"a".repeat(64)).unwrap();
    let issue = Issue {
        title: Some("Research chat".into()),
        location: Some("WVH 366".into()),
        ..crate::link::tests::issue("2026-10-26", "2026-10-30")
    };
    let booker = Checked::new(Form {
        name: "Ada Lovelace".into(),
        address: "ada@example.org".into(),
        note: "hi".into(),
    })
    .unwrap();
    let booking = Booking { start: "2026-10-27T15:00:00Z".parse().unwrap(), booker };
    let event = event(&fixture(), token.digest(), &Checked::new(issue).unwrap(), &booking);
    assert_eq!(event["start"], "2026-10-27T11:00:00");
    assert_eq!(event["timeZone"], "America/New_York");
    assert_eq!(event["duration"], "PT30M");
    assert_eq!(event["title"], "Research chat with Ada Lovelace");
    assert_eq!(event["organizerCalendarAddress"], "mailto:owen@example.test");
    assert_eq!(event["participants"]["booker"]["calendarAddress"], "mailto:ada@example.org");
    assert_eq!(event["description"], format!("hi\n\nBooked with link {}", token.digest().id()));
    assert_eq!(event["locations"]["place"]["name"], "WVH 366");
    assert!(event.get("calendarIds").is_none(), "only Stalwart::create names the calendar");
}
