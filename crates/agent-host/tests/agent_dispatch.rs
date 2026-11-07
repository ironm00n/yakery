use std::path::Path;

use serde_json::{Value, json};

use super::*;

fn dispatch(state: &Path) -> Dispatch {
    let names = ["alpha", "beta"].map(String::from);
    Dispatch::new(names, "agent.ironmoon.dev", 4, [0; 32], state).unwrap()
}

fn batch(attempts: &[Value]) -> Batch {
    let attempt = |to: &Value| json!({ "type": "delivery.attempt-end", "data": { "to": to } });
    let events: Vec<Value> = attempts.iter().map(attempt).collect();
    serde_json::from_value(json!({ "events": events })).unwrap()
}

fn woken(attempts: &[Value]) -> Vec<String> {
    let state = tempfile::tempdir().unwrap();
    dispatch(state.path()).recipients(&batch(attempts)).into_iter().collect()
}

#[test]
fn an_attempt_wakes_the_listed_agents_it_tried() {
    assert_eq!(woken(&[json!(["alpha@agent.ironmoon.dev", "bob@gmail.com"])]), ["alpha"]);
}

#[test]
fn an_attempt_wakes_no_one_off_the_list_or_off_the_agent_domain() {
    assert!(woken(&[json!(["gamma@agent.ironmoon.dev", "alpha@ironmoon.dev"])]).is_empty());
}

#[test]
fn a_batch_wakes_each_agent_once() {
    let attempts = [
        json!(["alpha@agent.ironmoon.dev"]),
        json!(["alpha@agent.ironmoon.dev", "beta@agent.ironmoon.dev"]),
    ];
    assert_eq!(woken(&attempts), ["alpha", "beta"]);
}

#[test]
fn a_to_of_another_shape_wakes_its_string_and_rejects_nothing() {
    assert_eq!(
        woken(&[
            json!("alpha@agent.ironmoon.dev"),
            json!(7),
            json!([7, "beta@agent.ironmoon.dev"])
        ]),
        ["alpha", "beta"]
    );
}

#[test]
fn an_agent_is_woken_whatever_the_case_of_its_address() {
    assert_eq!(woken(&[json!(["Alpha@AGENT.ironmoon.dev"])]), ["alpha"]);
}

#[test]
fn a_refused_wake_is_not_taken() {
    assert!(
        taken(r#"{"refused":{"reason":"the session has stopped reading its input"}}"#).is_err()
    );
    assert!(matches!(taken(r#""injected""#), Ok(Reply::Injected)));
}

/// The agents whose wakes the mail to `agents` starts.
fn mail(dispatching: &Dispatch, agents: &[&str]) -> Vec<String> {
    let to: Vec<String> =
        agents.iter().map(|agent| format!("{agent}@agent.ironmoon.dev")).collect();
    dispatching.take(&batch(&[json!(to)]))
}

#[test]
fn an_agent_owed_a_wake_before_a_restart_is_woken_after_it() {
    let state = tempfile::tempdir().unwrap();
    mail(&dispatch(state.path()), &["alpha", "beta"]);
    assert_eq!(dispatch(state.path()).owed.agents(), ["alpha", "beta"]);
}

#[test]
fn only_the_first_mail_of_an_agent_owed_nothing_starts_a_wake() {
    let state = tempfile::tempdir().unwrap();
    let dispatching = dispatch(state.path());
    let started = [&["alpha"][..], &["alpha", "beta"], &["beta"]].map(|to| mail(&dispatching, to));
    assert_eq!(started, [vec!["alpha"], vec!["beta"], vec![]]);
}

/// The first wake takes a mail as it runs.
#[tokio::test]
async fn mail_during_a_wake_is_woken_for_once_it_ends_and_then_the_agent_is_settled() {
    let state = tempfile::tempdir().unwrap();
    let dispatching = dispatch(state.path());
    mail(&dispatching, &["alpha"]);
    let mut wakes = 0;
    let wake = || {
        wakes += 1;
        match wakes {
            1 => assert!(mail(&dispatching, &["alpha"]).is_empty()),
            2 => {}
            _ => panic!("woken after it was settled"),
        }
        async {}
    };
    dispatching.owed.until_settled("alpha", wake).await;
    assert_eq!(wakes, 2);
    assert!(dispatch(state.path()).owed.agents().is_empty());
}

/// A directory in the file's place fails every save until it is removed.
#[test]
fn an_agent_owed_a_wake_that_cannot_be_saved_is_saved_with_its_next_mail() {
    let state = tempfile::tempdir().unwrap();
    let dispatching = dispatch(state.path());
    let in_the_way = state.path().join("owed.json");
    std::fs::create_dir_all(in_the_way.join("full")).unwrap();
    assert_eq!(mail(&dispatching, &["alpha"]), ["alpha"]);
    std::fs::remove_dir_all(&in_the_way).unwrap();
    mail(&dispatching, &["alpha"]);
    assert_eq!(dispatch(state.path()).owed.agents(), ["alpha"]);
}

#[test]
fn an_owed_agent_no_longer_listed_is_not_woken() {
    let state = tempfile::tempdir().unwrap();
    std::fs::write(state.path().join("owed.json"), r#"["alpha","gamma"]"#).unwrap();
    assert_eq!(dispatch(state.path()).owed.agents(), ["alpha"]);
}
