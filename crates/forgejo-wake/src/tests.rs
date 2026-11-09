use serde_json::json;

use super::*;

fn router() -> Router {
    Router {
        to: "agent+{lane}@example.org".into(),
        from: "code@example.org".into(),
        label: "owner/".into(),
        routes: [("owen".into(), "dispatch".into())].into(),
        fallback: "dispatch".into(),
        quiet: vec!["agent".into()],
    }
}

fn hook(labels: &[&str], sender: &str, action: &str, title: &str) -> Hook {
    serde_json::from_value(json!({
        "action": action,
        "issue": {
            "number": 7,
            "title": title,
            "labels": labels.iter().map(|name| json!({ "name": name, "color": "e11d21" })).collect::<Vec<_>>(),
        },
        "comment": {
            "id": 41,
            "body": "@agent can you look\r\n. a line with a dot",
            "html_url": "https://code.example.org/agents/work/issues/7#issuecomment-41",
        },
        "repository": { "full_name": "agents/work" },
        "sender": { "login": sender, "full_name": "Owen" },
        "is_pull": false,
    }))
    .unwrap()
}

fn to(labels: &[&str]) -> String {
    router().mail(&hook(labels, "ironmoon", "created", "t"), "d").unwrap().to
}

#[test]
fn a_comment_goes_to_the_lane_its_owner_label_names_and_to_the_fallback_without_one() {
    assert_eq!(to(&["priority/high", "owner/protocols"]), "agent+protocols@example.org");
    assert_eq!(to(&["priority/high"]), "agent+dispatch@example.org");
    assert_eq!(
        to(&["owner/owen"]),
        "agent+dispatch@example.org",
        "a lane mailed in another's place"
    );
    assert_eq!(to(&["owner/a b@evil.org"]), "agent+dispatch@example.org", "no lane");
}

#[test]
fn a_quiet_user_s_comment_and_an_edit_wake_no_one() {
    let router = router();
    assert_eq!(
        router.mail(&hook(&["owner/protocols"], "agent", "created", "t"), "d"),
        Err("a comment by agent".into())
    );
    assert_eq!(
        router.mail(&hook(&["owner/protocols"], "ironmoon", "edited", "t"), "d"),
        Err("a comment edited".into())
    );
}

#[test]
fn the_mail_threads_under_forgejo_s_own_for_the_issue_and_carries_the_comment_and_its_link() {
    let mail = router()
        .mail(
            &hook(&["owner/protocols"], "ironmoon", "created", "deploy it"),
            "Sun, 8 Nov 2026 20:00:00 -0500",
        )
        .unwrap();
    let (head, body) = mail.text.split_once("\r\n\r\n").unwrap();
    assert_eq!(
        head.split("\r\n").collect::<Vec<_>>(),
        [
            "From: code@example.org",
            "To: agent+protocols@example.org",
            "Subject: Re: [agents/work] deploy it (#7)",
            "Date: Sun, 8 Nov 2026 20:00:00 -0500",
            "Message-ID: <agents/work/issues/7/comment/41/wake@code.example.org>",
            "In-Reply-To: <agents/work/issues/7@code.example.org>",
            "References: <agents/work/issues/7@code.example.org>",
            "MIME-Version: 1.0",
            "Content-Type: text/plain; charset=utf-8",
            "Content-Transfer-Encoding: base64",
        ]
    );
    assert!(body.split("\r\n").all(|line| line.len() <= 76));
    let decoded = String::from_utf8(BASE64.decode(body.replace("\r\n", "")).unwrap()).unwrap();
    assert_eq!(
        decoded,
        "ironmoon commented on agents/work#7, deploy it:\n\n@agent can you look\r\n. a line with a dot\n\n\
         https://code.example.org/agents/work/issues/7#issuecomment-41\n"
    );
}

#[test]
fn a_subject_that_isn_t_ascii_is_encoded() {
    let mail = router().mail(&hook(&[], "ironmoon", "created", "déployer"), "d").unwrap();
    let subject = mail.text.split("\r\n").find(|l| l.starts_with("Subject: ")).unwrap();
    assert_eq!(subject, "Subject: =?utf-8?b?UmU6IFthZ2VudHMvd29ya10gZMOpcGxveWVyICgjNyk=?=");
}

#[test]
fn only_the_secret_s_hmac_sha256_signs_a_request() {
    // RFC 4231's test case 2
    let key = hmac::Key::new(hmac::HMAC_SHA256, b"Jefe");
    let body = b"what do ya want for nothing?";
    let tag = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
    assert!(signed(&key, body, Some(tag)));
    assert!(!signed(&key, b"what do ya want for something?", Some(tag)));
    assert!(!signed(&key, body, Some("not hex")));
    assert!(!signed(&key, body, None));
}
