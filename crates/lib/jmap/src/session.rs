use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::CORE;

crate::object!(@struct
    /// A session (RFC 8620 2). Its URLs are read through [`Session::checked`] alone.
    #[serde(default)]
    Session {
        pub username: String = "username",
        pub accounts: BTreeMap<String, Account> = "accounts",
        primaries: HashMap<String, String> = "primaryAccounts",
        capabilities: HashMap<String, Value> = "capabilities",
        api: String = "apiUrl", upload: String = "uploadUrl", download: String = "downloadUrl",
        events: String = "eventSourceUrl",
    }
);

crate::object! {
    Account { name: String = "name" }
}

/// A session's URLs, each taken by the check [`Session::checked`] was given; the upload, download
/// and event-source URLs are templates for [`crate::fill`].
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Urls {
    pub api: String,
    pub upload: String,
    pub download: String,
    pub events: String,
}

/// Core's request limits (RFC 8620 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub get: usize,
    pub set: usize,
    pub calls: usize,
}

impl Session {
    /// Its URLs, once `on` takes each of them; the first it refuses is the error.
    pub fn checked<E>(&self, on: impl Fn(&str) -> Result<(), E>) -> Result<Urls, E> {
        let urls = [&self.upload, &self.download, &self.events, &self.api];
        urls.iter().try_for_each(|u| on(u))?;
        let [upload, download, events, api] = urls.map(String::clone);
        Ok(Urls { api, upload, download, events })
    }

    /// The id of the account it uses by default for `capability`'s data.
    pub fn primary(&self, capability: &str) -> Result<&str> {
        let name = capability.strip_prefix(crate::PREFIX).unwrap_or(capability);
        let account = self.primaries.get(capability);
        crate::id(account.with_context(|| format!("the JMAP session names no {name} account"))?)
    }

    /// The id of the account named `name`, the first by id if several are.
    pub fn account(&self, name: &str) -> Option<&str> {
        self.accounts.iter().find(|(_, a)| a.name == name).map(|(id, _)| id.as_str())
    }

    /// Each at least 1, and 100 where the session names none.
    pub fn limits(&self) -> Limits {
        let core = self.capabilities.get(CORE).unwrap_or(&Value::Null);
        let [get, set, calls] = ["maxObjectsInGet", "maxObjectsInSet", "maxCallsInRequest"]
            .map(|n| core[n].as_u64().map_or(100, |n| n as usize).max(1));
        Limits { get, set, calls }
    }
}

#[cfg(test)]
#[path = "../tests/session.rs"]
mod tests;
