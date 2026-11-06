//! Microsoft Entra ID's v2.0 endpoints. Entra won't let a web app redeem a code on PKCE alone,
//! so such a client proves itself with an [`crate::Assertion`].

use anyhow::{Context, Result};

use crate::Endpoints;

pub const MICROSOFT: &str = "https://login.microsoftonline.com";

/// The v2.0 endpoints at `origin` ([`MICROSOFT`] but for a stand-in) for `authority`: a tenant's
/// id, or `organizations`, `common` or `consumers`.
pub fn endpoints(origin: &str, authority: &str) -> Result<Endpoints> {
    let at = |path: &str| {
        let url = format!("{}/{authority}/oauth2/v2.0/{path}", origin.trim_end_matches('/'));
        url.parse().with_context(|| format!("Entra's {path} endpoint"))
    };
    Ok(Endpoints {
        authorization: at("authorize")?,
        token: at("token")?,
        userinfo: None,
        device: Some(at("devicecode")?),
    })
}

#[cfg(test)]
#[path = "../tests/entra.rs"]
mod tests;
