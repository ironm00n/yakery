use clap::Parser;

use super::{HIM, Scratch, wire::Stalwart, wire::mail};
use crate::{
    Cli, Command,
    cmd::invite::{self, invitations, invite},
    prelude::*,
};

fn command(args: &[&str]) -> Result<Command, String> {
    let words = ["agent-mail"].iter().chain(args);
    Cli::try_parse_from(words).map(|c| c.command).map_err(|e| e.render().to_string())
}

fn asked(args: &[&str]) -> invite::Args {
    match command(&[&["invite"], args].concat()) {
        Ok(Command::Invite(args)) => args,
        other => panic!("not an invite: {:?}", other.err()),
    }
}

/// Stalwart whose session names a calendars account, `cal`, apart from the mail account, `a`.
fn with_calendars(calendars: Vec<Value>) -> Stalwart {
    let mut stalwart = Stalwart::new(vec![]);
    stalwart.session["primaryAccounts"]["urn:ietf:params:jmap:calendars"] = json!("cal");
    stalwart.calendars = calendars;
    stalwart
}

/// The one request to the JMAP API that calls `method`: what it declares, and the method's arguments.
fn request(stalwart: &Stalwart, method: &str) -> (Value, Value) {
    let found: Vec<&Value> =
        stalwart.requests.iter().filter(|r| r["methodCalls"][0][0] == method).collect();
    assert_eq!(found.len(), 1, "{method} asked {} times", found.len());
    (found[0]["using"].clone(), found[0]["methodCalls"][0][1].clone())
}

const NEW: &[&str] = &[
    "new",
    "--topic",
    "pool",
    "--to",
    HIM,
    "--title",
    "Review the sync page",
    "--start",
    "2026-11-06T20:00:00Z",
    "--duration",
    "30m",
];

#[tokio::test]
async fn a_new_invitation_is_an_event_in_the_default_calendar_that_stalwart_sends_from() {
    let dir = Scratch::new("invite-new");
    let calendars =
        vec![json!({ "id": "c8", "isDefault": false }), json!({ "id": "c9", "isDefault": true })];
    let (_wire, server, mail) = mail(&dir, with_calendars(calendars)).await;
    assert_eq!(invite(&mail.unwrap(), asked(NEW)).await.unwrap(), "e1");
    let (using, args) = request(&server.borrow(), "CalendarEvent/set");
    assert_eq!(using, json!(["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:calendars"]));
    assert_eq!(
        (&args["accountId"], &args["sendSchedulingMessages"]),
        (&json!("cal"), &json!(true))
    );
    let event = &args["create"]["i"];
    assert_eq!(event["calendarIds"], json!({ "c9": true }));
    assert_eq!(
        (&event["start"], &event["timeZone"]),
        (&json!("2026-11-06T15:00:00"), &json!("America/New_York"))
    );
    assert_eq!(event["duration"], "PT30M");
    assert_eq!(event["organizerCalendarAddress"], "mailto:agent+pool@example.org");
    let uid = event["uid"].as_str().unwrap();
    assert!(uid.len() == 32 + "@example.org".len() && uid.ends_with("@example.org"), "{uid}");
    assert_eq!(event["participants"]["organizer"]["roles"], json!({ "owner": true }));
    assert_eq!(event["participants"]["invited0"]["calendarAddress"], format!("mailto:{HIM}"));
}

#[tokio::test]
async fn a_login_with_no_default_calendar_makes_one_first() {
    let dir = Scratch::new("invite-calendar");
    let (_wire, server, mail) = mail(&dir, with_calendars(vec![])).await;
    invite(&mail.unwrap(), asked(NEW)).await.unwrap();
    let server = server.borrow();
    let (_, made) = request(&server, "Calendar/set");
    assert_eq!(made["onSuccessSetIsDefault"], "#c");
    assert_eq!(
        request(&server, "CalendarEvent/set").1["create"]["i"]["calendarIds"],
        json!({ "c1": true })
    );
}

/// An invitation from `organizer` already in the calendar, as `e1`, in `zone`.
fn held(organizer: &str, zone: &str) -> Value {
    json!({
        "id": "e1", "uid": "u1", "title": "t", "start": "2026-11-06T15:00:00", "timeZone": zone,
        "organizerCalendarAddress": format!("mailto:{organizer}"), "participants": {},
    })
}

#[tokio::test]
async fn an_update_sends_only_what_changes_and_a_cancel_destroys_the_event() {
    let dir = Scratch::new("invite-change");
    let mut stalwart = with_calendars(vec![]);
    stalwart.events = vec![held("agent+pool@example.org", "America/New_York")];
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    let moved = asked(&[
        "update",
        "e1",
        "--topic",
        "pool",
        "--start",
        "2026-11-06T21:30:00Z",
        "--location",
        "Room 101",
    ]);
    assert_eq!(invite(&mail, moved).await.unwrap(), "e1");
    let (_, args) = request(&server.borrow(), "CalendarEvent/set");
    assert_eq!(args["sendSchedulingMessages"], true);
    assert_eq!(
        args["update"],
        json!({ "e1": { "start": "2026-11-06T16:30:00", "timeZone": "America/New_York", "locations": { "place": { "name": "Room 101" } } } })
    );
    server.borrow_mut().requests.clear();
    invite(&mail, asked(&["cancel", "e1", "--topic", "pool"])).await.unwrap();
    let (_, args) = request(&server.borrow(), "CalendarEvent/set");
    assert_eq!((&args["destroy"], &args["sendSchedulingMessages"]), (&json!(["e1"]), &json!(true)));
    assert!(server.borrow().events.is_empty());
}

#[tokio::test]
async fn an_update_keeps_the_events_zone_and_another_topics_invitation_is_refused() {
    let dir = Scratch::new("invite-zone");
    let mut stalwart = with_calendars(vec![]);
    stalwart.events = vec![held("agent+pool@example.org", "Europe/Paris")];
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    invite(&mail, asked(&["update", "e1", "--topic", "pool", "--start", "2026-11-06T20:00:00Z"]))
        .await
        .unwrap();
    let (_, args) = request(&server.borrow(), "CalendarEvent/set");
    assert_eq!(
        args["update"]["e1"],
        json!({ "start": "2026-11-06T21:00:00", "timeZone": "Europe/Paris" })
    );
    server.borrow_mut().requests.clear();
    for change in [
        &["update", "e1", "--topic", "other", "--title", "x"][..],
        &["cancel", "e1", "--topic", "other"],
    ] {
        let refused = invite(&mail, asked(change)).await.map_err(|e| e.to_string());
        assert_eq!(
            refused.unwrap_err(),
            "event e1 isn't an invitation from mailto:agent+other@example.org"
        );
    }
    assert!(server.borrow().requests.iter().all(|r| r["methodCalls"][0][0] != "CalendarEvent/set"));
}

#[tokio::test]
async fn an_invitation_made_again_is_the_one_already_made() {
    let dir = Scratch::new("invite-again");
    let (_wire, server, mail) = mail(&dir, with_calendars(vec![])).await;
    let mail = mail.unwrap();
    assert_eq!(invite(&mail, asked(NEW)).await.unwrap(), "e1");
    assert_eq!(invite(&mail, asked(NEW)).await.unwrap(), "e1");
    let him = HIM.to_uppercase();
    let shouted: Vec<&str> = NEW.iter().map(|w| if *w == HIM { him.as_str() } else { w }).collect();
    assert_eq!(
        invite(&mail, asked(&shouted)).await.unwrap(),
        "e1",
        "an address the same but for case"
    );
    assert_eq!(server.borrow().events.len(), 1);
    let other = [NEW, &["--location", "elsewhere"]].concat();
    assert_eq!(
        invite(&mail, asked(&other)).await.unwrap(),
        "e1",
        "the place doesn't make it another invitation"
    );
    let renamed: Vec<&str> = NEW
        .iter()
        .map(|w| if *w == "Review the sync page" { "Another review" } else { w })
        .collect();
    assert_eq!(invite(&mail, asked(&renamed)).await.unwrap(), "e2");
}

#[tokio::test]
async fn invites_lists_the_logins_invitations_with_each_answer() {
    let dir = Scratch::new("invites");
    let mut stalwart = with_calendars(vec![]);
    let event = |id: &str, organizer: &str, answer: Value| {
        json!({
            "id": id, "title": format!("about {id}"), "start": "2026-11-06T15:00:00", "timeZone": "America/New_York",
            "organizerCalendarAddress": format!("mailto:{organizer}"),
            "participants": {
                "organizer": { "calendarAddress": format!("mailto:{organizer}"), "roles": { "owner": true } },
                "invited0": { "calendarAddress": format!("mailto:{HIM}"), "roles": { "attendee": true }, "participationStatus": answer },
            },
        })
    };
    stalwart.events = vec![
        event("e1", "agent+pool@example.org", json!("accepted")),
        event("e2", "agent+other@example.org", Value::Null),
        event("e3", "someone@example.com", json!("declined")),
        // an all-day invitation received, as Stalwart serves it: no title or time zone set, so none given
        json!({ "id": "e4", "start": "2026-11-07T00:00:00", "organizerCalendarAddress": "mailto:someone@example.com" }),
    ];
    let (_wire, _server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    let list = |args: &[&str]| match command(&[&["invites"], args].concat()) {
        Ok(Command::Invites(list)) => list,
        _ => panic!("not invites"),
    };
    let line = |id: &str, answer: &str| {
        format!("{id}  2026-11-06T15:00:00 America/New_York  about {id}  {HIM} {answer}")
    };
    assert_eq!(
        invitations(&mail, &list(&["--topic", "pool"])).await.unwrap(),
        [line("e1", "accepted")]
    );
    assert_eq!(
        invitations(&mail, &list(&[])).await.unwrap(),
        [line("e1", "accepted"), line("e2", "needs-action")]
    );
}

#[tokio::test]
async fn an_invitation_listing_that_stalwart_cut_at_its_get_limit_is_refused() {
    let dir = Scratch::new("invite-cut");
    let mut stalwart = with_calendars(vec![]);
    stalwart.session["capabilities"]["urn:ietf:params:jmap:core"]["maxObjectsInGet"] = json!(4);
    let event = |n: u32| json!({ "id": format!("e{n}"), "organizerCalendarAddress": "mailto:agent+pool@example.org" });
    stalwart.events = (1..=5).map(event).collect();
    let (_wire, _server, mail) = mail(&dir, stalwart).await;
    let Ok(Command::Invites(list)) = command(&["invites"]) else { panic!("not invites") };
    let refused = invitations(&mail.unwrap(), &list).await.map_err(|e| format!("{e:#}"));
    assert_eq!(
        refused.unwrap_err(),
        "stalwart answered CalendarEvent/get with 4, its whole get limit, so it may have left some out"
    );
}

#[test]
fn an_invitation_lasts_a_length_above_zero() {
    let with = |duration: &str| {
        let given = format!("--duration={duration}");
        let args: Vec<&str> =
            NEW[..NEW.len() - 2].iter().copied().chain([given.as_str()]).collect();
        command(&[&["invite"], &args[..]].concat()).map(drop)
    };
    assert!(with("30m").is_ok());
    for refused in ["-30m", "0s", "PT0S"] {
        assert!(
            with(refused).unwrap_err().contains(&format!("{refused} isn't a length above zero")),
            "{refused}"
        );
    }
}

#[tokio::test]
async fn a_session_with_no_calendars_account_refuses_an_invitation_and_mail_asks_as_before() {
    let dir = Scratch::new("invite-no-calendars");
    let (_wire, server, mail) = mail(&dir, Stalwart::new(vec![])).await;
    let refused = invite(&mail.unwrap(), asked(NEW)).await.map_err(|e| format!("{e:#}"));
    assert_eq!(refused.unwrap_err(), "the JMAP session names no calendars account");
    let (using, args) = request(&server.borrow(), "Mailbox/get");
    assert_eq!(
        using,
        json!([
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:mail",
            "urn:ietf:params:jmap:submission"
        ])
    );
    assert_eq!(args["accountId"], "a");
}

#[test]
fn the_cli_refuses_what_a_change_cannot_mean() {
    let refused = |args: &[&str]| command(&[&["invite"], args].concat()).is_err();
    assert!(refused(&["cancel", "e1", "--title", "t"]));
    assert!(refused(&["update", "e1"]));
    assert!(refused(&["update", "e1", "--title", "t", "--zone", "Europe/Paris"]));
    assert!(refused(&["update", "e1", "--zone", "Europe/Paris"]));
    assert!(refused(&["update", "e1", "--start", "2026-11-06T15:00:00", "--zone", "Europe/Paris"]));
    assert!(refused(&[
        "new",
        "--title",
        "t",
        "--start",
        "2026-11-06T20:00:00Z",
        "--duration",
        "30m"
    ]));
    assert!(refused(&[
        "new",
        "--to",
        HIM,
        "--title",
        "t",
        "--start",
        "2026-11-06T20:00:00Z",
        "--duration",
        "30m",
        "--zone",
        "Nowhere/Else"
    ]));
    assert!(!refused(&[
        "update",
        "e1",
        "--start",
        "2026-11-06T20:00:00Z",
        "--zone",
        "Europe/Paris"
    ]));
}
