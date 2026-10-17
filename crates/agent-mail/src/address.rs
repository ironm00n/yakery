use crate::prelude::*;

macro_rules! text {
    ($($ty:ident(|$s:ident| $parse:expr, |$v:ident| $show:expr);)*) => {$(
        impl std::str::FromStr for $ty {
            type Err = anyhow::Error;
            fn from_str($s: &str) -> Result<$ty> {
                $parse
            }
        }
        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let $v = self;
                f.write_str(&$show)
            }
        }
    )*};
}
pub(crate) use text;

fn dot_atom(s: &str) -> bool {
    let atext = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c);
    s.split('.').all(|run| !run.is_empty() && run.chars().all(atext))
}

fn ldh(s: &str) -> bool {
    let label = |l: &str| {
        (1..=63).contains(&l.len())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    s.len() <= 253 && s.split('.').count() >= 2 && s.split('.').all(label)
}

#[derive(Clone, Debug)]
pub struct Address(String, String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic(String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageId(String);

text! {
    Address(|s| match s.split_once('@') {
        Some((local, domain)) if s.len() <= 254 && dot_atom(local) && ldh(domain) => {
            Ok(Address(local.into(), domain.into()))
        }
        _ => bail!("{} is not a plain email address", s.escape_debug()),
    }, |a| format!("{}@{}", a.0, a.1));
    Topic(|s| {
        let word = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ensure!(word, "topic \"{s}\" may hold only letters, digits and dashes");
        Ok(Topic(s.into()))
    }, |t| t.0.clone());
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

impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0) && self.1.eq_ignore_ascii_case(&other.1)
    }
}

impl Eq for Address {}

impl Address {
    pub fn local(&self) -> &str {
        &self.0
    }

    pub fn domain(&self) -> &str {
        &self.1
    }

    pub fn topic(&self, topic: &Topic) -> Result<Address> {
        format!("{}+{}@{}", self.0, topic.0, self.1).parse()
    }
}

impl MessageId {
    pub fn all(ids: &[String]) -> Vec<MessageId> {
        ids.iter().filter_map(|id| id.parse().ok()).collect()
    }
}
