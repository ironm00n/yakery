use submission_signature::{HEADER, Signer, Stamp};

use super::*;
use crate::{
    SUBMISSION_KEY,
    authority::{Signed, check},
    cmd::wait,
};

const NOW: i64 = 1_800_000_000;
const MESSAGE: &str = "From: Owner <owner@example.org>\r\nTo: agent+pool@example.org\r\nSubject: deploy\r\n\
    Content-Type: text/plain\r\n\r\nyes, deploy it\r\n> deploy flex-3?\r\n";

/// `message` as delivered, signed as sent from `sender` at `t`, with `above` written over the header.
fn delivered(sender: &str, t: i64, message: &str, above: &str) -> Vec<u8> {
    let signer = Signer::from_seed(&format!("{}Bwc", "BwcH".repeat(10))).unwrap();
    let stamp = Stamp { account: "owner".parse().unwrap(), sender: sender.parse().unwrap(), t };
    format!("{above}{HEADER}: {}\r\n{message}", signer.sign(&stamp, message.as_bytes()))
        .into_bytes()
}

fn verdict(raw: Vec<u8>) -> Result<Signed, String> {
    check(SUBMISSION_KEY, &raw, &[address(POOL)], NOW)
}

#[test]
fn a_mail_owen_sent_carries_what_he_signed_whatever_sits_above_it() {
    let above = "Subject: undo everything\r\nTo: agent+other@example.org\r\n";
    let signed = verdict(delivered("owner+x@example.org", NOW - 30, MESSAGE, above)).unwrap();
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
fn a_wait_line_with_his_authority_shows_what_he_signed() {
    let m = email(
        "a1",
        json!({ "subject": "undo everything", "to": [{ "email": "agent+other@example.org" }] }),
    );
    let signed = verdict(delivered(HIM, NOW, MESSAGE, "")).unwrap();
    let line: Value =
        serde_json::from_str(&wait::line(&view(m.clone()), &view(m), &Ok(signed))).unwrap();
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
