//! The signed-in login and the subaddresses that count as ours.

use std::{fmt, str::FromStr};

use anyhow::{Result, bail};

use crate::address::{Address, Topic};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login(Address);

impl Login {
    pub fn address(&self) -> &Address {
        &self.0
    }

    pub fn topic_address(&self, topic: &Topic) -> String {
        format!("{}+{topic}@{}", self.0.local(), self.0.domain())
    }

    /// Mail from the login itself or from one of its topic subaddresses.
    pub fn owns(&self, email: &str) -> bool {
        self.detail(email).is_some()
    }

    pub fn is_topic_address(&self, email: &str) -> bool {
        matches!(self.detail(email), Some(Some(_)))
    }

    pub fn first_topic_address<'a>(
        &self,
        emails: impl IntoIterator<Item = &'a str>,
    ) -> Option<&'a str> {
        emails.into_iter().find(|e| self.is_topic_address(e))
    }

    fn detail(&self, email: &str) -> Option<Option<Topic>> {
        let address: Address = email.parse().ok()?;
        if !address.domain().eq_ignore_ascii_case(self.0.domain()) {
            return None;
        }
        let (base, detail) = match address.local().split_once('+') {
            Some((base, detail)) => (base, Some(detail.parse().ok()?)),
            None => (address.local(), None),
        };
        base.eq_ignore_ascii_case(self.0.local()).then_some(detail)
    }
}

impl FromStr for Login {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let address: Address = s.parse()?;
        if address.local().contains('+') {
            bail!("login {s} must be a plain address, not a subaddress");
        }
        Ok(Login(address))
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> Login {
        "agent@example.org".parse().unwrap()
    }

    #[test]
    fn login_parsing() {
        assert!("ada@agent.example.org".parse::<Login>().is_ok());
        assert!("agent+x@example.org".parse::<Login>().is_err());
        assert!("agent".parse::<Login>().is_err());
    }

    #[test]
    fn topic_address_uses_the_login_local_part_and_domain() {
        let ada: Login = "ada@agent.example.org".parse().unwrap();
        assert_eq!(
            ada.topic_address(&"pin".parse().unwrap()),
            "ada+pin@agent.example.org"
        );
    }

    #[test]
    fn ours_is_the_login_or_its_topic_subaddresses() {
        let me = agent();
        for ours in [
            "agent@example.org",
            "AGENT@Example.org",
            "agent+tooling@example.org",
            "Agent+A-1@example.org",
        ] {
            assert!(me.owns(ours), "{ours}");
        }
        for theirs in [
            "owner@example.org",
            "agent@agent.example.org",
            "agents@example.org",
            "agent+@example.org",
            "agent+a_b@example.org",
            "ada+agent@example.org",
            "agent",
        ] {
            assert!(!me.owns(theirs), "{theirs}");
        }
    }

    #[test]
    fn only_subaddresses_are_topic_addresses() {
        let me = agent();
        assert!(me.is_topic_address("agent+tooling@example.org"));
        assert!(!me.is_topic_address("agent@example.org"));
        assert!(!me.is_topic_address("owner+tooling@example.org"));
    }
}
