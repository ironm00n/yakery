//! OAuth 2.0 and OpenID Connect, as a client: the requests an issuer takes and what its answers
//! say. Nothing here sends a request: a caller sends each through its own HTTP client.

mod callback;
mod client;
mod device;
mod endpoints;
pub mod entra;
mod pkce;
mod token;

use std::fmt;

pub use self::callback::{Code, Denied, Returned};
pub use self::client::{Assertion, Auth, Client};
pub use self::device::Device;
pub use self::endpoints::Endpoints;
pub use self::pkce::{Authorization, Pkce};
pub use self::token::{Grant, Tokens};

/// Why an exchange with the issuer came to nothing, in this crate's words or a fixed vocabulary,
/// never with a code or a token; an OAuth error's description, the issuer's text, is kept apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed {
    /// No issuer's answer: unreachable, or answering as no issuer would.
    Unreached(String),
    /// An OAuth error reply (RFC 6749 5.2).
    Refused(OAuthError),
    /// An exchange this crate won't make or take, such as a reply of another shape.
    Unacceptable(String),
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failed::Unreached(says) | Failed::Unacceptable(says) => f.write_str(says),
            Failed::Refused(error) => write!(f, "the issuer refused it: {}", error.code),
        }
    }
}

impl std::error::Error for Failed {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthError {
    pub code: ErrorCode,
    pub description: Option<String>,
}

macro_rules! codes {
    ($($code:ident = $name:literal),* $(,)?) => {
        /// An OAuth error code: RFC 6749 5.2's and 4.1.2.1's, and RFC 8628 3.5's.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum ErrorCode { $($code,)* Other(String) }

        impl ErrorCode {
            fn read(name: &str) -> ErrorCode {
                match name { $($name => ErrorCode::$code,)* other => ErrorCode::Other(other.to_owned()) }
            }
        }

        /// A code outside the RFCs' is the issuer's text, so it prints as no code of theirs.
        impl fmt::Display for ErrorCode {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { $(ErrorCode::$code => $name,)* ErrorCode::Other(_) => "a code outside OAuth's" })
            }
        }
    };
}

codes! {
    InvalidRequest = "invalid_request", InvalidClient = "invalid_client",
    InvalidGrant = "invalid_grant", UnauthorizedClient = "unauthorized_client",
    UnsupportedGrantType = "unsupported_grant_type", InvalidScope = "invalid_scope",
    AccessDenied = "access_denied", AuthorizationPending = "authorization_pending",
    SlowDown = "slow_down", ExpiredToken = "expired_token",
}

/// What an issuer's endpoint answering `status` with `body` says when the answer is no success:
/// a client error with an OAuth error body refuses; anything else is no issuer's answer.
pub(crate) fn failure(what: &str, status: http::StatusCode, body: &[u8]) -> Failed {
    #[derive(serde::Deserialize)]
    struct Error {
        error: String,
        error_description: Option<String>,
    }
    match serde_json::from_slice::<Error>(body) {
        Ok(Error { error, error_description }) if status.is_client_error() => {
            Failed::Refused(OAuthError {
                code: ErrorCode::read(&error),
                description: error_description,
            })
        }
        _ => Failed::Unreached(format!("the issuer's {what} answered {status}")),
    }
}

pub(crate) fn base64url(bytes: impl AsRef<[u8]>) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn sha256(bytes: &[u8]) -> ring::digest::Digest {
    ring::digest::digest(&ring::digest::SHA256, bytes)
}

#[cfg(test)]
#[path = "../tests/lib.rs"]
mod tests;
