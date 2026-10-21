//! Addresses, topics and message ids, each parsed strictly enough that its text can go into a header unchanged.

use std::{fmt, str::FromStr};

use anyhow::{Result, bail};

/// RFC 5322 `atext`.
fn is_atext(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c)
}

/// RFC 5322 `dot-atom-text`: atext runs joined by single dots.
fn is_dot_atom(s: &str) -> bool {
    !s.is_empty()
        && s.split('.')
            .all(|run| !run.is_empty() && run.chars().all(is_atext))
}

/// Letters, digits and hyphens, one to 63 per label, no hyphen at either end, at least two labels.
fn is_ldh_domain(s: &str) -> bool {
    let label = |l: &str| {
        (1..=63).contains(&l.len())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    s.len() <= 253 && s.split('.').count() >= 2 && s.split('.').all(label)
}

/// A mailbox: a dot-atom local part and an LDH domain, nothing else. Equality ignores case.
#[derive(Clone, Debug)]
pub struct Address {
    local: String,
    domain: String,
}

impl Address {
    pub fn local(&self) -> &str {
        &self.local
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }
}

impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        self.local.eq_ignore_ascii_case(&other.local)
            && self.domain.eq_ignore_ascii_case(&other.domain)
    }
}

impl Eq for Address {}

impl FromStr for Address {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('@') {
            Some((local, domain))
                if s.len() <= 254 && is_dot_atom(local) && is_ldh_domain(domain) =>
            {
                Ok(Address {
                    local: local.into(),
                    domain: domain.into(),
                })
            }
            _ => bail!("{} is not a plain email address", s.escape_debug()),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local, self.domain)
    }
}

/// The detail of a subaddress: letters, digits and dashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic(String);

impl FromStr for Topic {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            bail!("topic \"{s}\" may hold only letters, digits and dashes");
        }
        Ok(Topic(s.into()))
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An RFC 5322 `msg-id` without its angle brackets: `dot-atom-text "@" (dot-atom-text / no-fold-literal)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageId(String);

impl MessageId {
    /// The ids that parse; the rest are dropped rather than sent.
    pub fn all_valid(ids: &[String]) -> Vec<MessageId> {
        ids.iter().filter_map(|id| id.parse().ok()).collect()
    }

    /// For JMAP's `messageId`, `inReplyTo` and `references`, which carry ids without brackets.
    pub fn bare(&self) -> &str {
        &self.0
    }
}

impl FromStr for MessageId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let bare = s
            .strip_prefix('<')
            .and_then(|s| s.strip_suffix('>'))
            .unwrap_or(s);
        let literal = |r: &str| {
            r.strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .is_some_and(|d| {
                    d.chars()
                        .all(|c| ('!'..='~').contains(&c) && !"[]\\".contains(c))
                })
        };
        match bare.split_once('@') {
            Some((left, right)) if is_dot_atom(left) && (is_dot_atom(right) || literal(right)) => {
                Ok(MessageId(bare.into()))
            }
            _ => bail!("{} is not a message id", s.escape_debug()),
        }
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_plain_mailboxes_only() {
        for ok in [
            "a@b.dev",
            "agent+tooling@example.org",
            "o'neil.x@sub.example-1.com",
            "A@Example.ORG",
        ] {
            assert!(ok.parse::<Address>().is_ok(), "{ok}");
        }
        for bad in [
            "evil@example.com>,<x@example.org",
            "\"a@b\"@example.org",
            "a@b@example.org",
            "a b@example.org",
            "a@example.org\r\nBcc: x@y.z",
            "<a@example.org>",
            "a,b@example.org",
            ".a@example.org",
            "a..b@example.org",
            "a@-example.org",
            "a@example",
            "a@example.org.",
            "a@[127.0.0.1]",
            "ä@example.org",
            "a@exämple.org",
            "@example.org",
            "a@",
        ] {
            assert!(bad.parse::<Address>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn addresses_compare_without_case() {
        assert_eq!(
            "Agent@Example.org".parse::<Address>().unwrap(),
            "agent@example.org".parse().unwrap()
        );
    }

    #[test]
    fn topics_hold_letters_digits_and_dashes() {
        assert!("agent-mail-test".parse::<Topic>().is_ok());
        assert!("Tooling2".parse::<Topic>().is_ok());
        for bad in ["", "a+b", "a b", "a_b", "a@b", "é"] {
            assert!(bad.parse::<Topic>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn message_ids_follow_msg_id_syntax() {
        let id: MessageId = "<abc.1@mail.example.org>".parse().unwrap();
        assert_eq!(
            (id.to_string().as_str(), id.bare()),
            ("<abc.1@mail.example.org>", "abc.1@mail.example.org")
        );
        assert!("x@[1.2.3.4]".parse::<MessageId>().is_ok());
        for bad in [
            "abc\r@x.dev",
            "abc@x.dev\r\nBcc: y",
            "a b@x",
            "noat",
            "a@b@c",
            "<a@b",
        ] {
            assert!(bad.parse::<MessageId>().is_err(), "{bad:?}");
        }
        let kept = MessageId::all_valid(&["good@x.dev".into(), "bad\r@x.dev".into()]);
        assert_eq!(kept, ["good@x.dev".parse::<MessageId>().unwrap()]);
    }
}
