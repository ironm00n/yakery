use serde::Deserialize;
use serde_json::{Map, Value};

/// A /get answer (RFC 8620 5.1).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(bound = "T: Deserialize<'de>")]
pub struct Got<T> {
    #[serde(default, deserialize_with = "crate::nullable")]
    pub state: String,
    #[serde(default, deserialize_with = "crate::nullable")]
    pub list: Vec<T>,
    #[serde(default, rename = "notFound", deserialize_with = "crate::nullable")]
    pub not_found: Vec<String>,
}

crate::object! {
    /// A /query answer (RFC 8620 5.5), with the limit the server used when it set its own.
    Queried { ids: Vec<String> = "ids", limit: Option<u64> = "limit" }
    /// A /set answer (RFC 8620 5.3), whose refusals are kept as sent.
    Set {
        old_state: String = "oldState", new_state: String = "newState",
        created: Map<String, Value> = "created", not_created: Map<String, Value> = "notCreated",
        updated: Map<String, Value> = "updated", not_updated: Map<String, Value> = "notUpdated",
        not_destroyed: Map<String, Value> = "notDestroyed",
    }
    /// A /changes answer (RFC 8620 5.2).
    Changed {
        new_state: String = "newState", more: bool = "hasMoreChanges",
        created: Vec<String> = "created", updated: Vec<String> = "updated",
    }
}

#[cfg(test)]
#[path = "../tests/answers.rs"]
mod tests;
