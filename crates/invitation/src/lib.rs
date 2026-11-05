//! An event that invites people, as the JSCalendar that Stalwart's `CalendarEvent/set` takes and, with
//! `sendSchedulingMessages`, writes and sends the iTIP messages from: invitations, updates, cancels, and the answers
//! it applies back to the organizer's copy.

use anyhow::{Context, Result};
use jiff::{SignedDuration, Zoned};
use serde_json::{Map, Value, json};

/// What an event says, each part optional, as a change to an event sets it.
#[derive(Default)]
pub struct Details<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    /// A local time in a named zone, so a series or a later move keeps its local time across a change of offset.
    pub start: Option<&'a Zoned>,
    pub duration: Option<SignedDuration>,
    pub location: Option<&'a str>,
}

impl Details<'_> {
    /// The JSCalendar properties of the parts given, and only those.
    pub fn properties(&self) -> Result<Map<String, Value>> {
        let start = self.start.map(|at| -> Result<_> {
            let zone = at.time_zone().iana_name().context("a time zone with no name")?;
            Ok([
                ("start", json!(at.datetime().strftime("%Y-%m-%dT%H:%M:%S").to_string())),
                ("timeZone", json!(zone)),
            ])
        });
        let parts = [
            self.title.map(|t| ("title", json!(t))),
            self.description.map(|d| ("description", json!(d))),
            self.duration.map(|d| ("duration", json!(d.to_string()))),
            self.location.map(|place| ("locations", json!({"place": {"name": place}}))),
        ];
        Ok(parts
            .into_iter()
            .flatten()
            .chain(start.transpose()?.into_iter().flatten())
            .map(|(name, value)| (name.to_string(), value))
            .collect())
    }
}

pub struct Person<'a> {
    pub address: &'a str,
    pub name: Option<&'a str>,
}

pub struct Invitation<'a> {
    pub title: &'a str,
    pub start: &'a Zoned,
    pub duration: SignedDuration,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub organizer: &'a str,
    /// Whether the organizer attends too, as the owner of a booked meeting does; an agent only invites.
    pub organizer_attends: bool,
    pub invited: &'a [Person<'a>],
}

impl Invitation<'_> {
    pub fn event(&self) -> Result<Value> {
        let mailto = |address: &str| format!("mailto:{address}");
        // JSCalendar's roles are a set, each present role true
        let roles: Map<String, Value> = [("owner", true), ("attendee", self.organizer_attends)]
            .into_iter()
            .filter(|(_, held)| *held)
            .map(|(role, _)| (role.to_string(), json!(true)))
            .collect();
        let organizer = json!({
            "calendarAddress": mailto(self.organizer),
            "roles": roles,
            "participationStatus": "accepted",
        });
        let invited = self.invited.iter().enumerate().map(|(n, p)| {
            let mut person = json!({"calendarAddress": mailto(p.address), "roles": {"attendee": true}, "expectReply": true});
            if let Some(name) = p.name {
                person["name"] = json!(name);
            }
            (format!("invited{n}"), person)
        });
        let participants: Map<String, Value> =
            [("organizer".to_string(), organizer)].into_iter().chain(invited).collect();
        let details = Details {
            title: Some(self.title),
            description: self.description,
            start: Some(self.start),
            duration: Some(self.duration),
            location: self.location,
        };
        let mut event = details.properties()?;
        event.extend([
            ("@type".to_string(), json!("Event")),
            ("organizerCalendarAddress".to_string(), json!(mailto(self.organizer))),
            ("participants".to_string(), Value::Object(participants)),
        ]);
        Ok(Value::Object(event))
    }
}
