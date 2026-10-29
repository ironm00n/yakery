use submission_signature::{HEADER, Signer, Stamp};

use super::*;
use crate::{
    SUBMISSION_KEY,
    authority::{Signed, check},
    cmd::wait,
    mail::Email,
};

const NOW: i64 = 1_800_000_000;
const MESSAGE: &str = "From: Owner <owner@example.org>\r\nTo: agent+pool@example.org\r\nSubject: deploy\r\n\
    Content-Type: text/plain\r\n\r\nyes, deploy it\r\n> deploy flex-3?\r\n";

/// `message` as delivered, signed for `account` as sent from `sender` at `t`, with `above` written over the header.
fn delivered_for(account: &str, sender: &str, t: i64, message: &str, above: &str) -> Vec<u8> {
    let signer = Signer::from_seed(&format!("{}Bwc", "BwcH".repeat(10))).unwrap();
    let stamp = Stamp { account: account.parse().unwrap(), sender: sender.parse().unwrap(), t };
    format!("{above}{HEADER}: {}\r\n{message}", signer.sign(&stamp, message.as_bytes()))
        .into_bytes()
}

fn delivered(sender: &str, t: i64, message: &str, above: &str) -> Vec<u8> {
    delivered_for(HIM, sender, t, message, above)
}

fn verdict(raw: Vec<u8>) -> Result<Signed, String> {
    check(SUBMISSION_KEY, &raw, &[address(POOL)], NOW)
}

#[test]
fn a_mail_owen_sent_carries_what_he_signed_whatever_sits_above_it() {
    let above = "Subject: undo everything\r\nTo: agent+other@example.org\r\n";
    let signed = verdict(delivered(HIM, NOW - 30, MESSAGE, above)).unwrap();
    assert_eq!(
        (signed.from, signed.to, signed.subject, signed.text),
        (
            "owner@example.org".into(),
            vec![POOL.into()],
            "deploy".into(),
            "yes, deploy it\n> deploy flex-3?\n".into()
        )
    );
}

#[test]
fn each_way_a_mail_falls_short_of_his_word_names_why() {
    let mut changed = delivered(HIM, NOW, MESSAGE, "");
    *changed.last_mut().unwrap() = b'!';
    rows!(|raw| verdict(raw).unwrap_err();
        delivered_for("agent@example.org", HIM, NOW, MESSAGE, "") => "signed for the account agent@example.org, not Owen's";
        delivered_for("owner+x@example.org", HIM, NOW, MESSAGE, "") => "signed for the account owner+x@example.org, not Owen's";
        delivered("owner+x@example.org", NOW, MESSAGE, "") => "signed as sent from owner+x@example.org, not one of Owen's addresses";
        delivered("agent@example.org", NOW, MESSAGE, "") => "signed as sent from agent@example.org, not one of Owen's addresses";
        delivered("owner@example.com", NOW, MESSAGE, "") => "signed as sent from owner@example.com, not one of Owen's addresses";
        delivered(HIM, NOW - 601, MESSAGE, "") => "signed 601 s ago, over 600 s";
        delivered(HIM, NOW + 61, MESSAGE, "") => "signed 61 s ahead of this clock, over 60 s";
        delivered(HIM, NOW, &MESSAGE.replace("agent+pool", "agent+other"), "") => "signed, but to no topic this wait watches";
        changed => "the signature does not verify";
        MESSAGE.as_bytes().to_vec() => "0 Submission-Signature headers, not one";
    );
}

#[test]
fn each_spelling_of_one_signature_is_one_approval() {
    let raw = String::from_utf8(delivered(HIM, NOW, MESSAGE, "")).unwrap();
    let approval = |raw: String| verdict(raw.into_bytes()).unwrap().signature;
    let padded = |pad: &str| raw.replacen("\r\nFrom:", &format!("{pad}\r\nFrom:"), 1);
    assert_eq!(approval(padded("=")), approval(raw.clone()));
    assert_eq!(approval(padded("==")), approval(raw));
}

#[test]
fn a_wait_line_with_his_authority_shows_what_he_signed() {
    let m = email(
        "a1",
        json!({ "subject": "undo everything", "to": [{ "email": "agent+other@example.org" }] }),
    );
    let signed = verdict(delivered(HIM, NOW, MESSAGE, "")).unwrap();
    let line: Value =
        serde_json::from_str(&wait::line(&view(m.clone()), &view(m), &Ok(signed), &[])).unwrap();
    assert_eq!(
        [&line["subject"], &line["to"], &line["authority"], &line["authorityReason"]],
        [
            &json!("deploy"),
            &json!([POOL]),
            &json!(true),
            &json!("Stalwart signed it as sent from Owen's account")
        ]
    );
    assert!(line["approval"].as_str().is_some_and(|s| s.len() == 86), "{line}");
}

#[test]
fn the_mail_his_reply_answers_is_the_one_his_signed_bytes_name() {
    let thread: Vec<Email> = vec![
        view(email("a1", json!({ "from": [{ "email": POOL }] }))),
        view(email("a2", json!({ "from": [{ "email": POOL }] }))),
    ];
    let replying = MESSAGE.replacen("Subject:", "In-Reply-To: <a1@example.org>\r\nSubject:", 1);
    let signed = verdict(delivered(HIM, NOW, &replying, "In-Reply-To: <a2@example.org>\r\n"));
    let m = email("r1", json!({ "inReplyTo": ["a2@example.org"] }));
    let answers = |verdict: &Result<Signed, String>| {
        let line: Value =
            serde_json::from_str(&wait::line(&view(m.clone()), &view(m.clone()), verdict, &thread))
                .unwrap();
        line["answers"]["id"].clone()
    };
    assert_eq!(answers(&signed), json!("a1"));
    assert_eq!(answers(&Err("unsigned".into())), json!("a2"));
}
