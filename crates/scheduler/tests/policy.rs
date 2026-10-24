use super::*;

/// The bundle's defaults, for an owner in New York with two counted calendars.
pub fn fixture() -> Policy {
    serde_json::from_value(serde_json::json!({
        "owner": "owen@example.test",
        "host": "schedule.example.test",
        "stalwart": "127.0.0.1:8080",
        "zone": "America/New_York",
        "hours": ["mon 10:00-18:00", "tue 10:00-18:00", "wed 10:00-18:00", "thu 10:00-18:00", "fri 10:00-18:00"],
        "notice": "PT4H",
        "horizonWeeks": 4,
        "gap": "PT5M",
        "calendars": [{"key": "work", "label": "Work"}, {"key": "personal", "label": "Personal"}],
        "showCalendars": true,
    }))
    .unwrap()
}

pub fn leaked(policy: Policy) -> &'static Policy {
    Box::leak(Box::new(policy))
}

#[test]
fn the_compiled_policy_parses() {
    LazyLock::force(&POLICY);
}

#[test]
fn gaps_under_five_minutes_are_refused() {
    assert!("PT5M".parse::<Gap>().is_ok());
    assert!("PT4M".parse::<Gap>().is_err());
    let policy = serde_json::to_value("PT4M").unwrap();
    assert!(serde_json::from_value::<Gap>(policy).is_err());
}

#[test]
fn windows_read_and_print_alike() {
    for text in ["mon 10:00-18:00", "sat 13:00-17:30"] {
        assert_eq!(text.parse::<Window>().unwrap().to_string(), text);
    }
    for bad in ["monday 10:00-18:00", "mon 18:00-10:00", "mon 10:00", "mon 25:00-26:00"] {
        assert!(bad.parse::<Window>().is_err(), "{bad}");
    }
}
