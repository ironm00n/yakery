use std::fmt;

use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};

/// A mirror file's name, `<stem>.ics`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileName(String);

impl FileName {
    /// Graph ids hold characters a path shouldn't.
    pub fn of(id: &str) -> FileName {
        FileName(private::hex::encode(digest(&SHA256, id.as_bytes()).as_ref()))
    }

    pub fn stem(&self) -> &str {
        &self.0
    }

    /// The UID the sync writes into this file, by which the file is the sync's: no calendar app
    /// makes one, and no deployment's settings change it.
    pub fn uid(&self) -> String {
        format!("{}@outlook-sync", self.0)
    }
}

impl TryFrom<String> for FileName {
    type Error = String;

    fn try_from(name: String) -> Result<FileName, String> {
        match name.strip_suffix(".ics") {
            Some(stem) if !stem.is_empty() && !stem.contains('/') => Ok(FileName(stem.into())),
            _ => Err(format!("{name:?} is not a calendar file's name")),
        }
    }
}

impl std::str::FromStr for FileName {
    type Err = String;

    fn from_str(name: &str) -> Result<FileName, String> {
        FileName::try_from(name.to_string())
    }
}

impl fmt::Display for FileName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.ics", self.0)
    }
}

impl From<FileName> for String {
    fn from(name: FileName) -> String {
        name.to_string()
    }
}

#[cfg(test)]
#[path = "../tests/file.rs"]
mod tests;
