//! What the host and the VM's root daemon say over vsock: a control connection carries one [`Request`], then
//! [`Reply`] lines. Any process on either side can reach the other's ports or bind a free one to pose as a service,
//! so each end proves it holds the [`Key`] first ([`prove`], [`challenge`]). The gateway, in workspace/claude, is
//! the acceptor of [`Service::Gateway`] and [`Service::Ca`].

use std::{collections::HashMap, io::Read, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
    Lines, ReadHalf,
};
use tokio_vsock::{VsockAddr, VsockStream};
use yakery_log::warn;

pub const HOST_CID: u32 = 2;
pub const CONTROL_PORT: u32 = 6000;

pub type Key = [u8; blake3::KEY_LEN];
pub const NONCE_LEN: usize = 32;
/// What a control connection claims; a gateway connection claims `uid <n>`.
pub const WAKE: &str = "wake";
pub const CA: &str = "ca";
const MAX_PROOF: usize = "uid 4294967295 ".len() + 2 * blake3::OUT_LEN + 1;
const HANDSHAKE: Duration = Duration::from_secs(10);
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug)]
pub enum Service {
    Control,
    Gateway,
    Ca,
}

impl Service {
    /// Distinct per service, so that one service's answer can't be passed off as another's.
    fn accepted(self) -> &'static str {
        match self {
            Service::Control => "accepted control",
            Service::Gateway => "accepted gateway",
            Service::Ca => "accepted ca",
        }
    }
}

/// The first bytes of `path`, which is a file on the host and a block device in the VM.
pub fn read_key(path: &Path) -> Result<Key> {
    let mut key = [0; blake3::KEY_LEN];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut key))
        .with_context(|| format!("read the key from {}", path.display()))?;
    Ok(key)
}

pub fn read_agents(path: &Path) -> Result<HashMap<String, u32>> {
    let read = std::fs::read(path).map_err(anyhow::Error::from);
    read.and_then(|json| Ok(serde_json::from_slice(&json)?))
        .with_context(|| format!("read {}", path.display()))
}

pub fn tag(key: &Key, nonce: &[u8; NONCE_LEN], claim: &str) -> blake3::Hash {
    blake3::Hasher::new_keyed(key).update(nonce).update(claim.as_bytes()).finalize()
}

fn nonce() -> Result<[u8; NONCE_LEN]> {
    let mut nonce = [0; NONCE_LEN];
    private::random::fill(&mut nonce)?;
    Ok(nonce)
}
pub async fn prove(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    key: &Key,
    service: Service,
    claim: &str,
) -> Result<()> {
    let handshake = async {
        let ours = nonce()?;
        stream.write_all(&ours).await?;
        let mut theirs = [0; NONCE_LEN];
        stream.read_exact(&mut theirs).await?;
        let mut accepted = [0; blake3::OUT_LEN];
        stream.read_exact(&mut accepted).await?;
        if blake3::Hash::from(accepted) != tag(key, &ours, service.accepted()) {
            bail!("the {service:?} end does not hold the key");
        }
        let proof = format!("{claim} {}\n", tag(key, &theirs, claim).to_hex());
        stream.write_all(proof.as_bytes()).await?;
        Ok(())
    };
    tokio::time::timeout(HANDSHAKE, handshake).await.context("the handshake timed out")?
}

/// The accepting end of `service`: proves it holds the key, then returns the
/// claim the opener proves, consuming nothing past its line.
pub async fn challenge(
    stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    key: &Key,
    service: Service,
) -> Result<String> {
    let handshake = async {
        let mut theirs = [0; NONCE_LEN];
        stream.read_exact(&mut theirs).await?;
        let ours = nonce()?;
        stream.write_all(&ours).await?;
        stream.write_all(tag(key, &theirs, service.accepted()).as_bytes()).await?;
        let mut line = Vec::with_capacity(MAX_PROOF);
        (&mut *stream).take(MAX_PROOF as u64).read_until(b'\n', &mut line).await?;
        let (claim, hex) = std::str::from_utf8(&line)?
            .strip_suffix('\n')
            .and_then(|line| line.rsplit_once(' '))
            .context("malformed proof")?;
        if claim.starts_with("accepted ") {
            bail!("an opener claimed {claim:?}");
        }
        if blake3::Hash::from_hex(hex)? != tag(key, &ours, claim) {
            bail!("{claim:?} was not proved with the key");
        }
        Ok(claim.to_owned())
    };
    tokio::time::timeout(HANDSHAKE, handshake).await.context("the handshake timed out")?
}

/// The next connection `accepting` takes; the pause after an error, such as running out of descriptors, keeps it
/// from spinning.
pub async fn accept<T>(mut accepting: impl AsyncFnMut() -> std::io::Result<T>) -> T {
    loop {
        match accepting().await {
            Ok(accepted) => return accepted,
            Err(e) => {
                warn!("accept: {e}");
                tokio::time::sleep(ACCEPT_PAUSE).await;
            }
        }
    }
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
    Refused { reason: String },
    Event(Box<RawValue>),
    Exited { code: Option<i32> },
}

impl Reply {
    pub fn line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("replies serialize");
        line.push('\n');
        line
    }
}

pub async fn wake(
    cid: u32,
    key: &Key,
    request: &Request,
) -> Result<Lines<BufReader<ReadHalf<VsockStream>>>> {
    let mut stream = VsockStream::connect(VsockAddr::new(cid, CONTROL_PORT)).await?;
    prove(&mut stream, key, Service::Control, WAKE).await?;
    let (read, mut write) = tokio::io::split(stream);
    write.write_all(format!("{}\n", serde_json::to_string(request)?).as_bytes()).await?;
    Ok(BufReader::new(read).lines())
}

#[cfg(test)]
mod tests {
    use tokio::io::{BufReader, duplex};

    use super::*;

    const KEY: Key = [7; blake3::KEY_LEN];
    const OTHER: Key = [8; blake3::KEY_LEN];

    /// The gateway checks the same tags in another repository and pins these
    /// values too, which b3sum gives for this key and nonce.
    #[test]
    fn tags_are_keyed_blake3_of_nonce_then_claim() {
        assert_eq!(
            tag(&KEY, &[9; NONCE_LEN], "uid 2001").to_hex().as_str(),
            "0b346bc239a5621e087194715abb7698e6e72161480cbb0a962272c42ed80984",
        );
        assert_eq!(
            tag(&KEY, &[9; NONCE_LEN], Service::Gateway.accepted()).to_hex().as_str(),
            "9a86f6875380db50f0168a833724b6b4441386f2056d9cceedfaa79f5862e980",
        );
    }

    async fn exchange(
        opener: Key,
        dialled: Service,
        acceptor: Key,
        is: Service,
        claim: &str,
    ) -> (Result<()>, Result<String>) {
        let (accepting, mut opening) = duplex(1024);
        let mut accepting = BufReader::new(accepting);
        tokio::join!(
            async move { prove(&mut opening, &opener, dialled, claim).await },
            async move { challenge(&mut accepting, &acceptor, is).await }
        )
    }

    #[tokio::test]
    async fn both_ends_holding_the_key_agree_on_the_claim() {
        let (proved, claim) = exchange(KEY, Service::Control, KEY, Service::Control, WAKE).await;
        proved.unwrap();
        assert_eq!(claim.unwrap(), WAKE);
    }

    #[tokio::test]
    async fn an_opener_without_the_key_is_refused() {
        let (_, claim) = exchange(OTHER, Service::Control, KEY, Service::Control, WAKE).await;
        assert!(claim.is_err());
    }

    #[tokio::test]
    async fn an_acceptor_without_the_key_is_refused() {
        let (proved, _) =
            exchange(KEY, Service::Gateway, OTHER, Service::Gateway, "uid 2001").await;
        assert!(proved.is_err());
    }

    #[tokio::test]
    async fn an_acceptor_of_another_service_is_refused() {
        let (proved, _) = exchange(KEY, Service::Gateway, KEY, Service::Control, "uid 2001").await;
        assert!(proved.is_err());
    }

    #[tokio::test]
    async fn an_opener_claiming_to_be_an_acceptor_is_refused() {
        let (_, claim) =
            exchange(KEY, Service::Control, KEY, Service::Control, "accepted control").await;
        assert!(claim.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_peer_times_out() {
        let (accepting, _opening) = duplex(1024);
        let mut accepting = BufReader::new(accepting);
        assert!(challenge(&mut accepting, &KEY, Service::Control).await.is_err());
    }
}
