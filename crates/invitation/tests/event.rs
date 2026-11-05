use invitation::{Details, Invitation, Person};
use jiff::{SignedDuration, Zoned};
use serde_json::json;

fn at(text: &str) -> Zoned {
    text.parse().unwrap()
}

#[test]
fn an_agents_invitation_names_it_organizer_only_and_him_invited() {
    let start = at("2026-11-06T15:00:00-05:00[America/New_York]");
    let invited = [Person { address: "owen@example.org", name: None }];
    let event = Invitation {
        title: "Review the NEU sync page",
        description: None,
        start: &start,
        duration: SignedDuration::from_mins(30),
        location: None,
        organizer: "agent+scheduler@example.org",
        organizer_attends: false,
        invited: &invited,
    }
    .event()
    .unwrap();
    assert_eq!(
        event,
        json!({
            "@type": "Event",
            "title": "Review the NEU sync page",
            "start": "2026-11-06T15:00:00",
            "timeZone": "America/New_York",
            "duration": "PT30M",
            "organizerCalendarAddress": "mailto:agent+scheduler@example.org",
            "participants": {
                "organizer": {"calendarAddress": "mailto:agent+scheduler@example.org", "roles": {"owner": true}, "participationStatus": "accepted"},
                "invited0": {"calendarAddress": "mailto:owen@example.org", "roles": {"attendee": true}, "expectReply": true},
            },
        })
    );
}

#[test]
fn a_booked_meeting_has_its_organizer_attending_and_the_invitee_named() {
    let start = at("2026-10-27T11:00:00-04:00[America/New_York]");
    let invited = [Person { address: "ada@example.org", name: Some("Ada Lovelace") }];
    let event = Invitation {
        title: "Research chat with Ada Lovelace",
        description: Some("hi"),
        start: &start,
        duration: SignedDuration::from_mins(30),
        location: Some("WVH 366"),
        organizer: "owen@example.org",
        organizer_attends: true,
        invited: &invited,
    }
    .event()
    .unwrap();
    assert_eq!(
        event["participants"]["organizer"]["roles"],
        json!({"owner": true, "attendee": true})
    );
    assert_eq!(event["participants"]["invited0"]["name"], "Ada Lovelace");
    assert_eq!(event["description"], "hi");
    assert_eq!(event["locations"], json!({"place": {"name": "WVH 366"}}));
}

#[test]
fn a_zone_with_no_name_is_refused() {
    let fixed = at("2026-11-06T15:00:00-05:00[-05:00]");
    let invitation = Invitation {
        title: "t",
        description: None,
        start: &fixed,
        duration: SignedDuration::from_mins(30),
        location: None,
        organizer: "agent@example.org",
        organizer_attends: false,
        invited: &[],
    };
    assert!(invitation.event().is_err());
}

#[test]
fn a_change_sets_only_the_parts_given() {
    let start = at("2026-11-06T16:30:00-05:00[America/New_York]");
    let change = Details { start: Some(&start), location: Some("WVH 366"), ..Details::default() };
    assert_eq!(
        serde_json::Value::Object(change.properties().unwrap()),
        json!({"start": "2026-11-06T16:30:00", "timeZone": "America/New_York", "locations": {"place": {"name": "WVH 366"}}})
    );
    assert!(Details::default().properties().unwrap().is_empty());
}
