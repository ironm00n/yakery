//! A sign-in sent to the issuer (RFC 7636): the state it must come back with, and the verifier
//! of the S256 challenge it was sent with.

use anyhow::Result;
use private::Secret;

use crate::{Endpoints, base64url, sha256};

pub struct Pkce {
    state: Secret,
    verifier: Secret,
}

impl Pkce {
    /// 32 random bytes each, in base64url: as long as RFC 7636 lets a verifier be at the least.
    pub fn random() -> Result<Pkce> {
        let text = || {
            let mut drawn = [0; 32];
            private::random::fill(&mut drawn).map(|()| Secret::new(base64url(drawn)))
        };
        Ok(Pkce { state: text()?, verifier: text()? })
    }

    /// One the caller kept while the browser was at the issuer.
    pub fn kept(state: Secret, verifier: Secret) -> Pkce {
        Pkce { state, verifier }
    }

    pub fn state(&self) -> &Secret {
        &self.state
    }

    pub fn verifier(&self) -> &Secret {
        &self.verifier
    }

    pub fn challenge(&self) -> String {
        base64url(sha256(self.verifier.expose().as_bytes()))
    }

    pub fn authorization<'a>(
        &'a self,
        at: &'a Endpoints,
        client: &'a str,
        redirect: &'a str,
        scope: &'a str,
    ) -> Authorization<'a> {
        Authorization { pkce: self, at, client, redirect, scope, mode: None }
    }
}

/// Where to send a browser to sign in (RFC 6749 4.1.1).
pub struct Authorization<'a> {
    pkce: &'a Pkce,
    at: &'a Endpoints,
    client: &'a str,
    redirect: &'a str,
    scope: &'a str,
    mode: Option<&'static str>,
}

impl Authorization<'_> {
    pub fn response_mode(self, mode: &'static str) -> Self {
        Authorization { mode: Some(mode), ..self }
    }

    pub fn url(&self) -> String {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("response_type", "code")
            .append_pair("client_id", self.client)
            .append_pair("redirect_uri", self.redirect)
            .append_pair("scope", self.scope)
            .append_pair("state", self.pkce.state.expose())
            .append_pair("code_challenge", &self.pkce.challenge())
            .append_pair("code_challenge_method", "S256")
            .extend_pairs(self.mode.map(|mode| ("response_mode", mode)))
            .finish();
        let at = &self.at.authorization;
        format!("{at}{}{query}", if at.query().is_some() { '&' } else { '?' })
    }
}

#[cfg(test)]
#[path = "../tests/pkce.rs"]
mod tests;
