use super::*;
use crate::{CALENDARS, CORE, MAIL};

const NOTES: Cap = Cap { using: &[CORE, MAIL], account: MAIL };
const EVENTS: Cap = Cap { using: &[CALENDARS], account: CALENDARS };

crate::view! {
    under NOTES {
        Note in "Note/get" { id: String = "id", tags: Vec<String> = "tags" }
        Long in "Email/get" with ("fetchTextBodyValues", 64) { id: String = "id" }
    }
}

crate::view! {
    under self::EVENTS {
        Event in "CalendarEvent/get" (default) { id: String = "id", title: String = "title" }
    }
}

crate::object! {
    Part { kind: String = "type", size: usize = "size" }
}

#[test]
fn a_view_fetches_exactly_its_properties() {
    assert_eq!((Note::GET, Note::PROPERTIES), ("Note/get", &["id", "tags"][..]));
    assert_eq!((Note::CAP.using, Long::CAP.account), (&[CORE, MAIL][..], MAIL));
    assert_eq!(Note::args(json!(["n1"])), json!({ "ids": ["n1"], "properties": ["id", "tags"] }));
    let bodies = json!({
        "ids": null, "properties": ["id"], "fetchTextBodyValues": true, "maxBodyValueBytes": 64,
    });
    assert_eq!(Long::args(Value::Null), bodies);
    assert_eq!(serde_json::from_str::<Long>(r#"{"id":"m1"}"#).unwrap().id, "m1");
}

#[test]
fn a_view_needs_each_property_but_reads_null_as_empty() {
    let note: Note = serde_json::from_str(r#"{"id":"n1","tags":null}"#).unwrap();
    assert_eq!((note.id.as_str(), note.tags), ("n1", vec![]));
    let missing = serde_json::from_str::<Note>(r#"{"id":"n1"}"#).unwrap_err();
    assert_eq!(missing.to_string(), "missing field `tags` at line 1 column 11");
}

#[test]
fn a_default_view_reads_a_missing_property_as_its_default() {
    let event: Event = serde_json::from_str(r#"{"id":"e1"}"#).unwrap();
    assert_eq!((event.id.as_str(), event.title.as_str()), ("e1", ""));
    assert_eq!(Event::CAP.using, [CALENDARS]);
}

#[test]
fn an_object_reads_a_missing_or_null_property_as_empty() {
    let part: Part = serde_json::from_str(r#"{"type":null,"size":3}"#).unwrap();
    assert_eq!((part.kind.as_str(), part.size), ("", 3));
    let part: Part = serde_json::from_str("{}").unwrap();
    assert_eq!((part.kind.as_str(), part.size), ("", 0));
}
