//! Every change is a pure function from one set of links to the next, which the core persists
//! before acting on it.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use anyhow::{Result, anyhow, bail, ensure};
use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp, ToSpan};
use private::{Secret, hex, random};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use text::{Address, text};

use crate::checked::{Check, Checked};
use crate::policy::{Gap, Window};
use crate::slots::{Period, Taken};

const TOKEN_BYTES: usize = 32;

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "Secret")]
pub struct Token(Secret);

impl TryFrom<Secret> for Token {
    type Error = anyhow::Error;

    fn try_from(secret: Secret) -> Result<Token> {
        Token::parse(secret.expose())
    }
}

impl Token {
    pub fn random() -> Result<Token> {
        Ok(Token(random::token(TOKEN_BYTES)?))
    }

    pub fn parse(text: &str) -> Result<Token> {
        Ok(Token(Secret::hex(text, TOKEN_BYTES)?))
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }

    pub fn digest(&self) -> Digest {
        let bytes = hex::decode(self.0.expose()).expect("a token is hex");
        Digest(digest(&SHA256, &bytes).as_ref().try_into().expect("SHA-256 is 32 bytes"))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LinkId([u8; 4]);

fn bytes<const N: usize>(s: &str) -> Result<[u8; N]> {
    hex::decode(s)?.try_into().map_err(|_| anyhow!("not {} hex digits", 2 * N))
}

text! {
    Digest(|s| Ok(Digest(bytes(s)?)), |d| hex::encode(&d.0));
    LinkId(|s| Ok(LinkId(bytes(s)?)), |i| hex::encode(&i.0));
}

impl Digest {
    pub fn id(&self) -> LinkId {
        LinkId(self.0[..4].try_into().unwrap())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, clap::Args)]
pub struct Issue {
    /// First day, in the owner's zone
    #[arg(long)]
    pub from: Date,
    /// Last day
    #[arg(long)]
    pub until: Date,
    #[arg(long)]
    pub duration: SignedDuration,
    #[arg(long, default_value_t = Gap::FLOOR)]
    pub gap: Gap,
    /// Bookable hours in place of the policy's, like "sat 13:00-17:00"; repeatable
    #[arg(long)]
    pub hours: Vec<Window>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub location: Option<String>,
}

pub type Limits = Checked<Issue>;

impl Check for Issue {
    type Problem = anyhow::Error;

    fn check(self) -> Result<Issue> {
        let weeks = self.from <= self.until && self.until <= self.from + MOST_WEEKS.weeks();
        ensure!(weeks, "a link's window is one day to {MOST_WEEKS} weeks");
        ensure!(
            DURATIONS.contains(&self.duration),
            "a meeting lasts {:#} to {:#}",
            DURATIONS.start(),
            DURATIONS.end()
        );
        let short = |s: &Option<String>| s.as_deref().is_none_or(|s| plain(s, MOST_TITLE));
        ensure!(
            short(&self.title) && short(&self.location),
            "titles and places are 1 to {MOST_TITLE} plain characters"
        );
        Ok(self)
    }
}

pub const MOST_WEEKS: i64 = 8;
pub const DURATIONS: std::ops::RangeInclusive<SignedDuration> =
    SignedDuration::from_mins(15)..=SignedDuration::from_hours(8);
/// In characters.
pub const MOST_TITLE: usize = 100;
pub const MOST_NAME: usize = 100;
pub const MOST_NOTE: usize = 2000;

fn plain(s: &str, most: usize) -> bool {
    (1..=most).contains(&s.chars().count()) && !s.chars().any(char::is_control)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Form {
    pub name: String,
    pub address: String,
    pub note: String,
}

pub type Booker = Checked<Form>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Address,
    Note,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Field::Name => write!(f, "Give a name of 1 to {MOST_NAME} characters."),
            Field::Address => f.write_str("That email address isn't one an invitation can go to."),
            Field::Note => write!(f, "Keep the note to {MOST_NOTE} characters or fewer."),
        }
    }
}

impl Check for Form {
    type Problem = Field;

    fn check(self) -> Result<Form, Field> {
        let [name, address, note] = [self.name, self.address, self.note.replace("\r\n", "\n")]
            .map(|s| s.trim().to_string());
        let lines = !note.chars().any(|c| c.is_control() && !"\r\n".contains(c));
        let fields = [
            (plain(&name, MOST_NAME), Field::Name),
            (address.parse::<Address>().is_ok(), Field::Address),
            (note.chars().count() <= MOST_NOTE && lines, Field::Note),
        ];
        match fields.into_iter().find(|(ok, _)| !ok) {
            Some((_, field)) => Err(field),
            None => Ok(Form { name, address, note }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Booking {
    pub start: Timestamp,
    pub booker: Booker,
}

/// How long after a send Stalwart may still be working on it. Stalwart's UID check isn't atomic
/// with its insert, so a create is sent again only once its last send can't be in flight.
pub const QUIET: SignedDuration = SignedDuration::from_mins(2);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub booking: Booking,
    pub sent: Option<Sent>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Sent {
    pub count: NonZeroU32,
    pub last: Timestamp,
}

impl Claim {
    pub fn due(&self, now: Timestamp) -> bool {
        self.sent.is_none_or(|sent| now >= sent.last + QUIET)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum State {
    Open,
    Claiming(Claim),
    Booked { booking: Booking, event: Option<String> },
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Open => write!(f, "open"),
            State::Claiming(Claim { booking, sent }) => {
                let times = sent.map_or(0, |s| s.count.get());
                write!(f, "claiming {}, sent {times} times", booking.start)
            }
            State::Booked { booking, .. } => {
                write!(f, "booked {} by {}", booking.start, booking.booker.address)
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Link {
    pub limits: Limits,
    pub state: State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Created {
    Event(String),
    /// An event with the link's UID exists: an earlier send of this same create landed.
    UidTaken,
    Refused(String),
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Links(BTreeMap<Digest, Link>);

const KEPT_AFTER_WINDOW: i64 = 30;

impl Links {
    pub fn get(&self, digest: &Digest) -> Option<&Link> {
        self.0.get(digest)
    }

    pub fn live(&self, digest: &Digest, today: Date) -> Option<&Link> {
        self.0.get(digest).filter(|l| l.limits.until >= today)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Digest, &Link)> {
        self.0.iter()
    }

    fn with(&self, digest: Digest, link: Option<Link>) -> Links {
        let mut next = self.0.clone();
        match link {
            Some(link) => next.insert(digest, link),
            None => next.remove(&digest),
        };
        Links(next)
    }

    pub fn issue(&self, digest: Digest, limits: Limits) -> Result<Links> {
        ensure!(!self.0.contains_key(&digest), "that token is already a link");
        ensure!(
            self.0.keys().all(|d| d.id() != digest.id()),
            "another link has id {}; issue again",
            digest.id()
        );
        Ok(self.with(digest, Some(Link { limits, state: State::Open })))
    }

    fn claimed(&self, digest: Digest) -> Option<(Limits, Claim)> {
        match self.0.get(&digest)? {
            Link { limits, state: State::Claiming(claim) } => Some((limits.clone(), claim.clone())),
            _ => None,
        }
    }

    pub fn claim(&self, digest: Digest, booking: Booking) -> Result<Links> {
        let link = self.0.get(&digest).filter(|l| matches!(l.state, State::Open));
        let link = link.ok_or_else(|| anyhow!("not open"))?;
        let claim = Claim { booking, sent: None };
        Ok(self.with(digest, Some(Link { state: State::Claiming(claim), ..link.clone() })))
    }

    /// The links with the claim's next send written down, and what to send, when one is due.
    pub fn sending(&self, digest: Digest, now: Timestamp) -> Option<(Links, Limits, Claim)> {
        let (limits, claim) = self.claimed(digest).filter(|(_, c)| c.due(now))?;
        let count = claim.sent.map_or(NonZeroU32::MIN, |s| s.count.saturating_add(1));
        let claim = Claim { sent: Some(Sent { count, last: now }), ..claim };
        let next = self.with(
            digest,
            Some(Link { limits: limits.clone(), state: State::Claiming(claim.clone()) }),
        );
        Some((next, limits, claim))
    }

    /// A taken UID means an earlier send landed. A refusal reopens the link only when it answers
    /// the one send there was; after a resend, an earlier send may have landed, so the claim stays.
    pub fn settle(&self, digest: Digest, created: Created) -> Links {
        let Some((limits, claim)) = self.claimed(digest) else { return self.clone() };
        let state = match created {
            Created::Event(id) => State::Booked { booking: claim.booking, event: Some(id) },
            Created::UidTaken => State::Booked { booking: claim.booking, event: None },
            Created::Refused(_) if claim.sent.is_none_or(|s| s.count == NonZeroU32::MIN) => {
                State::Open
            }
            Created::Refused(_) => State::Claiming(claim),
        };
        self.with(digest, Some(Link { limits, state }))
    }

    pub fn revoke(&self, id: LinkId) -> Result<Links> {
        let Some((digest, link)) = self.0.iter().find(|(d, _)| d.id() == id) else {
            bail!("no link {id}")
        };
        ensure!(matches!(link.state, State::Open), "link {id} is booked or being booked");
        Ok(self.with(*digest, None))
    }

    pub fn expire(&self, today: Date) -> Links {
        Links(
            self.0
                .iter()
                .filter(|(_, l)| l.limits.until + KEPT_AFTER_WINDOW.days() >= today)
                .map(|(d, l)| (*d, l.clone()))
                .collect(),
        )
    }

    pub fn claiming(&self) -> Vec<Digest> {
        self.0
            .iter()
            .filter(|(_, l)| matches!(l.state, State::Claiming(_)))
            .map(|(d, _)| *d)
            .collect()
    }

    pub fn taken(&self) -> Vec<Taken> {
        let booking = |l: &Link| match &l.state {
            State::Open => None,
            State::Claiming(Claim { booking: b, .. }) | State::Booked { booking: b, .. } => {
                Some(Taken {
                    period: Period::starting(b.start, l.limits.duration),
                    gap: l.limits.gap,
                })
            }
        };
        self.0.values().filter_map(booking).collect()
    }
}

#[cfg(test)]
#[path = "../tests/link.rs"]
pub(crate) mod tests;
