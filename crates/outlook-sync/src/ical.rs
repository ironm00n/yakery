//! RFC 5545 content lines both ways: what the mirror's files are written as, and read back by.

use anyhow::{Result, bail};

pub const DATE: &str = "%Y%m%d";
pub const DATE_TIME: &str = "%Y%m%dT%H%M%S";
pub const UTC: &str = "%Y%m%dT%H%M%SZ";

pub fn transp(free: bool) -> &'static str {
    if free { "TRANSPARENT" } else { "OPAQUE" }
}

pub fn free(transp: Option<&str>) -> bool {
    transp == Some("TRANSPARENT")
}

/// RFC 5545 §3.3.11 TEXT.
pub fn escape(text: &str) -> String {
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

/// RFC 5545 §3.1, split between characters.
pub fn fold(line: &str) -> String {
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

pub struct Line {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
}

pub fn lines(ics: &str) -> Vec<Line> {
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

pub fn vevent(ics: &str) -> Result<Vec<Line>> {
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

pub fn unescape(text: &str) -> String {
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

#[cfg(test)]
#[path = "../tests/ical.rs"]
mod tests;
