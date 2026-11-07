use super::*;

#[test]
fn wakes_listed_agents_on_the_agent_domain_once() {
    let dispatch = Dispatch {
        agents: ["alpha", "beta"].map(String::from).into(),
        domain: "agent.ironmoon.dev".into(),
        cid: 4,
        key: [0; 32],
    };
    let batch: Batch = serde_json::from_value(serde_json::json!({ "events": [
        { "type": "delivery.completed", "data": { "to": [
            "alpha@agent.ironmoon.dev", "beta@agent.ironmoon.dev",
            "gamma@agent.ironmoon.dev", "alpha@ironmoon.dev", "ironmoon@ironmoon.dev",
        ] } },
        { "type": "delivery.completed", "data": { "to": "alpha@AGENT.ironmoon.dev" } },
        { "type": "message-ingest.ham", "data": { "to": ["beta@agent.ironmoon.dev"] } },
    ] }))
    .unwrap();
    assert_eq!(
        dispatch.recipients(&batch),
        BTreeSet::from(["alpha", "beta"].map(String::from))
    );
}
