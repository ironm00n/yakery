//! Whether a mail is Owen's word: its `Submission-Signature` verifies with the signer's key built in, it was signed
//! for his account and sent from one of his addresses, it was signed within ten minutes, and its signed To or Cc
//! names a topic the wait watches. What it says is then read from the signed bytes alone, never from headers above
//! them, which anyone who can put a message in the mailbox writes.

use mail_parser::{Address as Parsed, MessageParser};
use submission_signature::verify;
use text::Address;

use crate::DOMAIN;

/// Owen's account as Stalwart names it (ZITADEL's preferred_username at the domain), and the addresses he sends
/// from, each at [`DOMAIN`]. Both are matched whole: the sender alone could be a group's address another account
/// sends from, and a `+detail` of his is a name another account may hold.
#[cfg(not(test))]
const ACCOUNT: &str = "ironmoon";
#[cfg(not(test))]
const OWNERS: &[&str] = &["me", "ironmoon"];
#[cfg(test)]
const ACCOUNT: &str = "owner";
#[cfg(test)]
const OWNERS: &[&str] = &["owner"];
const AGE: i64 = 600;
const AHEAD: i64 = 60;

/// A mail as Owen signed it; `signature` is unique to it.
#[derive(Debug, PartialEq)]
pub struct Signed {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub text: String,
    pub signature: String,
}

fn emails(a: Option<&Parsed>) -> Vec<String> {
    a.map_or_else(Vec::new, |a| {
        a.iter().filter_map(|a| a.address.as_deref().map(str::to_lowercase)).collect()
    })
}

/// `raw` as Owen's word, or why it isn't.
pub fn check(public: &str, raw: &[u8], topics: &[Address], now: i64) -> Result<Signed, String> {
    let verified = verify(public, raw).map_err(|e| e.to_string())?;
    let (account, sender) = (verified.stamp.account.to_string(), verified.stamp.sender.to_string());
    let his = |local: &str, name: &str| name.eq_ignore_ascii_case(&format!("{local}@{DOMAIN}"));
    if !his(ACCOUNT, &account) {
        return Err(format!("signed for the account {account}, not Owen's"));
    }
    if !OWNERS.iter().any(|o| his(o, &sender)) {
        return Err(format!("signed as sent from {sender}, not one of Owen's addresses"));
    }
    match now - verified.stamp.t {
        age if age > AGE => return Err(format!("signed {age} s ago, over {AGE} s")),
        age if -age > AHEAD => {
            return Err(format!("signed {} s ahead of this clock, over {AHEAD} s", -age));
        }
        _ => {}
    }
    let parsed =
        MessageParser::new().parse(verified.message).ok_or("the signed bytes are no message")?;
    let (to, cc) = (emails(parsed.to()), emails(parsed.cc()));
    let named = |a: &String| a.parse::<Address>().is_ok_and(|a| topics.contains(&a));
    if !to.iter().chain(&cc).any(named) {
        return Err("signed, but to no topic this wait watches".into());
    }
    let texts: Vec<_> = (0..parsed.text_body_count()).filter_map(|i| parsed.body_text(i)).collect();
    Ok(Signed {
        from: emails(parsed.from()).join(", "),
        to,
        cc,
        subject: parsed.subject().unwrap_or_default().into(),
        text: texts.join("\n").replace("\r\n", "\n"),
        signature: verified.signature,
    })
}
