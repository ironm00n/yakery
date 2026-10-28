use anyhow::{Result, bail};

#[doc(hidden)]
pub use {anyhow, serde};

/// `text! { Ty(|s| parse, |v| shown); }`: `FromStr` is `parse` and `Display` writes `shown`, and serde goes through
/// the two, so a `Ty` only ever holds what `parse` accepted. A type named like a secret fails to compile, since a
/// secret is a `private::Secret`, which has no `Display`.
#[macro_export]
macro_rules! text {
    ($($ty:ident(|$s:ident| $parse:expr, |$v:ident| $show:expr);)*) => {$(
        const _: () = assert!(
            !$crate::secret_like(stringify!($ty)),
            concat!(stringify!($ty), " is named like a secret, which text! would let print"),
        );
        impl ::std::str::FromStr for $ty {
            type Err = $crate::anyhow::Error;
            fn from_str($s: &str) -> $crate::anyhow::Result<$ty> {
                $parse
            }
        }
        impl ::std::fmt::Display for $ty {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let $v = self;
                ::std::fmt::Display::fmt(&$show, f)
            }
        }
        impl $crate::serde::Serialize for $ty {
            fn serialize<S: $crate::serde::Serializer>(&self, s: S) -> ::std::result::Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> $crate::serde::Deserialize<'de> for $ty {
            fn deserialize<D: $crate::serde::Deserializer<'de>>(d: D) -> ::std::result::Result<$ty, D::Error> {
                let text = <::std::string::String as $crate::serde::Deserialize>::deserialize(d)?;
                text.parse().map_err(|e| <D::Error as $crate::serde::de::Error>::custom(format!("{e:#}")))
            }
        }
    )*};
}

#[doc(hidden)]
pub const fn secret_like(name: &str) -> bool {
    const WORDS: [&[u8]; 7] =
        [b"token", b"secret", b"password", b"passwd", b"passphrase", b"credential", b"key"];
    let name = name.as_bytes();
    let mut w = 0;
    while w < WORDS.len() {
        let word = WORDS[w];
        let mut at = 0;
        while at + word.len() <= name.len() {
            let mut i = 0;
            while i < word.len() && name[at + i].to_ascii_lowercase() == word[i] {
                i += 1;
            }
            if i == word.len() {
                return true;
            }
            at += 1;
        }
        w += 1;
    }
    false
}

/// RFC 5322's dot-atom: runs of atext joined by single dots.
pub fn dot_atom(s: &str) -> bool {
    let atext = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c);
    s.split('.').all(|run| !run.is_empty() && run.chars().all(atext))
}

/// A host name of two or more letter-digit-hyphen labels.
pub fn ldh(s: &str) -> bool {
    let label = |l: &str| {
        (1..=63).contains(&l.len())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    s.len() <= 253 && s.split('.').count() >= 2 && s.split('.').all(label)
}

/// A plain mail address, equal to another regardless of ASCII case.
#[derive(Clone, Debug)]
pub struct Address(String, String);

text! {
    Address(|s| match s.split_once('@') {
        Some((local, domain)) if s.len() <= Address::MOST && dot_atom(local) && ldh(domain) => {
            Ok(Address(local.into(), domain.into()))
        }
        _ => bail!("{} is not a plain email address", s.escape_debug()),
    }, |a| format!("{}@{}", a.0, a.1));
}

impl Address {
    /// The longest address, in bytes, all ASCII.
    pub const MOST: usize = 254;
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

    /// The local part without its `+detail`, which every subaddress of the address shares.
    pub fn user(&self) -> &str {
        self.0.split_once('+').map_or(&self.0, |(user, _)| user)
    }

    pub fn subaddress(&self, detail: &impl std::fmt::Display) -> Result<Address> {
        format!("{}+{detail}@{}", self.0, self.1).parse()
    }
}

#[cfg(test)]
mod tests;
