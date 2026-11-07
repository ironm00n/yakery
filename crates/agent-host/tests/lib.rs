use super::*;

fn went_well(lines: &[&str]) -> bool {
    let mut outcome = Outcome::default();
    lines.iter().for_each(|line| outcome.read(&serde_json::from_str(line).unwrap()));
    outcome.well
}

#[test]
fn a_wake_goes_well_when_its_message_is_taken_or_its_session_exits_0() {
    assert!(went_well(&[r#""injected""#]));
    assert!(went_well(&[r#""started""#, r#"{"event":{}}"#, r#"{"exited":{"code":0}}"#]));
}

#[test]
fn a_wake_goes_badly_with_no_reply_a_refusal_a_cut_off_session_or_any_other_exit() {
    for lines in [
        &[][..],
        &[r#"{"refused":{"reason":"no agent named gamma"}}"#],
        &[r#""started""#, r#"{"event":{}}"#],
        &[r#""started""#, r#"{"exited":{"code":null}}"#],
        &[r#""started""#, r#"{"exited":{"code":2}}"#],
    ] {
        assert!(!went_well(lines), "{lines:?}");
    }
}
