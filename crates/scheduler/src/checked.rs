//! A value that has passed its check: every way to make a [`Checked`], in code or through serde,
//! runs [`Check::check`], so the core checks what the front and `scheduler link` send it.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

pub trait Check: Sized {
    type Problem: fmt::Display;

    /// The value as it is kept, or what is wrong with it.
    fn check(self) -> Result<Self, Self::Problem>;
}

#[derive(Clone, Debug)]
pub struct Checked<T>(T);

impl<T: Check> Checked<T> {
    pub fn new(value: T) -> Result<Checked<T>, T::Problem> {
        value.check().map(Checked)
    }
}

impl<T> Checked<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Checked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Serialize> Serialize for Checked<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de, T: Check + Deserialize<'de>> Deserialize<'de> for Checked<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Checked<T>, D::Error> {
        Checked::new(T::deserialize(d)?).map_err(de::Error::custom)
    }
}
