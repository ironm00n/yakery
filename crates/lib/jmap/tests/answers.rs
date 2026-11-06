use serde_json::json;

use super::*;

fn read<T: serde::de::DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_get_reads_null_or_missing_as_empty() {
    let got: Got<String> = read(r#"{"state":"s1","list":["a"],"notFound":["b"]}"#);
    assert_eq!(
        (got.state.as_str(), got.list, got.not_found),
        ("s1", vec!["a".into()], vec!["b".into()])
    );
    let got: Got<String> = read(r#"{"list":null}"#);
    assert_eq!((got.state.as_str(), got.list.len(), got.not_found.len()), ("", 0, 0));
}

#[test]
fn a_set_keeps_what_it_did_and_refused_as_sent() {
    let set: Set = read(
        r#"{"oldState":null,"newState":"s2","created":{"k":{"id":"m1"}},
            "updated":{"m2":null},"notUpdated":{"m3":{"type":"notFound"}}}"#,
    );
    assert_eq!((set.old_state.as_str(), set.new_state.as_str()), ("", "s2"));
    assert_eq!(set.created["k"]["id"], "m1");
    assert_eq!(set.updated.get("m2"), Some(&Value::Null));
    assert_eq!(
        json!([set.not_updated, set.not_created, set.not_destroyed]),
        json!([{ "m3": { "type": "notFound" } }, {}, {}])
    );
}

#[test]
fn changes_and_queries_read_null_or_missing_as_empty() {
    let changed: Changed =
        read(r#"{"newState":"s2","hasMoreChanges":true,"created":["a"],"updated":null}"#);
    assert_eq!((changed.new_state.as_str(), changed.more), ("s2", true));
    assert_eq!((changed.created, changed.updated), (vec!["a".to_owned()], vec![]));
    let queried: Queried = read(r#"{"ids":null}"#);
    assert_eq!((queried.ids.len(), queried.limit), (0, None));
    assert_eq!(read::<Queried>(r#"{"ids":["a"],"limit":50}"#).limit, Some(50));
}
