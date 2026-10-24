use std::time::{SystemTime, UNIX_EPOCH};

mod dir;
mod listen;
pub mod random;
mod secret;

pub use dir::{Dir, Mode, PARTIAL_EXPIRY, Replace};
pub use listen::listen_fds;
pub use secret::Secret;

pub mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        base16ct::lower::encode_string(bytes)
    }

    /// Never echoes `text`, which may be a secret.
    pub fn decode(text: &str) -> anyhow::Result<Vec<u8>> {
        base16ct::mixed::decode_vec(text).map_err(|e| anyhow::anyhow!("not hex: {e}"))
    }
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests;
