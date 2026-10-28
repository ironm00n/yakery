//! The `Submission-Signature` header the mail server writes above a message an account submits: the account, its
//! envelope sender and the time, signed with ed25519 over them and every byte of the message below the header.

use std::{fmt, str::FromStr};

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD_INDIFFERENT as B64};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use text::{Address, text};

pub const HEADER: &str = "Submission-Signature";

/// A login, in characters that can't end a field of the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account(String);

text! {
    Account(|s| {
        let allowed = |c: char| c.is_ascii_alphanumeric() || "._+-@".contains(c);
        ensure!((1..=255).contains(&s.len()) && s.chars().all(allowed), "{} is not a login the header carries", s.escape_debug());
        Ok(Account(s.into()))
    }, |a| a.0);
}

/// What a signature vouches for besides the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub account: Account,
    pub sender: Address,
    pub t: i64,
}

impl fmt::Display for Stamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v=1; account={}; sender={}; t={}", self.account, self.sender, self.t)
    }
}

impl FromStr for Stamp {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Stamp> {
        let fields: Vec<_> = s.split("; ").map(|f| f.split_once('=').unwrap_or((f, ""))).collect();
        let [("v", "1"), ("account", account), ("sender", sender), ("t", t)] = fields[..] else {
            bail!("the header's fields are not v=1, account, sender, t and sig");
        };
        let stamp = Stamp { account: account.parse()?, sender: sender.parse()?, t: t.parse()? };
        ensure!(stamp.to_string() == s, "the header is not written as the signer writes it");
        Ok(stamp)
    }
}

fn signed(stamp: &Stamp, message: &[u8]) -> Vec<u8> {
    [HEADER.as_bytes(), b": ", stamp.to_string().as_bytes(), b"\r\n", message].concat()
}

pub struct Signer(Ed25519KeyPair);

impl Signer {
    /// From an ed25519 seed, 32 bytes in base64.
    pub fn from_seed(text: &str) -> Result<Signer> {
        let seed = B64.decode(text.trim()).ok().filter(|s| s.len() == 32);
        let seed = seed.context("the seed is not 32 bytes in base64")?;
        Ed25519KeyPair::from_seed_unchecked(&seed)
            .map(Signer)
            .map_err(|_| anyhow!("not an ed25519 seed"))
    }

    pub fn public(&self) -> String {
        B64.encode(self.0.public_key())
    }

    /// The header's value for `message` under `stamp`.
    pub fn sign(&self, stamp: &Stamp, message: &[u8]) -> String {
        format!("{stamp}; sig={}", B64.encode(self.0.sign(&signed(stamp, message))))
    }
}

/// Each field of `raw`'s header block, as its name, its value as written and where the field ends; the block ends at
/// its blank line or at a line that is no field.
fn fields(raw: &[u8]) -> impl Iterator<Item = (&[u8], &[u8], usize)> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let start = at;
        loop {
            at = raw[at..].iter().position(|&b| b == b'\n').map_or(raw.len(), |i| at + i + 1);
            if !matches!(raw.get(at), Some(b' ' | b'\t')) {
                break;
            }
        }
        let field = &raw[start..at];
        let colon = field.iter().position(|&b| b == b':')?;
        Some((&field[..colon], &field[colon + 1..], at))
    })
}

/// The one `Submission-Signature` in a message's header block, verified.
#[derive(Debug, PartialEq)]
pub struct Verified<'a> {
    pub stamp: Stamp,
    /// As the header writes it, which no other message's header does.
    pub signature: String,
    /// The bytes below the header, as the account submitted them.
    pub message: &'a [u8],
}

/// The one `Submission-Signature` in `raw`'s header block, when `public` verifies its signature over the stamp and
/// the bytes below it.
pub fn verify<'a>(public: &str, raw: &'a [u8]) -> Result<Verified<'a>> {
    let key = B64.decode(public).ok().filter(|k| k.len() == 32);
    let key = key.context("no public key for submission signatures is built in")?;
    let found: Vec<_> =
        fields(raw).filter(|(name, ..)| name.eq_ignore_ascii_case(HEADER.as_bytes())).collect();
    let [(_, value, end)] = found[..] else {
        bail!("{} {HEADER} headers, not one", found.len());
    };
    let value: String = String::from_utf8_lossy(value).split(['\r', '\n']).collect();
    let (stamp, sig) = value.trim().rsplit_once("; sig=").context("the header has no sig")?;
    let stamp: Stamp = stamp.parse()?;
    let decoded = B64.decode(sig).map_err(|_| anyhow!("the sig is not base64"))?;
    let message = &raw[end..];
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&signed(&stamp, message), &decoded)
        .map_err(|_| anyhow!("the signature does not verify"))?;
    Ok(Verified { stamp, signature: sig.into(), message })
}

#[cfg(test)]
mod tests;
