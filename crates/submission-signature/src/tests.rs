use super::*;

fn seed() -> String {
    B64.encode([7; 32])
}
const MESSAGE: &[u8] = b"From: Owen <me@example.org>\r\nTo: agent+deploy@example.org\r\nSubject: deploy\r\n\r\nyes\r\n";
const TRACE: &[u8] = b"Received: from localhost\r\nReturn-Path: <me@example.org>\r\n";

fn stamp() -> Stamp {
    Stamp {
        account: "owen".parse().unwrap(),
        sender: "me@example.org".parse().unwrap(),
        t: 1_800_000_000,
    }
}

/// The message as delivered: Stalwart's trace headers above the signed header line, the message as sent below it.
fn delivered(value: &str, message: &[u8]) -> Vec<u8> {
    [TRACE, format!("{HEADER}: {value}\r\n").as_bytes(), message].concat()
}

#[test]
fn a_signed_message_verifies_below_the_trace_headers_and_yields_the_bytes_as_sent() {
    let signer = Signer::from_seed(&seed()).unwrap();
    let value = signer.sign(&stamp(), MESSAGE);
    assert!(
        value.starts_with("v=1; account=owen; sender=me@example.org; t=1800000000; sig="),
        "{value}"
    );
    let raw = delivered(&value, MESSAGE);
    let verified = verify(&signer.public(), &raw).unwrap();
    assert_eq!((&verified.stamp, verified.message), (&stamp(), MESSAGE));
    assert!(value.ends_with(&format!("; sig={}", verified.signature)), "{value}");
    let folded = delivered(&value.replace("; sender", ";\r\n sender"), MESSAGE);
    assert_eq!(verify(&signer.public(), &folded).unwrap().message, MESSAGE);
}

#[test]
fn each_change_after_signing_is_refused_with_its_reason() {
    let signer = Signer::from_seed(&seed()).unwrap();
    let other = Signer::from_seed(&B64.encode([8; 32])).unwrap();
    let value = signer.sign(&stamp(), MESSAGE);
    let refused = |public: &str, raw: Vec<u8>| verify(public, &raw).unwrap_err().to_string();
    let public = signer.public();
    let cases = [
        (
            &public,
            delivered(&value, b"From: Owen <me@example.org>\r\n\r\nyes, all\r\n"),
            "the signature does not verify",
        ),
        (
            &public,
            delivered(&value.replace("t=1800000000", "t=1800000001"), MESSAGE),
            "the signature does not verify",
        ),
        (
            &public,
            delivered(&value.replace("account=owen", "account=agent"), MESSAGE),
            "the signature does not verify",
        ),
        (&other.public(), delivered(&value, MESSAGE), "the signature does not verify"),
        (&public, [TRACE, MESSAGE].concat(), "0 Submission-Signature headers, not one"),
        (
            &public,
            [TRACE, format!("{HEADER}: x\r\n").as_bytes(), &delivered(&value, MESSAGE)].concat(),
            "2 Submission-Signature headers, not one",
        ),
        (
            &public,
            delivered(&value.replace("t=1800000000", "t=01800000000"), MESSAGE),
            "the header is not written as the signer writes it",
        ),
        (
            &public,
            delivered(&value.replace("v=1", "v=2"), MESSAGE),
            "the header's fields are not v=1, account, sender, t and sig",
        ),
        (&public, delivered(&value.replace("; sig=", "; s="), MESSAGE), "the header has no sig"),
        (
            &"".to_owned(),
            delivered(&value, MESSAGE),
            "no public key for submission signatures is built in",
        ),
    ];
    for (n, (public, raw, why)) in cases.into_iter().enumerate() {
        assert_eq!(refused(public, raw), why, "case {n}");
    }
}

#[test]
fn a_header_below_a_line_that_is_no_field_is_not_in_the_header_block() {
    let signer = Signer::from_seed(&seed()).unwrap();
    let raw =
        [b"Received: x\r\n\r\n".as_slice(), &delivered(&signer.sign(&stamp(), MESSAGE), MESSAGE)]
            .concat();
    assert_eq!(
        verify(&signer.public(), &raw).unwrap_err().to_string(),
        "0 Submission-Signature headers, not one"
    );
}

#[test]
fn a_login_that_could_end_a_field_is_no_account() {
    for login in ["owen", "owen@example.org", "o.w-e_n+1"] {
        assert_eq!(login.parse::<Account>().unwrap().to_string(), login);
    }
    for login in ["", "a; t=1", "a b", "a\r\nb", "ö"] {
        assert!(login.parse::<Account>().is_err(), "{login:?}");
    }
}
