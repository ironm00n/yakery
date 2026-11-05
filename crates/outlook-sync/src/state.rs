use jiff::Timestamp;
use private::Secret;
use serde::{Deserialize, Serialize};

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

/// The ETag of the sync's own last write to each file tells the owner's edits apart.
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
