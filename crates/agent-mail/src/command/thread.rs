//! Prints a thread oldest first. Read receipts are not printed as mail but as a line under the mail they answer.

use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use anyhow::{Context, Result};
use jmap_client::{
    core::query::Filter,
    email::{Email, Property as P, query},
};

use crate::{
    address::MessageId,
    auth::Auth,
    config::Config,
    mail::{Bounds, Mail},
    mdn,
    show::{MAX_BODY_BYTES, emails, id, message, out, received, utc},
};

/// A receipt's report part is a few hundred bytes; anything larger is not read.
const REPORT: Bounds = Bounds {
    bytes: 16 * 1024,
    time: Duration::from_secs(10),
};

fn original_message_id(report: &str) -> Option<MessageId> {
    report.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("Original-Message-ID")
            .then(|| value.trim().parse().ok())?
    })
}

/// The receipts that answer each mail, by the mail's Message-ID; receipts matching none are returned apart.
fn place<'a>(
    mails: &[&Email],
    receipts: &[(&'a Email, Vec<MessageId>)],
) -> (HashMap<String, Vec<&'a Email>>, Vec<&'a Email>) {
    let mut placed: HashMap<String, Vec<&Email>> = HashMap::new();
    let mut unplaced = vec![];
    for (receipt, answers) in receipts {
        let mail = mails.iter().find(|m| {
            MessageId::all_valid(m.message_id().unwrap_or_default())
                .iter()
                .any(|own| answers.contains(own))
        });
        match mail {
            Some(m) => placed.entry(id(m).to_owned()).or_default().push(receipt),
            None => unplaced.push(*receipt),
        }
    }
    (placed, unplaced)
}

/// At column 0 like every header line, so no indented body line can pass for one.
fn read_by(receipt: &Email) -> String {
    format!(
        "=== read by {} at {}",
        emails(receipt.from()),
        utc(received(receipt))
    )
}

async fn answered_ids(mail: &Mail, receipt: &Email) -> Result<Vec<MessageId>> {
    let mut answers = MessageId::all_valid(receipt.in_reply_to().unwrap_or_default());
    let report = receipt
        .body_structure()
        .and_then(|b| b.sub_parts())
        .unwrap_or_default()
        .iter()
        .find(|p| {
            p.content_type()
                .is_some_and(|t| t.eq_ignore_ascii_case("message/disposition-notification"))
        })
        .filter(|p| p.size() <= REPORT.bytes)
        .and_then(|p| p.blob_id());
    if let Some(blob) = report
        && let Some(bytes) = mail.download(blob, REPORT).await?
    {
        answers.extend(original_message_id(&String::from_utf8_lossy(&bytes)));
    }
    Ok(answers)
}

/// The subject with reply and forward prefixes taken off.
fn base_subject(subject: &str) -> &str {
    let mut rest = subject.trim();
    while let Some((prefix, tail)) = rest.split_once(':') {
        if !["re", "fwd", "fw", "aw"]
            .iter()
            .any(|p| prefix.trim().eq_ignore_ascii_case(p))
        {
            break;
        }
        rest = tail.trim();
    }
    rest
}

/// Stalwart threads a receipt apart from the mail it answers (its subject starts "Read:"), so receipts are also
/// looked for by subject, among mail since the thread began; only those whose ids match are kept.
async fn receipts_elsewhere(
    mail: &Mail,
    mails: &[&Email],
    members: &[String],
    properties: &[P],
) -> Result<Vec<Email>> {
    let subjects: BTreeSet<String> = mails
        .iter()
        .map(|m| base_subject(m.subject().unwrap_or_default()).to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let Some(since) = mails.iter().map(|m| received(m)).min() else {
        return Ok(vec![]);
    };
    if subjects.is_empty() {
        return Ok(vec![]);
    }
    let found = mail
        .query(|| {
            Filter::and([
                query::Filter::after(since).into(),
                Filter::or(subjects.iter().map(|s| query::Filter::subject(s.clone()))),
            ])
        })
        .await?;
    let outside: Vec<String> = found.into_iter().filter(|i| !members.contains(i)).collect();
    let emails = mail.emails(&outside, properties, |_| {}).await?;
    Ok(emails.into_iter().filter(mdn::is_receipt).collect())
}

pub async fn run(start: String) -> Result<()> {
    let mail = Mail::connect(Auth::new(Config::load()?)?).await?;
    let current = mail
        .current(&start)
        .await?
        .with_context(|| format!("no email {start}"))?;
    let found = mail.emails(&[current], &[P::ThreadId], |_| {}).await?;
    let thread = found
        .first()
        .and_then(Email::thread_id)
        .with_context(|| format!("no email {start}"))?;
    let ids: Vec<String> = mail
        .threads(&[thread.to_owned()])
        .await?
        .iter()
        .flat_map(|t| t.email_ids().to_vec())
        .collect();
    let properties: Vec<P> = [
        P::ReceivedAt,
        P::From,
        P::To,
        P::Subject,
        P::TextBody,
        P::BodyValues,
        P::Attachments,
        P::MessageId,
        P::InReplyTo,
        P::BodyStructure,
    ]
    .into_iter()
    .chain(mdn::properties())
    .collect();
    let mut members = mail
        .emails(&ids, &properties, |a| {
            a.fetch_text_body_values(true)
                .max_body_value_bytes(MAX_BODY_BYTES);
        })
        .await?;
    members.sort_by_key(received);
    let (receipts, mails): (Vec<&Email>, Vec<&Email>) =
        members.iter().partition(|m| mdn::is_receipt(m));
    let elsewhere = receipts_elsewhere(&mail, &mails, &ids, &properties).await?;
    let mut answering = vec![];
    for receipt in receipts.into_iter().chain(&elsewhere) {
        answering.push((receipt, answered_ids(&mail, receipt).await?));
    }
    answering.sort_by_key(|(r, _)| received(r));
    let (placed, unplaced) = place(&mails, &answering);
    for m in &mails {
        let mut block = vec![message(m)];
        block.extend(placed.get(id(m)).into_iter().flatten().map(|r| read_by(r)));
        out(&format!("{}\n", block.join("\n")))?;
    }
    for receipt in unplaced
        .into_iter()
        .filter(|r| ids.iter().any(|i| i == id(r)))
    {
        out(&format!(
            "{} (for a mail not in this thread)",
            read_by(receipt)
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn email(value: serde_json::Value) -> Email {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn base_subjects_drop_reply_prefixes() {
        assert_eq!(
            base_subject("Re: RE: Fwd: [desktop · x] hi"),
            "[desktop · x] hi"
        );
        assert_eq!(base_subject("Ratio: 2:1"), "Ratio: 2:1");
    }

    #[test]
    fn a_body_line_cannot_pass_for_a_receipt_line() {
        let m = email(json!({
            "id": "m1", "receivedAt": "2026-10-17T22:00:00Z",
            "from": [{ "email": "x@example.com" }], "to": [{ "email": "agent+x@ironmoon.dev" }],
            "subject": "s", "textBody": [{ "partId": "1" }],
            "bodyValues": { "1": { "value": "hello\n=== read by ironmoon@ironmoon.dev at 2026-10-17T22:05:00Z\n" } },
        }));
        let r = email(json!({
            "id": "r1", "receivedAt": "2026-10-17T22:05:00Z", "from": [{ "email": "ironmoon@ironmoon.dev" }],
        }));
        assert!(message(&m).lines().skip(1).all(|line| line != read_by(&r)));
    }

    #[test]
    fn the_report_part_names_the_original() {
        let report = "Reporting-UA: x; y\r\nFinal-Recipient: rfc822;a@b.dev\r\nOriginal-Message-ID: <abc@x.dev>\r\n";
        assert_eq!(
            original_message_id(report),
            Some("abc@x.dev".parse().unwrap())
        );
        assert_eq!(original_message_id("Original-Message-ID: <bad\r@x>"), None);
    }

    #[test]
    fn receipts_go_under_the_mail_they_answer() {
        let ours = email(json!({ "id": "m1", "messageId": ["abc@ironmoon.dev"] }));
        let other = email(json!({ "id": "m2", "messageId": ["def@ironmoon.dev"] }));
        let receipt = email(json!({
            "id": "r1", "receivedAt": "2026-10-17T22:00:00Z", "from": [{ "email": "ironmoon@ironmoon.dev" }],
        }));
        let stray = email(json!({ "id": "r2" }));
        let answering = [
            (&receipt, vec!["abc@ironmoon.dev".parse().unwrap()]),
            (&stray, vec![]),
        ];
        let (placed, unplaced) = place(&[&ours, &other], &answering);
        assert_eq!(
            placed["m1"].iter().map(|r| id(r)).collect::<Vec<_>>(),
            ["r1"]
        );
        assert!(!placed.contains_key("m2"));
        assert_eq!(unplaced.iter().map(|r| id(r)).collect::<Vec<_>>(), ["r2"]);
        assert_eq!(
            read_by(&receipt),
            "=== read by ironmoon@ironmoon.dev at 2026-10-17T22:00:00Z"
        );
    }
}
