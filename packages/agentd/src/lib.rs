//! What the host and the VM's root daemon say to each other over vsock. A
//! control connection carries one [`Request`], then [`Reply`] lines until the
//! daemon closes it.
//!
//! Every process on the host, and anything in the VM that gets past agentd's
//! socket filters, can reach the other side's vsock ports, so a connection
//! counts only once its opener proves it holds the [`Key`]: the accepting end
//! sends a nonce, the opener answers `<claim> <tag>\n` ([`prove`],
//! [`challenge`]). The gateway checks the same proof of a `uid <n>` claim.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};

pub const HOST_CID: u32 = 2;
pub const CONTROL_PORT: u32 = 6000;

/// Shared by the host and the VM's root daemon, and readable by nothing else in
/// the VM.
pub type Key = [u8; blake3::KEY_LEN];
pub const NONCE_LEN: usize = 32;
/// What a control connection claims; a gateway connection claims `uid <n>`.
pub const WAKE: &str = "wake";
const MAX_PROOF: usize = "uid 4294967295 ".len() + 2 * blake3::OUT_LEN + 1;

/// The key's first bytes of `path`, a file on the host and a block device in
/// the VM.
pub fn read_key(path: &Path) -> Result<Key> {
    let mut key = [0; blake3::KEY_LEN];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut key))
        .with_context(|| format!("read the key from {}", path.display()))?;
    Ok(key)
}

pub fn tag(key: &Key, nonce: &[u8; NONCE_LEN], claim: &str) -> blake3::Hash {
    blake3::Hasher::new_keyed(key)
        .update(nonce)
        .update(claim.as_bytes())
        .finalize()
}

/// The opening end: answers the nonce with `claim` and its tag.
pub async fn prove(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    key: &Key,
    claim: &str,
) -> Result<()> {
    let mut nonce = [0; NONCE_LEN];
    stream
        .read_exact(&mut nonce)
        .await
        .context("read the nonce")?;
    let proof = format!("{claim} {}\n", tag(key, &nonce, claim).to_hex());
    stream.write_all(proof.as_bytes()).await?;
    Ok(())
}

/// The accepting end: sends a fresh nonce and returns the claim the answer
/// proves, consuming nothing past its line.
pub async fn challenge(
    stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    key: &Key,
) -> Result<String> {
    let mut nonce = [0; NONCE_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    stream.write_all(&nonce).await?;
    let mut line = Vec::with_capacity(MAX_PROOF);
    (&mut *stream)
        .take(MAX_PROOF as u64)
        .read_until(b'\n', &mut line)
        .await?;
    let (claim, hex) = std::str::from_utf8(&line)?
        .strip_suffix('\n')
        .and_then(|line| line.rsplit_once(' '))
        .context("malformed proof")?;
    if blake3::Hash::from_hex(hex)? != tag(key, &nonce, claim) {
        bail!("{claim:?} was not proved with the key");
    }
    Ok(claim.to_owned())
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Hand `message` to the agent: a new session if it has none running, or
    /// a `next` user message into the one that is.
    Wake { agent: String, message: String },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Started,
    Injected,
    Refused {
        reason: String,
    },
    /// One stream-json line from the session, as the client wrote it.
    Event(Box<RawValue>),
    Exited {
        code: Option<i32>,
    },
}

impl Reply {
    pub fn line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("replies serialize");
        line.push('\n');
        line
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{BufReader, duplex};

    use super::*;

    const KEY: Key = [7; blake3::KEY_LEN];

    /// The gateway checks the same tag in another repository and pins this value
    /// too, which b3sum gives for this key, nonce and claim.
    #[test]
    fn the_tag_is_keyed_blake3_of_nonce_then_claim() {
        assert_eq!(
            tag(&KEY, &[9; NONCE_LEN], "uid 2001").to_hex().as_str(),
            "0b346bc239a5621e087194715abb7698e6e72161480cbb0a962272c42ed80984",
        );
    }

    async fn exchange(prover: Key, claim: &str) -> Result<String> {
        let (accepting, mut opening) = duplex(1024);
        let mut accepting = BufReader::new(accepting);
        let (proved, checked) = tokio::join!(
            prove(&mut opening, &prover, claim),
            challenge(&mut accepting, &KEY)
        );
        proved?;
        checked
    }

    #[tokio::test]
    async fn a_proof_with_the_key_is_accepted() {
        assert_eq!(exchange(KEY, WAKE).await.unwrap(), WAKE);
        assert_eq!(exchange(KEY, "uid 2001").await.unwrap(), "uid 2001");
    }

    #[tokio::test]
    async fn a_proof_without_the_key_is_refused() {
        assert!(exchange([8; blake3::KEY_LEN], WAKE).await.is_err());
    }
}
