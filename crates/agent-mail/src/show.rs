//! Mail is untrusted text, so only header lines start at column 0 (`=== `), their fields cannot hold line breaks,
//! and every body line is indented.

use std::io::Write;

use chrono::{DateTime, SecondsFormat};

use crate::{
    mail::{Addr, Body, Email, Keyword, MAX_BODY_BYTES, Part},
    prelude::*,
};

const MAX_PARTS: usize = 4;
const MAX_SHOWN_BYTES: usize = 256 * 1024;

pub fn utc(secs: i64) -> String {
    DateTime::from_timestamp(secs, 0)
        .map_or_else(|| secs.to_string(), |t| t.to_rfc3339_opts(SecondsFormat::Secs, true))
}

pub fn plain(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() || c == '\u{2028}' || c == '\u{2029}' { ' ' } else { c })
        .collect()
}

pub fn emails(list: &[Addr]) -> String {
    plain(&list.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", "))
}

pub fn preview(m: &Email, chars: usize) -> String {
    plain(&m.preview).split_whitespace().collect::<Vec<_>>().join(" ").chars().take(chars).collect()
}

fn head(m: &Email) -> String {
    format!("{}  {}  {} -> {}", plain(&m.id), plain(&m.received), emails(&m.from), emails(&m.to))
}

pub fn inbox_line(m: &Email, files: &[Part]) -> String {
    let unread = if m.has(Keyword::Seen) { ' ' } else { '*' };
    let cc = if m.cc.is_empty() { String::new() } else { format!(" cc {}", emails(&m.cc)) };
    let files: String =
        files.iter().map(|a| format!(" [{} {}B]", plain(&a.name), a.size)).collect();
    format!("{unread} {}{cc}  {}{files}\n    {}", head(m), plain(&m.subject), preview(m, 160))
}

pub fn message(m: &Email, body: Option<&Body>) -> String {
    let files = body.iter().flat_map(|b| &b.attachments);
    let files: String = files.map(|a| format!(" [{}]", plain(&a.name))).collect();
    let values: Vec<_> =
        body.iter().flat_map(|b| b.text.iter().filter_map(|p| b.values.get(&p.part))).collect();
    let shown = &values[..values.len().min(MAX_PARTS)];
    let text = shown.iter().map(|v| v.value.as_str()).collect::<Vec<_>>().join("\n");
    let text = text.trim_end();
    let cut = text.floor_char_boundary(MAX_SHOWN_BYTES);
    let mut lines: Vec<String> = text[..cut].lines().map(|l| format!("  {}", plain(l))).collect();
    if shown.iter().any(|v| v.truncated) {
        lines.push(format!("  [a part was cut at {} KiB]", MAX_BODY_BYTES / 1024));
    }
    if cut < text.len() {
        lines.push(format!("  [{} more bytes left out]", text.len() - cut));
    }
    if values.len() > shown.len() {
        lines.push(format!("  [{} more text parts left out]", values.len() - shown.len()));
    }
    format!("=== {}  {}{files}\n{}", head(m), plain(&m.subject), lines.join("\n"))
}

pub fn reminder(m: &Email) -> String {
    let json = serde_json::to_string(&m.subject).unwrap_or_default();
    let escape = |c: char| match c {
        '\u{2028}' | '\u{2029}' | '\u{85}' | '\u{7f}' => format!("\\u{:04x}", u32::from(c)),
        c => c.to_string(),
    };
    format!(
        "seen but unanswered: {} {}",
        plain(&m.id),
        json.chars().map(escape).collect::<String>()
    )
}

pub fn out(text: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{text}").and_then(|()| stdout.flush()).context("writing to stdout")
}
