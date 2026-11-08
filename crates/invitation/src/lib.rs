//! The JSCalendar that Stalwart's `CalendarEvent/set` takes and, with `sendSchedulingMessages`, writes and sends the
//! iTIP messages from.

use anyhow::{Context, Result};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value, json};

/// Only a tz database zone, whose name JSCalendar's `timeZone` takes.
#[derive(Clone, Debug)]
pub struct Zone {
    tz: TimeZone,
    name: String,
}

impl Zone {
    pub fn named(name: &str) -> Result<Zone> {
        let tz = jiff::tz::db().get(name).with_context(|| format!("time zone {name:?}"))?;
        let name = tz.iana_name().context("a time zone with no name")?.to_string();
        Ok(Zone { tz, name })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn time_zone(&self) -> TimeZone {
        self.tz.clone()
    }
}

impl std::ops::Deref for Zone {
    type Target = TimeZone;
    fn deref(&self) -> &TimeZone {
        &self.tz
    }
}

impl std::str::FromStr for Zone {
    type Err = anyhow::Error;
    fn from_str(name: &str) -> Result<Zone> {
        Zone::named(name)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct At<'a> {
    pub when: Timestamp,
    pub zone: &'a Zone,
}

#[derive(Default)]
pub struct Details<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub start: Option<At<'a>>,
    pub duration: Option<SignedDuration>,
    pub location: Option<&'a str>,
}

impl Details<'_> {
    /// Only the parts given, so a change leaves the others as they are.
    pub fn properties(&self) -> Map<String, Value> {
        let start = self.start.map(|At { when, zone }| {
            let local = when.to_zoned(zone.time_zone()).datetime();
            // Stalwart stores no start at a local time the zone repeats, so either instance goes in UTC.
            let (local, zone) = if zone.to_ambiguous_timestamp(local).is_ambiguous() {
                (when.to_zoned(TimeZone::UTC).datetime(), "Etc/UTC")
            } else {
                (local, zone.name())
            };
            [
                ("start", json!(local.strftime("%Y-%m-%dT%H:%M:%S").to_string())),
                ("timeZone", json!(zone)),
            ]
        });
        let parts = [
            self.title.map(|t| ("title", json!(t))),
            self.description.map(|d| ("description", json!(d))),
            self.duration.map(|d| ("duration", json!(d.to_string()))),
            self.location.map(|place| ("locations", json!({"place": {"name": place}}))),
        ];
        parts
            .into_iter()
            .flatten()
            .chain(start.into_iter().flatten())
            .map(|(name, value)| (name.to_string(), value))
            .collect()
    }
}

/// `address` as a JSCalendar calendar address: a `mailto:` URI, its local part percent-encoded as RFC 6068 says, so
/// a `%`, `#` or `?` in it stays part of the address.
pub fn mailto(address: &str) -> String {
    let (local, domain) = address.rsplit_once('@').unwrap_or((address, ""));
    let kept = |b: u8| b.is_ascii_alphanumeric() || b"-._~!$'()*+,;".contains(&b);
    let local: String = local
        .bytes()
        .map(|b| if kept(b) { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("mailto:{local}@{domain}")
}

pub struct Person<'a> {
    pub address: &'a str,
    pub name: Option<&'a str>,
}

pub struct Invitation<'a> {
    /// What makes a create sent again the same event: Stalwart refuses a second event with it.
    pub uid: &'a str,
    pub title: &'a str,
    pub start: At<'a>,
    pub duration: SignedDuration,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub organizer: &'a str,
    pub organizer_attends: bool,
    pub invited: &'a [Person<'a>],
}

impl Invitation<'_> {
    pub fn event(&self) -> Value {
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
        let mut event = details.properties();
        event.extend([
            ("@type".to_string(), json!("Event")),
            ("uid".to_string(), json!(self.uid)),
            ("organizerCalendarAddress".to_string(), json!(mailto(self.organizer))),
            ("participants".to_string(), Value::Object(participants)),
        ]);
        Value::Object(event)
    }
}
