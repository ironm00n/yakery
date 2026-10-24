use super::*;

fn token(byte: char) -> Token {
    Token::parse(&byte.to_string().repeat(64)).unwrap()
}

pub fn issue(from: &str, until: &str) -> Issue {
    Issue {
        from: from.parse().unwrap(),
        until: until.parse().unwrap(),
        duration: SignedDuration::from_mins(30),
        gap: Gap::FLOOR,
        hours: vec![],
        title: None,
        location: None,
    }
}

fn booking() -> Booking {
    let form = Form {
        name: "Ada Lovelace".into(),
        address: "ada@example.org".into(),
        note: String::new(),
    };
    Booking { start: "2026-10-26T15:00:00Z".parse().unwrap(), booker: Checked::new(form).unwrap() }
}

#[test]
fn tokens_are_64_hex_digits_kept_lowercase() {
    assert_eq!(token('a').expose(), "a".repeat(64));
    assert!(Token::parse(&"A".repeat(64)).unwrap().digest() == token('a').digest());
    for bad in ["a".repeat(63), "a".repeat(65), "g".repeat(64), format!("{}é", "a".repeat(62))] {
        assert!(Token::parse(&bad).is_err(), "{bad}");
    }
    let digest = token('a').digest();
    assert_eq!(digest.id().to_string(), digest.to_string()[..8]);
    assert!(token('a').digest() != token('b').digest());
}

#[test]
fn limits_bound_window_duration_and_gap() {
    assert!(Checked::new(issue("2026-10-26", "2026-12-21")).is_ok());
    assert!(Checked::new(issue("2026-10-26", "2026-12-22")).is_err());
    assert!(Checked::new(issue("2026-10-26", "2026-10-25")).is_err());
    assert!("PT4M".parse::<Gap>().is_err() && "PT5M".parse::<Gap>().is_ok());
    let mut forged = serde_json::to_value(issue("2026-10-26", "2026-10-26")).unwrap();
    forged["gap"] = "PT4M".into();
    assert!(serde_json::from_value::<Limits>(forged).is_err(), "the core checks what `link` sends");
    assert!(
        Checked::new(Issue {
            duration: SignedDuration::from_mins(10),
            ..issue("2026-10-26", "2026-10-26")
        })
        .is_err()
    );
}

#[test]
fn bookers_are_checked_field_by_field() {
    let form = |name: &str, address: &str, note: &str| Form {
        name: name.into(),
        address: address.into(),
        note: note.into(),
    };
    assert!(Checked::new(form("Ada", "ada@example.org", "hi\nthere")).is_ok());
    assert_eq!(Checked::new(form(" ", "ada@example.org", "")).unwrap_err(), Field::Name);
    assert_eq!(Checked::new(form("A\u{7}da", "ada@example.org", "")).unwrap_err(), Field::Name);
    assert_eq!(
        Checked::new(form("Ada", "ada@example.org>, x@y.z", "")).unwrap_err(),
        Field::Address
    );
    assert_eq!(
        Checked::new(form("Ada", "ada@example.org", &"n".repeat(2001))).unwrap_err(),
        Field::Note
    );
    let forged = serde_json::json!({"name": "", "address": "ada@example.org", "note": ""});
    assert!(
        serde_json::from_value::<Booker>(forged).is_err(),
        "the core checks what the front sends"
    );
}

#[test]
fn a_link_books_once() {
    let d = token('a').digest();
    let links = Links::default()
        .issue(d, Checked::new(issue("2026-10-26", "2026-10-30")).unwrap())
        .unwrap();
    let claimed = links.claim(d, booking()).unwrap();
    assert!(claimed.claim(d, booking()).is_err(), "claiming is not open");
    assert!(claimed.claiming() == [d]);
    assert_eq!(claimed.taken().len(), 1);
    let booked = claimed.settle(d, Created::Event("e".into()));
    assert!(matches!(booked.get(&d).unwrap().state, State::Booked { event: Some(_), .. }));
    assert!(booked.claim(d, booking()).is_err(), "booked is not open");
    assert!(booked.claiming().is_empty());
}

#[test]
fn only_stalwarts_refusal_reopens_a_link() {
    let d = token('a').digest();
    let claimed = Links::default()
        .issue(d, Checked::new(issue("2026-10-26", "2026-10-30")).unwrap())
        .unwrap()
        .claim(d, booking())
        .unwrap();
    assert!(matches!(
        claimed.settle(d, Created::Refused("no".into())).get(&d).unwrap().state,
        State::Open
    ));
    assert!(matches!(
        claimed.settle(d, Created::UidTaken).get(&d).unwrap().state,
        State::Booked { event: None, .. }
    ));
}

#[test]
fn revoking_and_expiring() {
    let (a, b) = (token('a').digest(), token('b').digest());
    let limits = || -> Limits { Checked::new(issue("2026-10-26", "2026-10-30")).unwrap() };
    let links = Links::default()
        .issue(a, limits())
        .unwrap()
        .issue(b, limits())
        .unwrap()
        .claim(b, booking())
        .unwrap();
    assert!(links.issue(a, limits()).is_err());
    assert!(links.revoke(a.id()).unwrap().get(&a).is_none());
    assert!(links.revoke(b.id()).is_err(), "a claiming link stays");
    assert_eq!(links.expire("2026-11-29".parse().unwrap()).iter().count(), 2);
    assert_eq!(links.expire("2026-11-30".parse().unwrap()).iter().count(), 0);
}
