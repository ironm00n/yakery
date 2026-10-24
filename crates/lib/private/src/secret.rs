use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use subtle::{Choice, ConstantTimeEq};

use crate::{hex, random};

pub(crate) const PASSWORD_CLASSES: [&[u8]; 4] = [
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    b"abcdefghijklmnopqrstuvwxyz",
    b"0123456789",
    b"!#%&*+-./:;=?@^_~",
];

/// Prints as `Secret(..)` and compares in constant time; only `expose` and serde, for storing it, read it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// `text` as a token of `bytes` bytes in hex, kept lowercase.
    pub fn hex(text: &str, bytes: usize) -> Result<Secret> {
        ensure!(text.len() == 2 * bytes, "a token of {bytes} bytes has {} hex digits", 2 * bytes);
        hex::decode(text)?;
        Ok(Secret(text.to_ascii_lowercase()))
    }

    /// At least one of each class, shuffled.
    pub fn password(len: usize) -> Result<Secret> {
        let pick = |class: &[u8]| random::below(class.len()).map(|i| class[i]);
        let all = PASSWORD_CLASSES.concat();
        let mut chars = PASSWORD_CLASSES.map(pick).into_iter().collect::<Result<Vec<u8>>>()?;
        while chars.len() < len {
            chars.push(pick(&all)?);
        }
        for i in (1..chars.len()).rev() {
            chars.swap(i, random::below(i + 1)?);
        }
        Ok(Secret(chars.into_iter().map(char::from).collect()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl ConstantTimeEq for Secret {
    fn ct_eq(&self, other: &Secret) -> Choice {
        self.0.as_bytes().ct_eq(other.0.as_bytes())
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Secret) -> bool {
        self.ct_eq(other).into()
    }
}

impl Eq for Secret {}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}
