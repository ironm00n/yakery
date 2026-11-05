//! The keys a page's links and forms carry, and what the front reads back from them.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;

use crate::link::Form;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tz,
    Week,
    Slot,
    Name,
    Address,
    Note,
}

impl Key {
    pub fn name(self) -> &'static str {
        match self {
            Key::Tz => "tz",
            Key::Week => "week",
            Key::Slot => "slot",
            Key::Name => "name",
            Key::Address => "address",
            Key::Note => "note",
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A slot's start, as Unix seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot(pub Timestamp);

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.as_second())
    }
}

impl FromStr for Slot {
    type Err = String;

    fn from_str(s: &str) -> Result<Slot, String> {
        let seconds = s.parse().map_err(|_| format!("{s:?} is not a slot"))?;
        Timestamp::from_second(seconds).map(Slot).map_err(|e| e.to_string())
    }
}

struct Pairs(BTreeMap<String, String>);

impl Pairs {
    fn of(bytes: &[u8]) -> Pairs {
        Pairs(form_urlencoded::parse(bytes).into_owned().collect())
    }

    fn get(&self, key: Key) -> Option<&str> {
        self.0.get(key.name()).map(String::as_str)
    }

    fn parsed<T: FromStr>(&self, key: Key) -> Option<T> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    fn text(&self, key: Key) -> String {
        self.get(key).unwrap_or_default().to_string()
    }
}

/// What a page's URL asks for; a value that doesn't parse reads as absent.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub tz: Option<String>,
    pub week: u8,
    pub slot: Option<Timestamp>,
}

impl Query {
    pub fn parse(bytes: &[u8]) -> Query {
        let pairs = Pairs::of(bytes);
        Query {
            tz: pairs.get(Key::Tz).map(str::to_string),
            week: pairs.parsed(Key::Week).unwrap_or(0),
            slot: pairs.parsed(Key::Slot).map(|Slot(t)| t),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Posted {
    pub tz: Option<String>,
    pub slot: Option<Timestamp>,
    pub form: Form,
}

impl Posted {
    pub fn parse(bytes: &[u8]) -> Posted {
        let pairs = Pairs::of(bytes);
        Posted {
            tz: pairs.get(Key::Tz).map(str::to_string),
            slot: pairs.parsed(Key::Slot).map(|Slot(t)| t),
            form: Form {
                name: pairs.text(Key::Name),
                address: pairs.text(Key::Address),
                note: pairs.text(Key::Note),
            },
        }
    }
}

#[cfg(test)]
#[path = "../tests/params.rs"]
mod tests;
