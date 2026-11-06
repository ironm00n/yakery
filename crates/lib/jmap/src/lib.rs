//! JMAP's wire format (RFC 8620, and the parts of RFC 8621 its callers use): the requests a server
//! takes and what its answers say. Nothing here sends a request: a caller sends each through its
//! own HTTP client.

extern crate self as jmap;

mod answers;
mod events;
pub mod mail;
mod request;
mod session;
mod view;

use anyhow::{Result, ensure};

pub use self::answers::{Changed, Got, Queried, Set};
pub use self::events::{Event, Events, StateChange};
pub use self::request::{Call, Failed, MethodError, Request, Response};
pub use self::session::{Account, Limits, Session, Urls};
pub use self::view::{View, nullable};

macro_rules! capabilities {
    ($prefix:literal { $($name:ident = $urn:literal),* $(,)? }) => {
        const PREFIX: &str = $prefix;
        $(pub const $name: &str = concat!($prefix, $urn);)*
    };
}

capabilities! {
    "urn:ietf:params:jmap:" {
        CORE = "core", MAIL = "mail", SUBMISSION = "submission",
        CALENDARS = "calendars", PRINCIPALS = "principals",
    }
}

/// What a method is called under: the capabilities a request with it declares, and the one whose
/// primary account it acts on (RFC 8620 2, 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap {
    pub using: &'static [&'static str],
    pub account: &'static str,
}

/// `id` if it is a JMAP Id (RFC 8620 1.2).
pub fn id(id: &str) -> Result<&str> {
    let valid = (1..=255).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ensure!(valid, "{} is not a JMAP id", id.escape_debug());
    Ok(id)
}

/// A session's URL template (RFC 8620 2: RFC 6570's level 1) with each `{name}` given its value,
/// percent-encoded, so no value reaches past its own place in the URL.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let encoded = |value: &str| -> String {
        let unreserved = |b: u8| b.is_ascii_alphanumeric() || b"-._~".contains(&b);
        let byte =
            |b: u8| if unreserved(b) { char::from(b).to_string() } else { format!("%{b:02X}") };
        value.bytes().map(byte).collect()
    };
    let fill = |url: String, (name, value): &(&str, &str)| {
        url.replace(&format!("{{{name}}}"), &encoded(value))
    };
    values.iter().fold(template.to_owned(), fill)
}

#[cfg(test)]
#[path = "../tests/lib.rs"]
mod tests;
