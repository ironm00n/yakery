use super::*;

#[test]
fn addresses_are_plain_and_compare_without_case() {
    for kept in
        ["a@b.dev", "o'neil.x@sub.example-1.com", "A@Example.ORG", "agent+tooling@example.org"]
    {
        assert_eq!(kept.parse::<Address>().unwrap().to_string(), kept);
    }
    for refused in [
        "evil@example.com>,<x@example.org",
        "\"a@b\"@example.org",
        "a@b@example.org",
        "a b@example.org",
        "<a@example.org>",
        "a@example.org\r\nBcc: x@y.z",
        "a,b@example.org",
        ".a@example.org",
        "a..b@example.org",
        "a@-example.org",
        "a@example",
        "a@example.org.",
        "a@[127.0.0.1]",
        "ä@example.org",
        "a@exämple.org",
        "@example.org",
        "a@",
    ] {
        assert!(refused.parse::<Address>().is_err(), "{refused}");
    }
    let spaced = "a b@example.org".parse::<Address>().unwrap_err();
    assert_eq!(spaced.to_string(), "a b@example.org is not a plain email address");
    let a: Address = "Agent@Example.org".parse().unwrap();
    assert_eq!(a, "agent@example.org".parse().unwrap());
    assert_eq!((a.local(), a.domain()), ("Agent", "Example.org"));
    let long = |n: usize| a.subaddress(&"t".repeat(n)).map(|a| a.to_string().len());
    assert_eq!((long(4).unwrap(), long(236).unwrap()), (22, 254));
    assert!(long(237).is_err() && a.subaddress(&"a b").is_err());
    let user = |s: &str| s.parse::<Address>().unwrap().user().to_owned();
    assert_eq!([user("agent+a+b@example.org"), user("Agent@example.org")], ["agent", "Agent"]);
}

#[test]
fn serde_goes_through_the_parser() {
    let a: Address = serde_json::from_str(r#""ada@agent.example.org""#).unwrap();
    assert_eq!(serde_json::to_string(&a).unwrap(), r#""ada@agent.example.org""#);
    let refused = serde_json::from_str::<Address>(r#""a b@example.org""#).unwrap_err();
    assert!(
        refused.to_string().starts_with("a b@example.org is not a plain email address"),
        "{refused}"
    );
    assert!(serde_json::from_str::<Address>("7").is_err());
}

#[test]
fn names_like_secrets_are_caught() {
    for name in [
        "Token",
        "ApiToken",
        "SECRET",
        "PasswordFile",
        "Passwd",
        "Passphrase",
        "Credentials",
        "Key",
        "Monkey",
    ] {
        assert!(secret_like(name), "{name}");
    }
    for name in ["Address", "Topic", "MessageId", "RunId", "Fingerprint", "Login"] {
        assert!(!secret_like(name), "{name}");
    }
}
