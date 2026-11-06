//! The token endpoint (RFC 6749 3.2): a grant's request, and the tokens or refusal it answers.

use std::time::Duration;

use http::{StatusCode, Uri};
use private::Secret;
use serde::{Deserialize, Serialize};

use crate::{Client, Code, Failed, failure};

pub enum Grant<'a> {
    /// A code the issuer sent the browser back with, and the redirect its sign-in named.
    Code {
        code: &'a Code<'a>,
        redirect: &'a str,
    },
    Refresh(&'a Secret),
    Device(&'a Secret),
}

impl Grant<'_> {
    /// The fields of a request to the token endpoint `token`: the client's, `scope` if it is
    /// asked again, then the grant's. A client assertion in them is signed at `now`.
    pub fn form(
        &self,
        client: &Client,
        scope: Option<&str>,
        token: &Uri,
        now: i64,
    ) -> Result<Vec<(&'static str, String)>, Failed> {
        let mut fields = client.fields(&token.to_string(), now)?;
        fields.extend(scope.map(|scope| ("scope", scope.to_owned())));
        let grant: &[(&str, &str)] = match self {
            Grant::Code { code, redirect } => &[
                ("grant_type", "authorization_code"),
                ("code", &code.code),
                ("redirect_uri", redirect),
                ("code_verifier", code.pkce.verifier().expose()),
            ],
            Grant::Refresh(token) => {
                &[("grant_type", "refresh_token"), ("refresh_token", token.expose())]
            }
            Grant::Device(code) => &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", code.expose()),
            ],
        };
        fields.extend(grant.iter().map(|&(key, value)| (key, value.to_owned())));
        Ok(fields)
    }
}

/// What a token reply grants (RFC 6749 5.1), had only from [`Tokens::read`].
pub struct Tokens {
    pub access: Access,
    /// Required by RFC 6749, and left out by stand-ins that answer only what their callers read.
    pub token_type: Option<String>,
    pub refresh: Option<Secret>,
}

/// An access token, and when it expires if its issuer said: RFC 6749 5.1 only recommends saying.
#[derive(Clone, Serialize, Deserialize)]
pub struct Access {
    pub token: Secret,
    /// In seconds since the epoch.
    pub expires_at: Option<i64>,
}

impl Access {
    /// Whether a request sent at `now` still carries a live token when it reaches its server,
    /// as it does within `reach`. A token of no stated expiry is live until a server refuses it.
    pub fn fresh(&self, now: i64, reach: Duration) -> bool {
        self.expires_at.is_none_or(|at| now.saturating_add_unsigned(reach.as_secs()) < at)
    }
}

#[derive(Deserialize)]
struct Reply {
    access_token: Secret,
    token_type: Option<String>,
    refresh_token: Option<Secret>,
    expires_in: Option<i64>,
}

impl Tokens {
    /// The reply to a request sent at `sent`: no reply is generated before it, so an expiry
    /// counted from it comes no later than the issuer's.
    pub fn read(status: StatusCode, body: &[u8], sent: i64) -> Result<Tokens, Failed> {
        if status != StatusCode::OK {
            return Err(failure("token endpoint", status, body));
        }
        let shape = || Failed::Unacceptable("the token reply isn't of the shape it takes".into());
        let reply: Reply = serde_json::from_slice(body).map_err(|_| shape())?;
        let expires_at = reply.expires_in.map(|s| sent.checked_add(s).ok_or_else(shape));
        Ok(Tokens {
            access: Access { token: reply.access_token, expires_at: expires_at.transpose()? },
            token_type: reply.token_type,
            refresh: reply.refresh_token,
        })
    }
}

#[cfg(test)]
#[path = "../tests/token.rs"]
mod tests;
