//! One-time links: a token kept only as its digest, what the link may book, and whether it has.
//! Every change is a pure function from one set of links to the next, which the core persists
//! before acting on it.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Context, Result, anyhow, bail, ensure};
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

/// 256 bits, printed once by `scheduler link new` and then held only by whoever has the URL.
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

/// A token's SHA-256: what the core keeps, and what the event's UID is made of.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

/// The first 8 hex of a digest, for listing and revoking.
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

/// What `scheduler link new` asks for; a [`Limits`] once checked.
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
    #[arg(long, default_value = "PT5M")]
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
        let weeks = self.from <= self.until && self.until <= self.from + 8.weeks();
        ensure!(weeks, "a link's window is one day to 8 weeks");
        let lengths = SignedDuration::from_mins(15)..=SignedDuration::from_hours(8);
        ensure!(lengths.contains(&self.duration), "a meeting lasts 15 minutes to 8 hours");
        let short = |s: &Option<String>| s.as_deref().is_none_or(|s| plain(s, 100));
        ensure!(
            short(&self.title) && short(&self.location),
            "titles and places are 1 to 100 plain characters"
        );
        Ok(self)
    }
}

/// The longest name and note a booker may give, in characters.
pub const MOST_NAME: usize = 100;
pub const MOST_NOTE: usize = 2000;

fn plain(s: &str, most: usize) -> bool {
    (1..=most).contains(&s.chars().count()) && !s.chars().any(char::is_control)
}

/// What a booker types; a [`Booker`] once checked, by the front for its answer and by the core
/// again.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
        f.write_str(match self {
            Field::Name => "Give a name of 1 to 100 characters.",
            Field::Address => "That email address isn't one an invitation can go to.",
            Field::Note => "Keep the note under 2,000 characters.",
        })
    }
}

impl Check for Form {
    type Problem = Field;

    fn check(self) -> Result<Form, Field> {
        let [name, address, note] =
            [self.name, self.address, self.note].map(|s| s.trim().to_string());
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

/// A booking whose create may or may not have reached Stalwart: sent `sends` times, the last at
/// `sent`, each send written down before it goes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub booking: Booking,
    pub sends: u32,
    pub sent: Option<Timestamp>,
}

impl Claim {
    pub fn due(&self, now: Timestamp) -> bool {
        self.sent.is_none_or(|sent| now >= sent + QUIET)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum State {
    Open,
    Claiming(Claim),
    Booked { booking: Booking, event: Option<String> },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Link {
    pub limits: Limits,
    pub state: State,
}

/// What Stalwart made of a create.
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
        let claim = Claim { booking, sends: 0, sent: None };
        Ok(self.with(digest, Some(Link { state: State::Claiming(claim), ..link.clone() })))
    }

    /// The claim's next send, written down before it goes; refused until the claim is due.
    pub fn sending(&self, digest: Digest, now: Timestamp) -> Result<Links> {
        let (limits, claim) =
            self.claimed(digest).filter(|(_, c)| c.due(now)).context("no send due")?;
        let claim = Claim { sends: claim.sends + 1, sent: Some(now), ..claim };
        Ok(self.with(digest, Some(Link { limits, state: State::Claiming(claim) })))
    }

    /// A taken UID means an earlier send landed. A refusal reopens the link only when it answers
    /// the one send there was; after a resend, an earlier send may have landed, so the claim stays.
    pub fn settle(&self, digest: Digest, created: Created) -> Links {
        let Some((limits, claim)) = self.claimed(digest) else { return self.clone() };
        let state = match created {
            Created::Event(id) => State::Booked { booking: claim.booking, event: Some(id) },
            Created::UidTaken => State::Booked { booking: claim.booking, event: None },
            Created::Refused(_) if claim.sends <= 1 => State::Open,
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
