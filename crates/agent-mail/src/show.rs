//! Mail is untrusted text, so only header lines start at column 0 (`=== `), their fields cannot hold line breaks,
//! and every body line is indented.

use std::io::Write;

use chrono::{DateTime, SecondsFormat};

use crate::{
    address::MessageId,
    mail::{Addr, Body, Email, Keyword, MAX_BODY_BYTES, Part},
    out::is_receipt,
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

/// The mail of its thread a mail answers, by the In-Reply-To given, and the newest mail before it from anyone else
/// when that came after the one answered: the mail a quick reply is easily taken to answer.
#[derive(Clone, Copy, Default)]
pub struct Answering<'a> {
    pub answers: Option<&'a Email>,
    pub newest: Option<&'a Email>,
}

impl<'a> Answering<'a> {
    pub fn of(m: &Email, in_reply_to: &[String], thread: &'a [Email]) -> Answering<'a> {
        let named = MessageId::all(in_reply_to);
        let sender = |e: &Email| e.from.first().map(|a| a.email.to_lowercase());
        let others =
            || thread.iter().filter(|o| o.id != m.id && !is_receipt(o) && !o.has(Keyword::Draft));
        let answers =
            others().find(|o| MessageId::all(&o.message_id).iter().any(|id| named.contains(id)));
        let newest = others()
            .filter(|o| o.received <= m.received && sender(o) != sender(m))
            .max_by(|a, b| a.received.cmp(&b.received));
        Answering {
            answers,
            newest: newest.filter(|n| answers.is_some_and(|a| n.received > a.received)),
        }
    }

    fn line(&self) -> Option<String> {
        let one = |e: &Email| {
            format!("{} of {}: {}", plain(&e.id), plain(&e.received), plain(&e.subject))
        };
        let newest = self.newest.map(|n| format!(", not the newest in this thread ({})", one(n)));
        Some(format!("=== answers {}{}", one(self.answers?), newest.unwrap_or_default()))
    }

    /// `answers` and `newest` as a wait line names them.
    pub fn json(&self) -> [Value; 2] {
        [self.answers, self.newest].map(|e| {
            e.map_or(
                Value::Null,
                |e| json!({ "id": e.id, "receivedAt": e.received, "subject": e.subject }),
            )
        })
    }
}

pub fn inbox_line(m: &Email, files: &[Part]) -> String {
    let unread = if m.has(Keyword::Seen) { ' ' } else { '*' };
    let cc = if m.cc.is_empty() { String::new() } else { format!(" cc {}", emails(&m.cc)) };
    let files: String =
        files.iter().map(|a| format!(" [{} {}B]", plain(&a.name), a.size)).collect();
    format!("{unread} {}{cc}  {}{files}\n    {}", head(m), plain(&m.subject), preview(m, 160))
}

pub fn message(m: &Email, body: Option<&Body>, answering: Answering) -> String {
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
    lines.splice(0..0, answering.line());
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
