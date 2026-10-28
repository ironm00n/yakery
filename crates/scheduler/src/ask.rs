//! What the front and `scheduler link` may ask the core, one JSON value each way per connection
//! on a unix socket whose permissions systemd sets: the front's socket for its user's group, the
//! admin socket for root.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use jiff::{SignedDuration, Timestamp};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::link::{Booker, Digest, Issue, LinkId, Token};
use crate::slots::Period;
use crate::stalwart;

/// The most of a question the core reads: the front's are a few hundred bytes.
const MOST_ASKED: u64 = 64 * 1024;
/// The most of an answer the front reads: a link open every quarter hour of 8 weeks offers
/// 5,472 slots.
pub const MOST_ANSWERED: u64 = 1024 * 1024;
/// How long after the front asks the core may still claim a link for its booking: a question
/// that waited out a down core is refused, not carried out for a booker long gone.
pub const CLAIMED_WITHIN: SignedDuration = SignedDuration::from_secs(10);
/// How long the front waits for an answer: past the claim's deadline and the one Stalwart call
/// that follows a claim, so a booking it stops waiting for was never claimed. Under nginx's 60 s.
pub const ANSWERED_WITHIN: Duration =
    Duration::from_secs(CLAIMED_WITHIN.as_secs().unsigned_abs() + stalwart::TIMEOUT.as_secs() + 5);

#[derive(Serialize, Deserialize)]
pub enum FrontAsk {
    /// The public page's week, this one and up to the horizon's.
    Week(u8),
    Link(Token),
    Book {
        token: Token,
        start: Timestamp,
        booker: Booker,
        by: Timestamp,
    },
}

#[derive(Serialize, Deserialize)]
pub enum FrontAnswer {
    Week { monday: jiff::civil::Date, blocks: Vec<(Period, Option<String>)>, open: Vec<Period> },
    Link(LinkView),
    Booked { start: Timestamp, duration: SignedDuration, booker: Booker },
    Refused(Refusal),
}

#[derive(Serialize, Deserialize)]
pub struct LinkView {
    pub title: Option<String>,
    pub duration: SignedDuration,
    pub state: Shown,
}

#[derive(Serialize, Deserialize)]
pub enum Shown {
    Open(Vec<Timestamp>),
    Booked(Timestamp),
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    NoSuchLink,
    NoSuchWeek,
    Pending,
    Used,
    SlotGone,
    Unavailable,
    /// The front's own, when the core took a booking and gave no answer.
    Unanswered,
}

#[derive(Serialize, Deserialize)]
pub enum AdminAsk {
    Issue { digest: Digest, limits: Issue },
    List,
    Revoke(LinkId),
}

pub type AdminAnswer = Result<Vec<(LinkId, Issue, String)>, String>;

pub async fn ask<A: Serialize, B: DeserializeOwned>(socket: &Path, question: &A) -> Result<B> {
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to {}", socket.display()))?;
    stream.write_all(&serde_json::to_vec(question)?).await?;
    stream.shutdown().await?;
    let mut answer = vec![];
    stream.take(MOST_ANSWERED).read_to_end(&mut answer).await?;
    Ok(serde_json::from_slice(&answer)?)
}

/// Answers each connection on its own local task; a question that doesn't parse gets no answer.
pub async fn serve<A, B, F>(listener: UnixListener, answer: impl Fn(A) -> F + Clone + 'static)
where
    A: DeserializeOwned + 'static,
    B: Serialize,
    F: Future<Output = B>,
{
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { continue };
        let answer = answer.clone();
        tokio::task::spawn_local(async move {
            let mut question = vec![];
            if (&mut stream).take(MOST_ASKED).read_to_end(&mut question).await.is_err() {
                return;
            }
            let Ok(question) = serde_json::from_slice(&question) else { return };
            let reply = serde_json::to_vec(&answer(question).await).expect("answers serialize");
            let _ = stream.write_all(&reply).await;
        });
    }
}

#[cfg(test)]
#[path = "../tests/ask.rs"]
mod tests;
