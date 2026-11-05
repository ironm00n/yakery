//! What the sync is set up with, compiled in from its Nix module.

use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    /// The Entra app's client id.
    pub client_id: String,
    /// The Entra authority's tenant segment: `organizations` for any work or school account.
    pub tenant: String,
    /// The public HTTPS prefix nginx forwards to the sync, without a trailing slash.
    pub public: String,
    /// Stalwart's loopback origin.
    pub stalwart: String,
    /// The account the sync writes as, and the path of the calendar it mirrors into.
    pub account: String,
    pub calendar: String,
    pub days_back: i64,
    pub days_ahead: i64,
}

#[cfg(not(test))]
const COMPILED: &str = env!("OUTLOOK_SYNC_POLICY");
#[cfg(test)]
const COMPILED: &str = include_str!("../tests/policy.json");

pub static POLICY: LazyLock<Policy> =
    LazyLock::new(|| serde_json::from_str(COMPILED).expect("the module's policy parses"));

impl Policy {
    pub fn redirect(&self) -> String {
        format!("{}/callback", self.public)
    }

    pub fn authority(&self, path: &str) -> String {
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0/{path}", self.tenant)
    }

    /// The host part of the public prefix, which names the mirrored events' UIDs.
    pub fn host(&self) -> &str {
        let rest = self.public.split_once("://").map_or(self.public.as_str(), |(_, r)| r);
        rest.split('/').next().unwrap_or(rest)
    }
}
