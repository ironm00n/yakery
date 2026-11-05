//! Reads only what the mirror writes and what a calendar app sets when making an event: times,
//! all-day, title, place, description and a recurrence Graph can express.

use anyhow::{Context, Result, bail};
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

fn lines(ics: &str) -> Vec<Line> {
    let unfolded = ics.replace("\r\n ", "").replace("\r\n\t", "").replace("\n ", "");
    unfolded
        .lines()
        .filter_map(|l| {
            let (head, value) = l.split_once(':')?;
            let mut parts = head.split(';');
            let name = parts.next()?.to_ascii_uppercase();
            let params = parts
                .filter_map(|p| {
                    p.split_once('=')
                        .map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
                })
                .collect();
            Some(Line { name, params, value: value.to_string() })
        })
        .collect()
}

fn vevent(ics: &str) -> Result<Vec<Line>> {
    let mut inside = false;
    let mut out = vec![];
    for line in lines(ics) {
        match (line.name.as_str(), line.value.as_str()) {
            ("BEGIN", "VEVENT") => inside = true,
            ("END", "VEVENT") if inside => return Ok(out),
            _ if inside => out.push(line),
            _ => {}
        }
    }
    bail!("no VEVENT")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            },
            (c, false) => out.push(c),
        }
    }
    out
}

/// Zoned keeps the zone the calendar app named, which Graph needs for a recurrence to keep its
/// local time across a change of offset.
#[derive(Debug, PartialEq, Eq)]
pub enum At {
    Date(Date),
    Zoned(jiff::Zoned),
}

fn parse_at(value: &str, date: bool, zone: Option<TimeZone>) -> Result<At> {
    if date {
        return Ok(At::Date(Date::strptime("%Y%m%d", value)?));
    }
    let (civil, zone) = match (value.strip_suffix('Z'), zone) {
        (Some(utc), _) => (utc, TimeZone::UTC),
        (None, Some(zone)) => (value, zone),
        (None, None) => bail!("a floating time ({value}), which names no instant"),
    };
    Ok(At::Zoned(DateTime::strptime("%Y%m%dT%H%M%S", civil)?.to_zoned(zone)?))
}

fn at(line: &Line) -> Result<At> {
    let param = |k: &str| line.params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let zone = param("TZID")
        .map(|name| jiff::tz::db().get(name).with_context(|| format!("time zone {name:?}")))
        .transpose()?;
    parse_at(&line.value, param("VALUE") == Some("DATE"), zone)
}

fn graph_time(at: &At) -> Result<Value> {
    let (civil, zone) = match at {
        At::Date(d) => (d.to_datetime(jiff::civil::Time::midnight()), "UTC"),
        At::Zoned(z) => (z.datetime(), z.time_zone().iana_name().context("an unnamed time zone")?),
    };
    Ok(json!({"dateTime": civil.strftime("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": zone}))
}

pub fn graph(ics: &str) -> Result<Value> {
    let lines = vevent(ics)?;
    let get = |name: &str| lines.iter().find(|l| l.name == name);
    let start = at(get("DTSTART").context("no DTSTART")?)?;
    let end = match (get("DTEND"), get("DURATION")) {
        (Some(line), _) => at(line)?,
        (None, Some(line)) => {
            let span: jiff::Span =
                line.value.parse().with_context(|| format!("DURATION {:?}", line.value))?;
            match &start {
                At::Date(d) => At::Date(d.checked_add(span)?),
                At::Zoned(z) => At::Zoned(z.checked_add(span)?),
            }
        }
        (None, None) => bail!("neither DTEND nor DURATION"),
    };
    let mut event = json!({
        "subject": get("SUMMARY").map(|l| unescape(&l.value)).unwrap_or_default(),
        "start": graph_time(&start)?,
        "end": graph_time(&end)?,
        "isAllDay": matches!(start, At::Date(_)),
        "location": {"displayName": get("LOCATION").map(|l| unescape(&l.value)).unwrap_or_default()},
        "showAs": if get("TRANSP").is_some_and(|l| l.value == "TRANSPARENT") { "free" } else { "busy" },
    });
    if let Some(note) = get("DESCRIPTION") {
        event["body"] = json!({"contentType": "text", "content": unescape(&note.value)});
    }
    if let Some(rule) = get("RRULE") {
        event["recurrence"] = recurrence(&rule.value, &start)?;
    }
    Ok(event)
}

const DAYS: [(&str, &str); 7] = [
    ("MO", "monday"),
    ("TU", "tuesday"),
    ("WE", "wednesday"),
    ("TH", "thursday"),
    ("FR", "friday"),
    ("SA", "saturday"),
    ("SU", "sunday"),
];

fn day(code: &str) -> Result<&'static str> {
    DAYS.iter()
        .find(|(c, _)| *c == code)
        .map(|(_, d)| *d)
        .with_context(|| format!("weekday {code:?}"))
}

/// Graph reads the range's dates in the start's zone, so they are taken there.
fn recurrence(rule: &str, start: &At) -> Result<Value> {
    let parts: Vec<(&str, &str)> = rule.split(';').filter_map(|p| p.split_once('=')).collect();
    let part = |k: &str| parts.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
    let known = ["FREQ", "INTERVAL", "BYDAY", "BYMONTHDAY", "COUNT", "UNTIL", "WKST"];
    if let Some((k, _)) = parts.iter().find(|(k, _)| !known.contains(k)) {
        bail!("RRULE part {k} has no Graph pattern");
    }
    let (first, zone) = match start {
        At::Date(d) => (*d, None),
        At::Zoned(z) => (z.date(), Some(z.time_zone().clone())),
    };
    let interval: i64 = part("INTERVAL").map_or(Ok(1), str::parse)?;
    let weekly = |on: Option<&str>| -> Result<Value> {
        let on: Vec<&str> = match on {
            Some(list) => list.split(',').map(day).collect::<Result<_>>()?,
            None => vec![DAYS[first.weekday().to_monday_zero_offset() as usize].1],
        };
        Ok(
            json!({"type": "weekly", "interval": interval, "daysOfWeek": on, "firstDayOfWeek": day(part("WKST").unwrap_or("MO"))?}),
        )
    };
    let freq = part("FREQ").context("RRULE without FREQ")?;
    let pattern = match (freq, part("BYDAY"), part("BYMONTHDAY")) {
        ("DAILY", None, None) => json!({"type": "daily", "interval": interval}),
        ("DAILY", Some(on), None) if interval == 1 => weekly(Some(on))?,
        ("WEEKLY", on, None) => weekly(on)?,
        ("MONTHLY", Some(byday), None) => {
            let (ordinal, code) =
                byday.split_at_checked(byday.len().saturating_sub(2)).context("BYDAY")?;
            let index = match ordinal {
                "1" | "+1" => "first",
                "2" | "+2" => "second",
                "3" | "+3" => "third",
                "4" | "+4" => "fourth",
                "-1" => "last",
                other => bail!("monthly BYDAY ordinal {other:?} has no Graph pattern"),
            };
            json!({"type": "relativeMonthly", "interval": interval, "daysOfWeek": [day(code)?], "index": index})
        }
        ("MONTHLY", None, on) => {
            let on: i8 = on.map_or(Ok(first.day()), str::parse)?;
            json!({"type": "absoluteMonthly", "interval": interval, "dayOfMonth": on})
        }
        ("YEARLY", None, None) => {
            json!({"type": "absoluteYearly", "interval": interval, "month": first.month(), "dayOfMonth": first.day()})
        }
        _ => bail!("{rule} has no Graph pattern"),
    };
    let range = match (part("COUNT"), part("UNTIL")) {
        (Some(n), _) => {
            json!({"type": "numbered", "startDate": first.to_string(), "numberOfOccurrences": n.parse::<i64>()?})
        }
        (None, Some(until)) => {
            let last = match parse_at(until, until.len() == 8, zone.clone())? {
                At::Date(d) => d,
                At::Zoned(z) => z.with_time_zone(zone.unwrap_or(TimeZone::UTC)).date(),
            };
            json!({"type": "endDate", "startDate": first.to_string(), "endDate": last.to_string()})
        }
        (None, None) => json!({"type": "noEnd", "startDate": first.to_string()}),
    };
    Ok(json!({"pattern": pattern, "range": range}))
}

#[cfg(test)]
#[path = "../tests/local.rs"]
mod tests;
