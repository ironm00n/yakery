//! Fixed-width hex identifiers. Validated at construction so every consumer
//! (argv, file names, JSON) can rely on the character set.

use std::fmt;
use std::fs::File;
use std::io::Read;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

macro_rules! hex_id {
    ($name:ident, $bytes:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub const LEN: usize = $bytes * 2;

            pub fn random() -> Result<Self> {
                Ok(Self(random_hex($bytes)?))
            }

            pub fn parse(s: &str) -> Result<Self> {
                if s.len() == Self::LEN && s.bytes().all(|b| b.is_ascii_hexdigit()) {
                    Ok(Self(s.to_ascii_lowercase()))
                } else {
                    bail!("bad {}: {s:?}", stringify!($name));
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = anyhow::Error;
            fn try_from(s: String) -> Result<Self> {
                Self::parse(&s)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }
    };
}

hex_id!(
    RunId,
    16,
    "Per-attempt run identifier. Public (it rides in `--comment`), never a capability."
);
hex_id!(
    Token,
    32,
    "Single-use callback secret. Lives in memory and 0600 files only."
);

impl Token {
    /// Constant-time equality; the token is the one secret in a run.
    pub fn matches(&self, other: &Token) -> bool {
        let (a, b) = (self.0.as_bytes(), other.0.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

pub fn random_hex(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(hex(&buf))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_reject() {
        let id = RunId::random().unwrap();
        assert_eq!(RunId::parse(id.as_str()).unwrap(), id);
        assert!(RunId::parse("abc").is_err());
        assert!(RunId::parse(&"zz".repeat(16)).is_err());
        assert!(Token::parse(id.as_str()).is_err());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<RunId>(&json).unwrap(), id);
    }

    #[test]
    fn token_compare() {
        let a = Token::random().unwrap();
        let b = Token::random().unwrap();
        assert!(a.matches(&a));
        assert!(!a.matches(&b));
    }
}
