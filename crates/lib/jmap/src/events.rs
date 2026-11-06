use std::collections::HashMap;

use serde::Deserialize;

/// An event source's stream (RFC 8620 7.3: server-sent events) as its bytes arrive.
#[derive(Default)]
pub struct Events(Vec<u8>);

/// What one event says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Ping,
    Changed(StateChange),
    /// Data that is neither a ping nor a state change.
    Other,
}

/// The new state of each type of data that changed, by account (RFC 8620 7.1).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StateChange {
    pub changed: HashMap<String, HashMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(tag = "@type")]
enum Data {
    StateChange(StateChange),
}

impl Events {
    /// The text of each event `chunk` completes, carriage returns dropped.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.0.extend(chunk.iter().filter(|&&b| b != b'\r'));
        let next = || {
            let end = self.0.windows(2).position(|w| w == b"\n\n")?;
            let event: Vec<u8> = self.0.drain(..end + 2).collect();
            Some(String::from_utf8_lossy(&event[..end]).into_owned())
        };
        std::iter::from_fn(next).collect()
    }
}

impl Event {
    /// The event `text` is, or `None` for one without data, which is never dispatched.
    pub fn read(text: &str) -> Option<Event> {
        let fields =
            || text.lines().map(|l| l.split_once(':').map_or((l, ""), |(f, v)| (f, v.trim())));
        let values = |name: &'static str| fields().filter(move |(f, _)| *f == name).map(|f| f.1);
        let (data, ping) =
            (values("data").collect::<Vec<_>>(), values("event").next_back() == Some("ping"));
        (!data.is_empty()).then(|| match serde_json::from_str(&data.join("\n")) {
            _ if ping => Event::Ping,
            Ok(Data::StateChange(change)) => Event::Changed(change),
            Err(_) => Event::Other,
        })
    }
}

#[cfg(test)]
#[path = "../tests/events.rs"]
mod tests;
