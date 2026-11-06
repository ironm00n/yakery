use std::rc::Rc;

use anyhow::{Result, bail};
use jiff::Timestamp;
use oidc::{Pkce, Returned};
use private::{Dir, Secret};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum Login {
    Pending { state: Secret, verifier: Secret, until: Timestamp },
    Redeeming { state: Secret },
    Done(Result<(), String>),
}

/// Why a return to the callback redeems no code.
#[derive(Debug, PartialEq, Eq)]
pub enum Untaken {
    Nothing,
    /// It carries another sign-in's state, or came back past its sign-in's time.
    Stale,
    /// Another return is redeeming the code.
    Taken,
}

/// The waiting sign-in `returned` comes back from, recorded as redeeming. It awaits nothing, so no
/// other return reads the sign-in as waiting before it is recorded.
pub fn take(dir: &Dir, returned: &Returned, now: Timestamp) -> Result<Result<Pkce, Untaken>> {
    let (pkce, until) = match dir.load(LOGIN)? {
        Some(Login::Pending { state, verifier, until }) => (Pkce::kept(state, verifier), until),
        Some(Login::Redeeming { .. }) => return Ok(Err(Untaken::Taken)),
        _ => return Ok(Err(Untaken::Nothing)),
    };
    if !returned.returns_from(&pkce) || now > until {
        return Ok(Err(Untaken::Stale));
    }
    dir.save(LOGIN, &Some(Login::Redeeming { state: pkce.state().clone() }))?;
    Ok(Ok(pkce))
}

/// Ends the sign-in `take` recorded as redeeming under `state` with how `redeem` ends, unless a
/// newer sign-in has replaced it. The redeem runs as a task of its own, so a dropped connection
/// doesn't cancel it: Entra may have taken the code already, and a code is redeemed only once
/// (RFC 6749 §4.1.2).
pub async fn finish(
    dir: Rc<Dir>,
    state: Secret,
    redeem: impl Future<Output = Result<(), String>> + 'static,
) -> Result<(), String> {
    let redeeming = tokio::task::spawn_local(async move {
        let outcome = redeem.await;
        if let Err(e) = end(&dir, &state, &outcome) {
            yakery_log::warn!("recording the sign-in: {e:#}");
        }
        outcome
    });
    redeeming.await.unwrap_or_else(|e| Err(format!("the redeem stopped: {e}")))
}

fn end(dir: &Dir, state: &Secret, outcome: &Result<(), String>) -> Result<()> {
    match dir.load(LOGIN)? {
        Some(Login::Redeeming { state: taken }) if taken == *state => {
            dir.save(LOGIN, &Some(Login::Done(outcome.clone())))
        }
        _ => bail!("a newer sign-in replaced it"),
    }
}

/// Records as failed a sign-in a stopped daemon left redeeming: redeems run only in the daemon, so
/// none is in flight as it starts.
pub fn abandon(dir: &Dir) -> Result<()> {
    if let Some(Login::Redeeming { .. }) = dir.load(LOGIN)? {
        dir.save(LOGIN, &Some(Login::Done(Err(ABANDONED.into()))))?;
    }
    Ok(())
}

pub const ABANDONED: &str =
    "the sync stopped while redeeming the sign-in; run `outlook-sync login`";

#[derive(Default, Serialize, Deserialize)]
pub struct Tokens {
    pub signed_in: Option<SignedIn>,
}

impl Tokens {
    /// The sign-in, unless there is none or it asked for less than `needs`.
    pub fn signed_in_for(self, needs: &str) -> Result<SignedIn, String> {
        let unsigned = match self.signed_in {
            Some(s) if oidc::covers(&s.scope, needs) => return Ok(s),
            Some(s) => format!("the sign-in asked for `{}`, not `{needs}`", s.scope),
            None => "not signed in".into(),
        };
        Err(format!("{unsigned}; run `outlook-sync login`"))
    }
}

/// A refresh token, and the scope its sign-in asked for: every refresh of it asks for no more.
#[derive(Serialize, Deserialize)]
pub struct SignedIn {
    pub refresh: Secret,
    pub scope: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub client_state: Secret,
    pub expires: Timestamp,
}

/// Where the last delta run left off; its window is fixed in the link.
#[derive(Default, Serialize, Deserialize)]
pub struct Delta {
    pub link: Option<String>,
}

pub const WRITTEN: &str = "written.json";
pub const LOGIN: &str = "login.json";
pub const TOKENS: &str = "tokens.json";
pub const SUBSCRIPTION: &str = "subscription.json";
pub const DELTA: &str = "delta.json";
pub const KEY: &str = "key.pem";
pub const CERTIFICATE: &str = "certificate.pem";

#[cfg(test)]
#[path = "../tests/state.rs"]
mod tests;
