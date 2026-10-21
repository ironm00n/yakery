use std::fmt;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::random::Draws;

const PASSWORD_CLASSES: [&[u8]; 4] = [
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    b"abcdefghijklmnopqrstuvwxyz",
    b"0123456789",
    b"!#%&*+-./:;=?@^_~",
];

/// A password or token: no `Display`, and `Debug` shows nothing of it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn password(len: usize) -> Result<Self> {
        let everything = PASSWORD_CLASSES.concat();
        let mut draws = Draws::default();
        let mut chars = PASSWORD_CLASSES
            .iter()
            .map(|class| draws.pick(class))
            .collect::<Result<Vec<u8>>>()?;
        while chars.len() < len {
            chars.push(draws.pick(&everything)?);
        }
        draws.shuffle(&mut chars)?;
        Ok(Secret(chars.into_iter().map(char::from).collect()))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hold_every_class() {
        for _ in 0..200 {
            let s = Secret::password(40).unwrap();
            let s = s.expose();
            assert_eq!(s.len(), 40);
            for class in PASSWORD_CLASSES {
                assert!(s.bytes().any(|b| class.contains(&b)), "{class:?}");
            }
            assert!(s.bytes().all(|b| PASSWORD_CLASSES.concat().contains(&b)));
        }
        assert_ne!(Secret::password(40).unwrap(), Secret::password(40).unwrap());
    }

    #[test]
    fn debug_hides_the_value() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(..)");
    }
}
