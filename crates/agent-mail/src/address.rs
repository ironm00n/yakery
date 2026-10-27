use text::{dot_atom, text};

use crate::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Topic(String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId(String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageId(String);

text! {
    Topic(|s| {
        let word = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ensure!(word, "topic \"{s}\" may hold only letters, digits and dashes");
        Ok(Topic(s.into()))
    }, |t| &t.0);
    SessionId(|s| {
        let id = (1..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ensure!(id, "session id \"{}\" may hold only 1 to 64 letters, digits and dashes", s.escape_debug());
        Ok(SessionId(s.into()))
    }, |s| &s.0);
    MessageId(|s| {
        let bare = s.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(s);
        let literal = |r: &str| {
            let inner = r.strip_prefix('[').and_then(|r| r.strip_suffix(']'));
            let text = |c: char| ('!'..='~').contains(&c) && !"[]\\".contains(c);
            inner.is_some_and(|d| d.chars().all(text))
        };
        match bare.split_once('@') {
            Some((left, right)) if dot_atom(left) && (dot_atom(right) || literal(right)) => {
                Ok(MessageId(bare.into()))
            }
            _ => bail!("{} is not a message id", s.escape_debug()),
        }
    }, |m| format!("<{}>", m.0));
}

impl MessageId {
    pub fn all(ids: &[String]) -> Vec<MessageId> {
        ids.iter().filter_map(|id| id.parse().ok()).collect()
    }
}
