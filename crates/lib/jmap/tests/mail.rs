use super::*;

#[test]
fn a_delivery_is_refused_only_when_every_recipient_is() {
    let of = |statuses: Option<&str>| {
        let statuses =
            statuses.map(|s| serde_json::from_str::<HashMap<String, Status>>(s).unwrap());
        Delivery::of(statuses.as_ref())
    };
    let refused = |a: &str, r: &str| (a.to_owned(), r.to_owned());
    let rows = [
        (
            Some(r#"{"nosuch@example.org":{"delivered":"no","smtpReply":"550 no "}}"#),
            Delivery::Refused(vec![refused("nosuch@example.org", "550 no")]),
        ),
        (
            Some(
                r#"{"b@x":{"delivered":"no","smtpReply":"550 b"},"a@x":{"delivered":"no","smtpReply":"550 a"}}"#,
            ),
            Delivery::Refused(vec![refused("a@x", "550 a"), refused("b@x", "550 b")]),
        ),
        (
            Some(
                r#"{"a@x":{"delivered":"queued","smtpReply":"250 Queued"},"b@x":{"delivered":"no","smtpReply":"550 no"}}"#,
            ),
            Delivery::Accepted(vec![refused("b@x", "550 no")]),
        ),
        (Some(r#"{"a@x":{"delivered":null}}"#), Delivery::Accepted(vec![])),
        (Some("{}"), Delivery::Accepted(vec![])),
        (None, Delivery::Unknown),
    ];
    for (statuses, delivery) in rows {
        assert_eq!(of(statuses), delivery, "{statuses:?}");
    }
}
