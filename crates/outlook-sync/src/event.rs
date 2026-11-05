use anyhow::{Context, Result};
use jiff::Timestamp;
use jiff::civil::DateTime;
use ring::digest::{SHA256, digest};
use serde::Deserialize;

use crate::file::FileName;
use crate::ical::{self, escape, fold};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Change {
    Removed {
        id: String,
        #[serde(rename = "@removed")]
        _removed: serde_json::Value,
    },
    Present(Box<Event>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body_preview: Option<String>,
    pub start: When,
    pub end: When,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub is_cancelled: bool,
    #[serde(default)]
    pub show_as: Option<String>,
    #[serde(default)]
    pub sensitivity: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct When {
    pub date_time: String,
    pub time_zone: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    #[serde(default)]
    pub display_name: Option<String>,
}

impl When {
    fn civil(&self) -> Result<DateTime> {
        anyhow::ensure!(
            self.time_zone == "UTC",
            "a time in {} where UTC was asked for",
            self.time_zone
        );
        self.date_time
            .parse()
            .with_context(|| format!("{:?} is not a date and time", self.date_time))
    }

    fn property(&self, name: &str, all_day: bool) -> Result<String> {
        let civil = self.civil()?;
        Ok(match all_day {
            true => format!("{name};VALUE=DATE:{}", civil.date().strftime(ical::DATE)),
            false => {
                format!("{name}:{}", civil.to_zoned(jiff::tz::TimeZone::UTC)?.strftime(ical::UTC))
            }
        })
    }
}

/// A digest of the file apart from its DTSTAMP, the one line a rewrite of an unchanged event changes.
pub fn content(ics: &str) -> String {
    let lines = ics.split("\r\n").filter(|l| !l.starts_with("DTSTAMP:"));
    private::hex::encode(
        digest(&SHA256, lines.collect::<Vec<_>>().join("\r\n").as_bytes()).as_ref(),
    )
}

impl Event {
    pub fn ics(&self, now: Timestamp) -> Result<Option<String>> {
        if self.is_cancelled {
            return Ok(None);
        }
        let mut lines = vec![
            "BEGIN:VCALENDAR".into(),
            "VERSION:2.0".into(),
            "PRODID:-//ironmoon//outlook-sync//EN".into(),
            "BEGIN:VEVENT".into(),
            format!("UID:{}", FileName::of(&self.id).uid()),
            format!("DTSTAMP:{}", now.strftime(ical::UTC)),
            self.start.property("DTSTART", self.is_all_day)?,
            self.end.property("DTEND", self.is_all_day)?,
        ];
        let text = [
            ("SUMMARY", self.subject.as_deref()),
            ("LOCATION", self.location.as_ref().and_then(|l| l.display_name.as_deref())),
            ("DESCRIPTION", self.body_preview.as_deref()),
        ];
        lines.extend(text.into_iter().filter_map(|(k, v)| {
            v.filter(|v| !v.is_empty()).map(|v| format!("{k}:{}", escape(v)))
        }));
        let free = matches!(self.show_as.as_deref(), Some("free" | "workingElsewhere"));
        lines.push(format!("TRANSP:{}", ical::transp(free)));
        if self.show_as.as_deref() == Some("tentative") {
            lines.push("STATUS:TENTATIVE".into());
        }
        if matches!(self.sensitivity.as_deref(), Some("private" | "confidential")) {
            lines.push("CLASS:PRIVATE".into());
        }
        lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
        Ok(Some(lines.iter().map(|l| fold(l)).collect()))
    }
}

#[cfg(test)]
#[path = "../tests/event.rs"]
mod tests;
