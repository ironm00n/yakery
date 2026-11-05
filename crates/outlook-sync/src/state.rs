//! What the sync keeps between runs, one JSON file each in its state directory. Losing any of it
//! costs no events: the next full listing rewrites the mirror, and a lost refresh token means
//! signing in again.

use jiff::Timestamp;
use private::Secret;
use serde::{Deserialize, Serialize};

/// The sign-in `outlook-sync login` started, which the callback finishes once, leaving how it went
/// for the command to print.
#[derive(Serialize, Deserialize)]
pub enum Login {
    Pending { state: Secret, verifier: Secret, until: Timestamp },
    Done(Result<(), String>),
}

#[derive(Default, Serialize, Deserialize)]
pub struct Tokens {
    pub refresh: Option<Secret>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub client_state: Secret,
    pub expires: Timestamp,
}

/// Where the last delta run left off; its window is fixed in the link.
#[derive(Default, Serialize, Deserialize)]
pub struct Delta {
    pub link: Option<String>,
}

/// What the sync last wrote to each mirror file: the occurrence's Graph id and the ETag Stalwart
/// gave the write, so the owner's edits tell themselves apart from the sync's own; and the
/// sync-collection token the last look at the mirror ended on.
#[derive(Default, Serialize, Deserialize)]
pub struct Written {
    pub files: std::collections::BTreeMap<String, Copy>,
    pub token: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Copy {
    pub id: String,
    pub etag: String,
}

pub const WRITTEN: &str = "written.json";
pub const LOGIN: &str = "login.json";
pub const TOKENS: &str = "tokens.json";
pub const SUBSCRIPTION: &str = "subscription.json";
pub const DELTA: &str = "delta.json";
pub const KEY: &str = "key.pem";
pub const CERTIFICATE: &str = "certificate.pem";
