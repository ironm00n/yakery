mod auth;
mod authority;
mod claims;
mod files;
mod text;
mod wire;

pub use wire::wire;

use ::text::Address;

use crate::{policy::Login, prelude::*};

/// `rows!(f; a, b => x; c => y)`: `f(a)` and `f(b)` must give `x`, and `f(c)` must give `y`.
macro_rules! rows {
    ($f:expr; $($($input:expr),+ => $want:expr);+ $(;)?) => {{
        let f = $f;
        $($({
            let input = $input;
            let row = format!("{input:?}");
            assert_eq!(f(input), $want, "row {row}");
        })+)+
    }};
}
pub(crate) use rows;

pub const HIM: &str = "owner@example.org";
pub const POOL: &str = "agent+pool@example.org";

pub fn shown<T: std::fmt::Display>(r: Result<T>) -> String {
    r.map_or_else(|e| format!("error: {e:#}"), |v| v.to_string())
}

pub fn parsed<T: std::str::FromStr<Err = anyhow::Error> + std::fmt::Display>(s: &str) -> String {
    s.parse::<T>().map_or_else(|_| "refused".into(), |v| v.to_string())
}

pub fn login(s: &str) -> Login {
    s.parse().unwrap()
}

pub fn address(s: &str) -> Address {
    s.parse().unwrap()
}

pub fn addresses(list: &str) -> Vec<Address> {
    list.split(',').filter(|a| !a.is_empty()).map(address).collect()
}

/// An email as Stalwart sends one, from him to the pool topic, with every property a view reads; `with` replaces some.
pub fn email(id: &str, with: Value) -> Value {
    let mut m = json!({
        "id": id, "threadId": "T", "receivedAt": "2026-10-01T00:00:00Z", "subject": "s",
        "from": [{ "email": HIM }], "to": [{ "email": POOL }], "cc": [], "keywords": {},
        "mailboxIds": { "inbox": true }, "messageId": [format!("{id}@example.org")],
        "inReplyTo": [], "references": [], "preview": "", "attachments": [],
        "header:Content-Type": " text/plain", "header:Original-Recipient:asText": null,
        "header:Disposition-Notification-To:asAddresses": null, "textBody": [], "bodyValues": {}, "blobId": "B",
        "bodyStructure": {},
    });
    m.as_object_mut().unwrap().extend(with.as_object().unwrap().clone());
    m
}

pub fn view<V: DeserializeOwned>(value: Value) -> V {
    serde_json::from_value(value).unwrap()
}

pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("agent-mail-{}-{n}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn file(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.0.join(name)).ok()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
