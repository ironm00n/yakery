//! The token endpoint (RFC 6749 3.2): a grant's request, and the tokens or refusal it answers.

use http::{StatusCode, Uri};
use private::Secret;
use serde::Deserialize;

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
    pub access: Secret,
    /// Required by RFC 6749, and left out by stand-ins that answer only what their callers read.
    pub token_type: Option<String>,
    pub refresh: Option<Secret>,
    pub expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct Reply {
    access_token: Secret,
    token_type: Option<String>,
    refresh_token: Option<Secret>,
    expires_in: Option<i64>,
}

impl Tokens {
    pub fn read(status: StatusCode, body: &[u8]) -> Result<Tokens, Failed> {
        if status != StatusCode::OK {
            return Err(failure("token endpoint", status, body));
        }
        let shape = || Failed::Unacceptable("the token reply isn't of the shape it takes".into());
        let reply: Reply = serde_json::from_slice(body).map_err(|_| shape())?;
        Ok(Tokens {
            access: reply.access_token,
            token_type: reply.token_type,
            refresh: reply.refresh_token,
            expires_in: reply.expires_in,
        })
    }
}

#[cfg(test)]
#[path = "../tests/token.rs"]
mod tests;
