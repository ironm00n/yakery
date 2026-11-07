use serde_json::{Value, json};

use super::*;

fn woken(attempts: &[Value]) -> Vec<String> {
    let dispatch = Dispatch {
        agents: ["alpha", "beta"].map(String::from).into(),
        domain: "agent.ironmoon.dev".into(),
        cid: 4,
        key: [0; 32],
    };
    let attempt = |to: &Value| json!({ "type": "delivery.attempt-end", "data": { "to": to } });
    let events: Vec<Value> = attempts.iter().map(attempt).collect();
    let batch: Batch = serde_json::from_value(json!({ "events": events })).unwrap();
    dispatch.recipients(&batch).into_iter().collect()
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
