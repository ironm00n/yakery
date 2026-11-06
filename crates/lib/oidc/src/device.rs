//! The device authorization grant (RFC 8628): a code the user approves elsewhere, which the
//! client then trades at the token endpoint by [`crate::Grant::Device`].

use std::time::{Duration, Instant};

use http::{StatusCode, Uri};
use private::Secret;
use serde::Deserialize;

use crate::{Client, ErrorCode, Failed, OAuthError, failure};

#[derive(Deserialize)]
pub struct Device {
    pub device_code: Secret,
    pub user_code: String,
    pub verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

/// The wait between polls when the issuer names none (RFC 8628 3.2).
const INTERVAL: Duration = Duration::from_secs(5);
/// How much longer every later wait is after a `slow_down` (RFC 8628 3.5).
const SLOWER: Duration = Duration::from_secs(5);

/// When to poll the token endpoint for a device's grant (RFC 8628 3.5).
pub struct Polling {
    every: Duration,
    asked: Instant,
    lasts: Duration,
}

impl Polling {
    /// How long to wait, at `now`, before polling again after the issuer answered `error`.
    pub fn next(&mut self, error: OAuthError, now: Instant) -> Result<Duration, Failed> {
        match error.code {
            ErrorCode::AuthorizationPending => {}
            ErrorCode::SlowDown => self.every = self.every.saturating_add(SLOWER),
            _ => return Err(Failed::Refused(error)),
        }
        let polled_at = now.saturating_duration_since(self.asked).saturating_add(self.every);
        if polled_at >= self.lasts {
            return Err(Failed::Unacceptable(
                "the device code expires before its next poll".into(),
            ));
        }
        Ok(self.every)
    }
}

impl Device {
    /// The device authorization request's fields (RFC 8628 3.1). A client assertion in them is
    /// signed at `now`, for the token endpoint `token`.
    pub fn form(
        client: &Client,
        scope: &str,
        token: &Uri,
        now: i64,
    ) -> Result<Vec<(&'static str, String)>, Failed> {
        let mut fields = client.fields(&token.to_string(), now)?;
        fields.push(("scope", scope.to_owned()));
        Ok(fields)
    }

    /// The polls for a code asked for at `asked`: its lifetime counts from the reply, which comes
    /// no earlier.
    pub fn polling(&self, asked: Instant) -> Polling {
        let every = self.interval.map_or(INTERVAL, Duration::from_secs);
        Polling { every, asked, lasts: Duration::from_secs(self.expires_in) }
    }

    pub fn read(status: StatusCode, body: &[u8]) -> Result<Device, Failed> {
        if status != StatusCode::OK {
            return Err(failure("device authorization endpoint", status, body));
        }
        let shape = "the device authorization reply isn't of the shape it takes";
        serde_json::from_slice(body).map_err(|_| Failed::Unacceptable(shape.into()))
    }
}

#[cfg(test)]
#[path = "../tests/device.rs"]
mod tests;
