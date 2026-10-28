use serde_json::json;

use super::*;

const NOW: i64 = 1_800_000_000;
const ME: &str = "me@example.org";

fn judge() -> Judge {
    let seed = format!("{}Bwc", "BwcH".repeat(10));
    Judge {
        signer: Signer::from_seed(&seed).unwrap(),
        domains: vec!["example.org".into(), "agent.example.org".into()],
    }
}

/// A DATA-stage request as Stalwart sends one, each header value as written after its colon.
fn hook(sender: &str, headers: &[(&str, &str)], to: &[&str]) -> Hook {
    let contents = "yes\r\n";
    let size =
        headers.iter().map(|(n, v)| n.len() + 1 + v.len()).sum::<usize>() + 2 + contents.len();
    serde_json::from_value(json!({
        "context": {
            "stage": "data", "client": { "ip": "127.0.0.1", "port": 0, "activeConnections": 1 },
            "sasl": { "login": "owen" }, "server": { "port": 0 }, "protocol": { "version": 1 },
        },
        "envelope": { "from": { "address": sender }, "to": to.iter().map(|a| json!({ "address": a })).collect::<Vec<_>>() },
        "message": { "headers": headers, "contents": contents, "size": size },
    }))
    .unwrap()
}

fn from(value: &str) -> Vec<(&str, &str)> {
    vec![("From", value), ("To", " agent+deploy@example.org\r\n"), ("Subject", " deploy\r\n")]
}

#[test]
fn mail_to_a_local_domain_is_stamped_over_the_bytes_as_sent() {
    let headers = from(" Owen <me@example.org>\r\n");
    let Verdict::Stamp(value) =
        judge().judge(hook(ME, &headers, &["agent+deploy@example.org"]), NOW).unwrap()
    else {
        panic!("not stamped");
    };
    let sent = "From: Owen <me@example.org>\r\nTo: agent+deploy@example.org\r\nSubject: deploy\r\n\r\nyes\r\n";
    let delivered = format!("Received: from localhost\r\n{HEADER}: {value}\r\n{sent}");
    let verified = verify(&judge().signer.public(), delivered.as_bytes()).unwrap();
    assert_eq!(
        verified.stamp.to_string(),
        "v=1; account=owen; sender=me@example.org; t=1800000000"
    );
    assert_eq!(verified.message, sent.as_bytes());
}

#[test]
fn only_mail_with_a_recipient_at_a_listed_domain_is_stamped() {
    let stamped = |to: &[&str]| {
        matches!(
            judge().judge(hook(ME, &from(" me@example.org\r\n"), to), NOW).unwrap(),
            Verdict::Stamp(_)
        )
    };
    assert!(stamped(&["someone@else.org", "ann@AGENT.example.org"]));
    assert!(!stamped(&["someone@else.org"]));
    assert!(!stamped(&["agent@example.org.else.org"]));
}

#[test]
fn a_from_or_sender_that_is_not_the_senders_own_address_is_refused() {
    let verdict = |sender: &str, headers: Vec<(&str, &str)>| match judge()
        .judge(hook(sender, &headers, &["x@else.org"]), NOW)
        .unwrap()
    {
        Verdict::Refuse(why) => why,
        other => format!("{other:?}"),
    };
    let not_mine = "From is not me@example.org, the address this account sends from";
    let cases = [
        (ME, from(" me@example.org\r\n"), "Pass"),
        (ME, from(" Owen <ME+deploy@Example.org>\r\n"), "Pass"),
        ("agent+x@example.org", from(" agent+y@example.org\r\n"), "Pass"),
        (ME, from(" agent@example.org\r\n"), not_mine),
        (ME, from(" me@example.org.else.org\r\n"), not_mine),
        (ME, from(" \"me@example.org\" <agent@example.org>\r\n"), not_mine),
        (ME, from(" me@example.org (<agent@example.org>)\r\n"), "Pass"),
        (ME, from(" me@example.org, agent@example.org\r\n"), "From names 2 addresses, not one"),
        (ME, from(" undisclosed:;\r\n"), "From names 0 addresses, not one"),
        (ME, vec![("Subject", " x\r\n")], "0 From fields, not one"),
        (
            ME,
            [from(" me@example.org\r\n"), from(" me@example.org\r\n")].concat(),
            "2 From fields, not one",
        ),
        (
            ME,
            [from(" me@example.org\r\n"), vec![("Sender", " me@example.org\r\n")]].concat(),
            "Pass",
        ),
        (
            ME,
            [from(" me@example.org\r\n"), vec![("Sender", " agent@example.org\r\n")]].concat(),
            "Sender is not me@example.org, the address this account sends from",
        ),
        (ME, from(" me@example.org \u{fffd}\r\n"), "a header is not UTF-8"),
        ("", from(" me@example.org\r\n"), "\"\" is no address to send from"),
        (
            ME,
            [from(" me@example.org\r\n"), vec![(HEADER, " v=1\r\n")]].concat(),
            "a submitted message carries no Submission-Signature of its own",
        ),
    ];
    for (n, (sender, headers, want)) in cases.into_iter().enumerate() {
        assert_eq!(verdict(sender, headers), want, "case {n}");
    }
}

#[test]
fn unauthenticated_mail_passes_and_a_request_without_its_message_fails() {
    let mut unauthenticated = hook(ME, &from(" agent@example.org\r\n"), &["agent@example.org"]);
    unauthenticated.context.sasl = None;
    assert_eq!(judge().judge(unauthenticated, NOW).unwrap(), Verdict::Pass);
    let mut bare = hook(ME, &from(" me@example.org\r\n"), &["agent@example.org"]);
    bare.message = None;
    assert!(judge().judge(bare, NOW).is_err());
}

#[test]
fn a_message_whose_bytes_the_request_lost_goes_out_unstamped() {
    let mut lossy = hook(ME, &from(" me@example.org\r\n"), &["agent@example.org"]);
    lossy.message.as_mut().unwrap().size += 1;
    assert_eq!(judge().judge(lossy, NOW).unwrap(), Verdict::Pass);
}

#[test]
fn each_verdict_answers_as_stalwart_reads_a_hook_response() {
    assert_eq!(Verdict::Pass.answer(), json!({ "action": "accept" }));
    assert_eq!(
        Verdict::Stamp("v=1".into()).answer(),
        json!({ "action": "accept", "modifications": [{ "type": "addHeader", "name": HEADER, "value": "v=1" }] })
    );
    assert_eq!(
        Verdict::Refuse("no".into()).answer(),
        json!({ "action": "reject", "response": { "status": 550, "enhanced_status": "5.7.1", "message": "no" } })
    );
}
