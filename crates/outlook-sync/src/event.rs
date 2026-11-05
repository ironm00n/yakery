//! Graph's occurrences as the delta query returns them, and each as the iCalendar file the mirror
//! holds, named by its Graph id so every listing rewrites the same files.

use anyhow::{Context, Result};
use jiff::Timestamp;
use jiff::civil::DateTime;
use ring::digest::{SHA256, digest};
use serde::Deserialize;

/// One entry of a delta page: an occurrence as it now is, or the id of one that is gone.
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

/// Graph's `dateTimeTimeZone`, in UTC since no `Prefer: outlook.timezone` is sent.
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
            true => format!("{name};VALUE=DATE:{}", civil.date().strftime("%Y%m%d")),
            false => format!(
                "{name}:{}",
                civil.to_zoned(jiff::tz::TimeZone::UTC)?.strftime("%Y%m%dT%H%M%SZ")
            ),
        })
    }
}

/// The mirror's file name for an occurrence: Graph ids hold characters a path shouldn't.
pub fn file(id: &str) -> String {
    format!("{}.ics", private::hex::encode(digest(&SHA256, id.as_bytes()).as_ref()))
}

impl Event {
    /// The occurrence as one VEVENT, or None when Outlook shows it cancelled, which the mirror drops.
    pub fn ics(&self, host: &str, now: Timestamp) -> Result<Option<String>> {
        if self.is_cancelled {
            return Ok(None);
        }
        let stem = file(&self.id);
        let mut lines = vec![
            "BEGIN:VCALENDAR".into(),
            "VERSION:2.0".into(),
            "PRODID:-//ironmoon//outlook-sync//EN".into(),
            "BEGIN:VEVENT".into(),
            format!("UID:{}@{host}", stem.trim_end_matches(".ics")),
            format!("DTSTAMP:{}", now.strftime("%Y%m%dT%H%M%SZ")),
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
        lines.push(format!("TRANSP:{}", if free { "TRANSPARENT" } else { "OPAQUE" }));
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

/// RFC 5545 §3.3.11 TEXT.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | ';' | ',' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// RFC 5545 §3.1: lines of at most 75 octets, continued after CRLF and a space, split between
/// characters.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3 + 2);
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
#[path = "../tests/event.rs"]
mod tests;
