//! Read receipts (RFC 8098 MDNs) for delivered mail that asks for one, built and sent the way Bulwark sends them:
//! a raw multipart/report imported into Sent, then submitted to the requester.

use anyhow::Result;
use chrono::DateTime;
use jmap_client::email::{self, Email, Header, HeaderValue};

use crate::{
    address::{Address, MessageId},
    identity::Login,
    mail::{AGENT_MAIL, MDN_SENT, Mail, SEEN, has_keyword},
    mime,
    outbox::{Outbox, Outcome, require_all},
    policy::{Recipients, bare_sender},
    random::{ALPHANUMERIC, Draws},
    show::{addresses, plain},
};

pub fn properties() -> [email::Property; 3] {
    [notification_to(), original_recipient(), content_type()].map(email::Property::Header)
}

fn notification_to() -> Header {
    Header::as_addresses("Disposition-Notification-To", false)
}

fn original_recipient() -> Header {
    Header::as_text("Original-Recipient", false)
}

fn content_type() -> Header {
    Header::as_raw("Content-Type", false)
}

fn text<'a>(m: &'a Email, header: &Header) -> Option<&'a str> {
    match m.header(header) {
        Some(HeaderValue::AsText(v)) => Some(v.trim()),
        _ => None,
    }
}

/// A multipart/report of report-type disposition-notification; needs `properties()` fetched.
pub fn is_receipt(m: &Email) -> bool {
    text(m, &content_type()).is_some_and(|t| {
        let t = t.to_ascii_lowercase();
        t.starts_with("multipart/report") && t.contains("disposition-notification")
    })
}

/// What a delivered mail's receipt request comes to.
pub enum Asked {
    Nothing,
    /// Not sent, but `$MDNSent` is still set, as RFC 3503 §3 asks of automatic processing.
    Declined(String),
    Send(Receipt),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Receipt {
    to: Address,
    /// The address the mail reached us at, which the receipt is sent from.
    final_recipient: Address,
    original_recipient: Option<Address>,
    subject: String,
    original_message_id: Option<MessageId>,
}

impl Asked {
    /// Receipts go only to the sender: RFC 8098's Return-Path rule can't apply, since Stalwart adds no Return-Path
    /// to mail it delivers locally, and this keeps a third party from having us mail someone else.
    pub fn of(login: &Login, m: &Email) -> Asked {
        let asked: Vec<&str> = match m.header(&notification_to()) {
            Some(HeaderValue::AsAddresses(list)) => addresses(Some(list)).collect(),
            _ => vec![],
        };
        let Some(&to) = asked.first() else {
            return Asked::Nothing;
        };
        if has_keyword(m, MDN_SENT) {
            return Asked::Nothing;
        }
        let decline = |why: String| {
            Asked::Declined(plain(&format!(
                "no receipt for {}: {why}",
                m.id().unwrap_or_default()
            )))
        };
        if is_receipt(m) {
            return decline("it is itself a receipt".into());
        }
        let parsed: Vec<Option<Address>> = asked.iter().map(|a| a.parse().ok()).collect();
        let Some(Some(requester)) = parsed.first().cloned() else {
            return decline(format!("{to} is not a plain address"));
        };
        if parsed.iter().any(|a| a.as_ref() != Some(&requester)) {
            return decline(format!(
                "it asks for receipts to several addresses ({})",
                asked.join(", ")
            ));
        }
        let senders: Vec<&str> = addresses(m.from()).collect();
        if !matches!(senders.as_slice(), [sender] if sender.parse().ok() == Some(requester.clone()))
        {
            return decline(format!(
                "it asks for a receipt to {to}, not to its sender {}",
                senders.join(", ")
            ));
        }
        if let Err(e) = Recipients::check(login, vec![requester.clone()], vec![]) {
            return decline(e.to_string());
        }
        let reached: Option<Address> = login
            .first_topic_address(addresses(m.to()).chain(addresses(m.cc())))
            .and_then(|a| a.parse().ok());
        let original_recipient = text(m, &original_recipient())
            .and_then(|v| v.split_once(';'))
            .and_then(|(_, address)| address.trim().parse().ok())
            .or_else(|| reached.clone());
        let final_recipient = match reached.map(Ok).unwrap_or_else(|| bare_sender(login)) {
            Ok(address) => address,
            Err(e) => return decline(e.to_string()),
        };
        Asked::Send(Receipt {
            to: requester,
            final_recipient,
            original_recipient,
            subject: m.subject().unwrap_or_default().to_owned(),
            original_message_id: m
                .message_id()
                .and_then(|ids| MessageId::all_valid(ids).into_iter().next()),
        })
    }
}

impl Receipt {
    pub fn to(&self) -> &Address {
        &self.to
    }

    /// Addresses and ids are header-safe by construction; the subject goes through `mime::text`.
    pub fn message(&self, date_secs: i64, token: &str) -> String {
        let (to, from) = (&self.to, &self.final_recipient);
        let domain = from.domain();
        let boundary = format!("----=_MDN_{token}");
        let date = DateTime::from_timestamp(date_secs, 0)
            .unwrap_or_default()
            .to_rfc2822();

        let mut head = vec![
            format!("Date: {date}"),
            format!("From: {from}"),
            format!("To: {to}"),
            format!(
                "Subject: {}",
                mime::text(&format!("Read: {}", self.subject))
            ),
            format!("Message-ID: <mdn.{token}@{domain}>"),
        ];
        head.extend(
            self.original_message_id
                .iter()
                .map(|id| format!("In-Reply-To: {id}")),
        );
        head.extend([
            "MIME-Version: 1.0".to_owned(),
            "Content-Type: multipart/report; report-type=disposition-notification;".to_owned(),
            format!("\tboundary=\"{boundary}\""),
        ]);
        let human = [
            format!("This is a return receipt for the message you sent to {from}."),
            String::new(),
            "Note: This receipt only acknowledges that agent-mail displayed the message".to_owned(),
            "to an agent. There is no guarantee that it was read or understood.".to_owned(),
        ]
        .join("\r\n");
        let mut report = vec![
            format!("Reporting-UA: {domain}; agent-mail"),
            format!("Final-Recipient: rfc822;{from}"),
        ];
        report.extend(
            self.original_recipient
                .iter()
                .map(|a| format!("Original-Recipient: rfc822;{a}")),
        );
        report.extend(
            self.original_message_id
                .iter()
                .map(|id| format!("Original-Message-ID: {id}")),
        );
        report.push("Disposition: automatic-action/MDN-sent-automatically; displayed".to_owned());

        [
            head.join("\r\n"),
            String::new(),
            format!("--{boundary}"),
            "Content-Type: text/plain; charset=utf-8".to_owned(),
            "Content-Transfer-Encoding: base64".to_owned(),
            String::new(),
            mime::base64_body(&human),
            String::new(),
            format!("--{boundary}"),
            "Content-Type: message/disposition-notification".to_owned(),
            "Content-Transfer-Encoding: 7bit".to_owned(),
            String::new(),
            report.join("\r\n"),
            String::new(),
            format!("--{boundary}--"),
            String::new(),
        ]
        .join("\r\n")
    }

    /// The copy is destroyed only when it is known that nothing was sent: no submission created, or every
    /// recipient refused. When the outcome is unknown it is left, and said so.
    pub async fn send(&self, mail: &Mail, outbox: &Outbox, now: i64) -> Result<()> {
        let token = Draws::default().string(ALPHANUMERIC, 24)?;
        let message = self.message(now, &token).into_bytes();
        let copy = &mail
            .import(message, &outbox.sent, &[SEEN, MDN_SENT, AGENT_MAIL])
            .await?;
        let unsent = match outbox
            .send(mail, copy, std::slice::from_ref(&self.to))
            .await
        {
            Outcome::Submitted { id, delivery } => match require_all(&id, &delivery) {
                Ok(()) => return Ok(()),
                Err(refused) => refused,
            },
            Outcome::Unsent(e) => e,
            Outcome::Unknown(e) => {
                return Err(e.context(format!(
                    "not known to be sent; its copy {copy} stays in Sent"
                )));
            }
        };
        Err(match mail.discard_unsent(copy).await {
            Ok(()) => unsent.context("the receipt was not sent"),
            Err(d) => unsent.context(format!(
                "the receipt was not sent, and its copy {copy} could not be removed: {d:#}"
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn login() -> Login {
        "agent@ironmoon.dev".parse().unwrap()
    }

    fn address(a: &str) -> Address {
        a.parse().unwrap()
    }

    fn mail(extra: Value) -> Email {
        let mut m = json!({
            "id": "m1", "subject": "Re: [desktop · pool] x", "messageId": ["abc@ironmoon.dev"],
            "from": [{ "email": "ironmoon@ironmoon.dev" }],
            "to": [{ "email": "agent+pool@ironmoon.dev" }], "keywords": {},
            "header:Disposition-Notification-To:asAddresses": [{ "email": "ironmoon@ironmoon.dev" }],
            "header:Content-Type": " text/plain",
        });
        m.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(m).unwrap()
    }

    fn receipt(m: &Email) -> Receipt {
        match Asked::of(&login(), m) {
            Asked::Send(receipt) => receipt,
            _ => panic!("no receipt"),
        }
    }

    fn declined(m: &Email) -> String {
        match Asked::of(&login(), m) {
            Asked::Declined(why) => why,
            _ => panic!("not declined"),
        }
    }

    #[test]
    fn a_request_gets_a_receipt_from_the_address_it_reached() {
        assert_eq!(
            receipt(&mail(json!({}))),
            Receipt {
                to: address("ironmoon@ironmoon.dev"),
                final_recipient: address("agent+pool@ironmoon.dev"),
                original_recipient: Some(address("agent+pool@ironmoon.dev")),
                subject: "Re: [desktop · pool] x".into(),
                original_message_id: Some("abc@ironmoon.dev".parse().unwrap()),
            }
        );
    }

    #[test]
    fn without_a_topic_address_the_receipt_comes_from_the_login() {
        let ada: Login = "ada@agent.ironmoon.dev".parse().unwrap();
        let to_ada = |extra: Value| {
            let mut m = json!({ "to": [{ "email": "ada@agent.ironmoon.dev" }] });
            m.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            match Asked::of(&ada, &mail(m)) {
                Asked::Send(receipt) => receipt,
                _ => panic!("no receipt"),
            }
        };
        let bare = to_ada(json!({}));
        assert_eq!(
            (bare.final_recipient, bare.original_recipient),
            (address("ada@agent.ironmoon.dev"), None)
        );
        let header = to_ada(json!({
            "header:Original-Recipient:asText": " rfc822; ada+old@agent.ironmoon.dev",
        }));
        assert_eq!(
            header.original_recipient,
            Some(address("ada+old@agent.ironmoon.dev"))
        );
    }

    #[test]
    fn the_shared_login_sends_no_receipt_from_its_bare_address() {
        let why = declined(&mail(json!({ "to": [{ "email": "agent@ironmoon.dev" }] })));
        assert!(why.contains("sends only from a topic subaddress"), "{why}");
    }

    #[test]
    fn no_request_or_an_earlier_receipt_means_nothing_to_do() {
        let none = mail(json!({ "header:Disposition-Notification-To:asAddresses": null }));
        assert!(matches!(Asked::of(&login(), &none), Asked::Nothing));
        let sent = mail(json!({ "keywords": { "$mdnsent": true } }));
        assert!(matches!(Asked::of(&login(), &sent), Asked::Nothing));
    }

    #[test]
    fn a_request_for_a_receipt_to_anyone_but_the_sender_is_declined() {
        let other = mail(
            json!({ "header:Disposition-Notification-To:asAddresses": [{ "email": "me@ironmoon.dev" }] }),
        );
        assert!(
            declined(&other).contains("not to its sender ironmoon@ironmoon.dev"),
            "{}",
            declined(&other)
        );
        let case = mail(
            json!({ "header:Disposition-Notification-To:asAddresses": [{ "email": "IronMoon@IronMoon.dev" }] }),
        );
        assert!(matches!(Asked::of(&login(), &case), Asked::Send(_)));
        let two_senders = mail(
            json!({ "from": [{ "email": "ironmoon@ironmoon.dev" }, { "email": "me@ironmoon.dev" }] }),
        );
        assert!(declined(&two_senders).contains("not to its sender"));
    }

    #[test]
    fn receipts_and_requests_to_several_addresses_are_declined() {
        let several = mail(json!({ "header:Disposition-Notification-To:asAddresses": [
            { "email": "ironmoon@ironmoon.dev" }, { "email": "me@ironmoon.dev" }] }));
        assert!(declined(&several).contains("several addresses"));
        let receipt = mail(json!({ "header:Content-Type":
            " multipart/report; report-type=disposition-notification; boundary=x" }));
        assert!(declined(&receipt).contains("itself a receipt"));
    }

    #[test]
    fn the_recipient_policy_applies_to_receipts() {
        let outside = mail(json!({
            "from": [{ "email": "a@example.com" }],
            "header:Disposition-Notification-To:asAddresses": [{ "email": "a@example.com" }],
        }));
        assert!(declined(&outside).contains("may mail only"));
        let vm: Login = "ada@agent.ironmoon.dev".parse().unwrap();
        assert!(matches!(Asked::of(&vm, &outside), Asked::Send(_)));
    }

    #[test]
    fn an_invalid_original_message_id_is_dropped() {
        let bad = receipt(&mail(json!({ "messageId": ["abc\r@ironmoon.dev"] })));
        assert_eq!(bad.original_message_id, None);
        assert!(!bad.message(1_792_000_827, "T").contains("In-Reply-To"));
    }

    #[test]
    fn the_receipt_is_an_rfc_8098_report_shaped_like_bulwarks() {
        let expected = [
            "Date: Wed, 14 Oct 2026 18:00:27 +0000",
            "From: agent+pool@ironmoon.dev",
            "To: ironmoon@ironmoon.dev",
            "Subject: =?UTF-8?B?UmVhZDogUmU6IFtkZXNrdG9wIMK3IHBvb2xdIHg=?=",
            "Message-ID: <mdn.T@ironmoon.dev>",
            "In-Reply-To: <abc@ironmoon.dev>",
            "MIME-Version: 1.0",
            "Content-Type: multipart/report; report-type=disposition-notification;",
            "\tboundary=\"----=_MDN_T\"",
            "",
            "------=_MDN_T",
            "Content-Type: text/plain; charset=utf-8",
            "Content-Transfer-Encoding: base64",
            "",
            &mime::base64_body(
                "This is a return receipt for the message you sent to agent+pool@ironmoon.dev.\r\n\r\n\
                 Note: This receipt only acknowledges that agent-mail displayed the message\r\n\
                 to an agent. There is no guarantee that it was read or understood.",
            ),
            "",
            "------=_MDN_T",
            "Content-Type: message/disposition-notification",
            "Content-Transfer-Encoding: 7bit",
            "",
            "Reporting-UA: ironmoon.dev; agent-mail",
            "Final-Recipient: rfc822;agent+pool@ironmoon.dev",
            "Original-Recipient: rfc822;agent+pool@ironmoon.dev",
            "Original-Message-ID: <abc@ironmoon.dev>",
            "Disposition: automatic-action/MDN-sent-automatically; displayed",
            "",
            "------=_MDN_T--",
            "",
        ]
        .join("\r\n");
        assert_eq!(
            receipt(&mail(json!({}))).message(1_792_000_827, "T"),
            expected
        );
        assert!(
            !expected
                .to_ascii_lowercase()
                .contains("disposition-notification-to")
        );
    }

    #[test]
    fn a_hostile_subject_cannot_add_headers() {
        let hostile =
            mail(json!({ "subject": "hi\r\nBcc: victim@example.com\r\n\r\nforged body" }));
        let message = receipt(&hostile).message(1_792_000_827, "T");
        assert!(!message.contains("\r\nBcc:"), "{message}");
    }
}
