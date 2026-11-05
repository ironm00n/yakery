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

/// The front's questions are a few hundred bytes.
const MOST_ASKED: u64 = 64 * 1024;
pub const MOST_ANSWERED: u64 = 1024 * 1024;
/// How long after the front asks the core may still claim a link for its booking: a question
/// that waited out a down core is refused, not carried out for a booker long gone.
pub const CLAIMED_WITHIN: SignedDuration = SignedDuration::from_secs(10);
/// Past the claim's deadline and the one Stalwart call that follows a claim, so a booking the
/// front stops waiting for was never claimed.
pub const ANSWERED_WITHIN: Duration =
    Duration::from_secs(CLAIMED_WITHIN.as_secs().unsigned_abs() + stalwart::TIMEOUT.as_secs() + 5);

pub type Answer<T> = Result<T, Refusal>;

/// A question of the front's, whose type names the answer it gets.
pub trait Question: Into<FrontAsk> {
    type Answer: Serialize + DeserializeOwned;
    /// What an answer lost after the core took the question means.
    const LOST: Refusal;
}

macro_rules! questions {
    ($($question:ident => $variant:ident -> $answer:ty, lost $lost:ident;)*) => {
        #[derive(Serialize, Deserialize)]
        pub enum FrontAsk { $($variant($question)),* }

        $(
            impl From<$question> for FrontAsk {
                fn from(question: $question) -> FrontAsk {
                    FrontAsk::$variant(question)
                }
            }

            impl Question for $question {
                type Answer = $answer;
                const LOST: Refusal = Refusal::$lost;
            }
        )*
    };
}

questions! {
    WeekAsk => Week -> Week, lost Unavailable;
    LinkAsk => Link -> LinkView, lost Unavailable;
    BookAsk => Book -> Booked, lost Unanswered;
}

/// Weeks past this one.
#[derive(Serialize, Deserialize)]
pub struct WeekAsk(pub u8);

#[derive(Serialize, Deserialize)]
pub struct LinkAsk(pub Token);

#[derive(Serialize, Deserialize)]
pub struct BookAsk {
    pub token: Token,
    pub start: Timestamp,
    pub booker: Booker,
    pub by: Timestamp,
}

/// `later` when the week after is inside the horizon too.
#[derive(Serialize, Deserialize)]
pub struct Week {
    pub monday: jiff::civil::Date,
    pub blocks: Vec<Period>,
    pub open: Vec<Period>,
    pub later: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Booked {
    pub start: Timestamp,
    pub duration: SignedDuration,
    pub booker: Booker,
    pub owner: String,
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
