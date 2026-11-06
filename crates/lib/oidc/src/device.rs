//! The device authorization grant (RFC 8628): a code the user approves elsewhere, which the
//! client then trades at the token endpoint by [`crate::Grant::Device`].

use http::{StatusCode, Uri};
use private::Secret;
use serde::Deserialize;

use crate::{Client, Failed, failure};

#[derive(Deserialize)]
pub struct Device {
    pub device_code: Secret,
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds to wait between polls, five if the issuer names none (RFC 8628 3.2).
    pub interval: Option<u64>,
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
