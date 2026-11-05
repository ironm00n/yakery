//! What the sync is set up with, compiled in from its Nix module.

use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub client_id: String,
    pub tenant: String,
    pub public: Public,
    pub stalwart: String,
    pub account: String,
    pub calendar: String,
    pub owner: String,
    pub days_back: i64,
    pub days_ahead: i64,
    pub write: bool,
}

#[cfg(not(test))]
const COMPILED: &str = env!("OUTLOOK_SYNC_POLICY");
#[cfg(test)]
const COMPILED: &str = include_str!("../tests/policy.json");

pub static POLICY: LazyLock<Policy> =
    LazyLock::new(|| serde_json::from_str(COMPILED).expect("the module's policy parses"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Changes,
    Lifecycle,
}

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

/// The HTTPS prefix nginx forwards to the sync: `https://<host>/<path>`, with no trailing slash.
#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct Public {
    host: String,
    prefix: String,
}

impl TryFrom<String> for Public {
    type Error = String;

    fn try_from(url: String) -> Result<Public, String> {
        let shape = || format!("{url:?} is not https://<host>/<path> with no trailing slash");
        let (host, path) =
            url.strip_prefix("https://").and_then(|r| r.split_once('/')).ok_or_else(shape)?;
        if host.is_empty() || path.is_empty() || path.ends_with('/') {
            return Err(shape());
        }
        Ok(Public { host: host.into(), prefix: format!("/{path}") })
    }
}

impl Public {
    pub fn url(&self, route: Route) -> String {
        format!("https://{}{}{}", self.host, self.prefix, route.path())
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

impl Policy {
    pub fn authority(&self, path: &str) -> String {
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0/{path}", self.tenant)
    }
}

#[cfg(test)]
#[path = "../tests/policy.rs"]
mod tests;
