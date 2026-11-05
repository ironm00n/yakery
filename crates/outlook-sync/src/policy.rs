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
    /// The address of the account holding the calendar, whose changes Stalwart pushes.
    pub owner: String,
    pub days_back: i64,
    pub days_ahead: i64,
    /// Whether edits made in the mirror go back to Outlook, which needs Calendars.ReadWrite.
    pub write: bool,
}

#[cfg(not(test))]
const COMPILED: &str = env!("OUTLOOK_SYNC_POLICY");
#[cfg(test)]
const COMPILED: &str = include_str!("../tests/policy.json");

pub static POLICY: LazyLock<Policy> =
    LazyLock::new(|| serde_json::from_str(COMPILED).expect("the module's policy parses"));

/// Where Graph sends what it notices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Changes,
    Lifecycle,
}

/// The paths the sync serves under its public prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Notices(Feed),
    Callback,
}

impl Route {
    const ALL: [Route; 3] =
        [Route::Notices(Feed::Changes), Route::Notices(Feed::Lifecycle), Route::Callback];

    pub fn path(self) -> &'static str {
        match self {
            Route::Notices(Feed::Changes) => "/notify",
            Route::Notices(Feed::Lifecycle) => "/lifecycle",
            Route::Callback => "/callback",
        }
    }

    pub fn of(path: &str) -> Option<Route> {
        Route::ALL.into_iter().find(|r| r.path() == path)
    }
}

impl Policy {
    pub fn url(&self, route: Route) -> String {
        format!("{}{}", self.public, route.path())
    }

    /// The path part of the public prefix, which requests arrive under.
    pub fn prefix(&self) -> &str {
        let rest = self.public.split_once("://").map_or(self.public.as_str(), |(_, r)| r);
        rest.find('/').map_or("", |at| &rest[at..])
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

#[cfg(test)]
#[path = "../tests/policy.rs"]
mod tests;
