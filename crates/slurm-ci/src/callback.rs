//! The compute → VPS callback (§3): framed messages over TLS to a pinned
//! self-signed cert. `HELLO` claims a run with its single-use token; `DATA`
//! streams the log; `EXIT` carries the wrapper's exit status. Best-effort from
//! the build's side — a lost connection never stalls or fails a build.

use std::fs;
use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, ServerConfig, SignatureScheme,
    StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::frame::Frame;
use crate::ids::{hex, RunId, Token};
use crate::verbs::JobId;

pub const PORT: u16 = 443;
/// Well under common NAT idle timeouts (~300s).
pub const HEARTBEAT: Duration = Duration::from_secs(60);
/// A connection without a valid `HELLO` inside this window costs one socket.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub run: RunId,
    pub token: Token,
    /// `$SLURM_JOB_ID`: re-attaches a run whose submit reply was lost.
    pub job: Option<JobId>,
    pub node: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Msg {
    Hello(Hello),
    Data(Vec<u8>),
    Heartbeat,
    Exit(i32),
}

mod kind {
    pub const HELLO: u8 = 1;
    pub const DATA: u8 = 2;
    pub const HEARTBEAT: u8 = 3;
    pub const EXIT: u8 = 4;
}

impl Msg {
    pub fn to_frame(&self) -> Frame {
        match self {
            Msg::Hello(h) => Frame::new(
                kind::HELLO,
                serde_json::to_vec(h).expect("hello serialises"),
            ),
            Msg::Data(d) => Frame::new(kind::DATA, d.clone()),
            Msg::Heartbeat => Frame::new(kind::HEARTBEAT, Vec::new()),
            Msg::Exit(c) => Frame::new(kind::EXIT, c.to_be_bytes().to_vec()),
        }
    }

    pub fn from_frame(f: Frame) -> Result<Msg> {
        Ok(match f.kind {
            kind::HELLO => Msg::Hello(serde_json::from_slice(&f.payload).context("bad HELLO")?),
            kind::DATA => Msg::Data(f.payload),
            kind::HEARTBEAT => Msg::Heartbeat,
            kind::EXIT => {
                let b: [u8; 4] = f.payload.as_slice().try_into().context("bad EXIT")?;
                Msg::Exit(i32::from_be_bytes(b))
            }
            k => bail!("unknown callback frame kind {k}"),
        })
    }
}

pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    hex(&Sha256::digest(cert.as_ref()))
}

fn provider() -> Arc<CryptoProvider> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    CryptoProvider::get_default()
        .expect("provider installed")
        .clone()
}

/// The listener's self-signed identity, persisted so a restart keeps the
/// fingerprint that in-flight runs captured at submit.
pub struct Identity {
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    pub fingerprint: String,
}

impl Identity {
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
        if !cert_path.exists() || !key_path.exists() {
            let ck = rcgen::generate_simple_self_signed(vec!["slurm-ci".to_owned()])?;
            fs::write(&cert_path, ck.cert.pem())?;
            write_private(&key_path, ck.key_pair.serialize_pem().as_bytes())?;
        }
        let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(fs::File::open(&cert_path)?))
            .collect::<std::result::Result<_, _>>()
            .context("parse cert.pem")?;
        let key = rustls_pemfile::private_key(&mut BufReader::new(fs::File::open(&key_path)?))
            .context("parse key.pem")?
            .context("key.pem holds no private key")?;
        let fingerprint = fingerprint(certs.first().context("cert.pem is empty")?);
        Ok(Identity {
            certs,
            key,
            fingerprint,
        })
    }

    pub fn server_config(&self) -> Result<Arc<ServerConfig>> {
        let mut cfg = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), self.key.clone_key())?;
        // The build side only ever writes. Anything the server sends after the
        // handshake (session tickets) would sit unread in the client's receive
        // queue, and closing a socket with unread data sends RST, which makes
        // the server discard whatever it had not yet read — the EXIT frame.
        cfg.send_tls13_tickets = 0;
        Ok(Arc::new(cfg))
    }
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?
        .write_all(data)?;
    Ok(())
}

#[derive(Debug)]
struct Pinned {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "callback cert fingerprint mismatch".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub type ClientStream = StreamOwned<ClientConnection, TcpStream>;
pub type ServerStream = StreamOwned<rustls::ServerConnection, TcpStream>;

/// `host[:port]`; the port defaults to 443, the one measured egress (§10).
pub fn split_host_port(s: &str) -> (&str, u16) {
    match s.rsplit_once(':') {
        Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty() => {
            (h, p.parse().unwrap_or(PORT))
        }
        _ => (s, PORT),
    }
}

pub fn connect(host: &str, port: u16, fingerprint: &str) -> Result<ClientStream> {
    let provider = provider();
    let mut cfg = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned {
            fingerprint: fingerprint.to_owned(),
            provider,
        }))
        .with_no_client_auth();
    cfg.resumption = rustls::client::Resumption::disabled();
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))?
        .next()
        .with_context(|| format!("resolve {host}"))?;
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .with_context(|| format!("connect {addr}"))?;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let name =
        ServerName::try_from(host.to_owned()).with_context(|| format!("server name {host}"))?;
    let conn = ClientConnection::new(Arc::new(cfg), name)?;
    let mut stream = StreamOwned::new(conn, tcp);
    // Complete the handshake now so a pin mismatch surfaces at the gate.
    stream
        .conn
        .complete_io(&mut stream.sock)
        .context("TLS handshake")?;
    Ok(stream)
}

/// Shared handle for the build's relay and heartbeat threads. Drops the
/// connection on the first write error and stays silent thereafter.
pub struct Client {
    stream: Mutex<Option<ClientStream>>,
}

impl Client {
    pub fn new(stream: ClientStream) -> Arc<Self> {
        Arc::new(Client {
            stream: Mutex::new(Some(stream)),
        })
    }

    pub fn send(&self, msg: &Msg) -> bool {
        let mut guard = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stream) = guard.as_mut() else {
            return false;
        };
        if let Err(e) = msg.to_frame().write_to(stream) {
            eprintln!("callback dropped: {e:#}");
            *guard = None;
            return false;
        }
        true
    }

    pub fn close(&self) {
        let mut guard = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = guard.take() {
            close(s);
        }
    }
}

/// Orderly close: send close_notify, then drain until the peer's EOF so the
/// socket never closes with unread data (which would RST and lose frames the
/// peer has not read yet).
pub fn close(mut s: ClientStream) {
    s.conn.send_close_notify();
    let _ = s.conn.complete_io(&mut s.sock);
    let _ = s.sock.set_read_timeout(Some(Duration::from_secs(5)));
    let mut sink = [0u8; 1024];
    while matches!(std::io::Read::read(&mut s, &mut sink), Ok(n) if n > 0) {}
}

/// Server side of one accepted connection: TLS, then the first frame must be
/// a `HELLO` within `HELLO_TIMEOUT`.
pub fn accept(cfg: Arc<ServerConfig>, tcp: TcpStream) -> Result<(Hello, ServerStream)> {
    tcp.set_read_timeout(Some(HELLO_TIMEOUT))?;
    tcp.set_write_timeout(Some(HELLO_TIMEOUT))?;
    let conn = rustls::ServerConnection::new(cfg)?;
    let mut stream = StreamOwned::new(conn, tcp);
    let frame = Frame::read_from(&mut stream)?.context("closed before HELLO")?;
    match Msg::from_frame(frame)? {
        Msg::Hello(h) => Ok((h, stream)),
        other => bail!("first frame was not HELLO: {other:?}"),
    }
}

/// After `HELLO` the read side relaxes to the heartbeat cadence with slack.
pub fn read_next(stream: &mut ServerStream) -> Result<Option<Msg>> {
    stream.sock.set_read_timeout(Some(HEARTBEAT * 3))?;
    Frame::read_from(stream)?.map(Msg::from_frame).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let hello = Hello {
            run: RunId::random().unwrap(),
            token: Token::random().unwrap(),
            job: Some(JobId::parse("42").unwrap()),
            node: "d0020".into(),
        };
        for m in [
            Msg::Hello(hello),
            Msg::Data(b"log line\n".to_vec()),
            Msg::Heartbeat,
            Msg::Exit(-1),
            Msg::Exit(76),
        ] {
            assert_eq!(Msg::from_frame(m.to_frame()).unwrap(), m);
        }
        assert!(Msg::from_frame(Frame::new(9, Vec::new())).is_err());
        assert!(Msg::from_frame(Frame::new(kind::EXIT, vec![1, 2])).is_err());
    }

    #[test]
    fn host_port() {
        assert_eq!(split_host_port("vps.example"), ("vps.example", 443));
        assert_eq!(split_host_port("vps.example:8443"), ("vps.example", 8443));
        assert_eq!(split_host_port("10.0.0.1:1"), ("10.0.0.1", 1));
    }

    #[test]
    fn identity_persists_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir.path()).unwrap();
        let b = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 64);
        a.server_config().unwrap();
    }

    #[test]
    fn pinned_handshake() {
        let dir = tempfile::tempdir().unwrap();
        let id = Identity::load_or_create(dir.path()).unwrap();
        let cfg = id.server_config().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let (hello, mut stream) = accept(cfg, tcp).unwrap();
            let next = read_next(&mut stream).unwrap();
            (hello, next)
        });
        let hello = Hello {
            run: RunId::random().unwrap(),
            token: Token::random().unwrap(),
            job: None,
            node: "n".into(),
        };
        let mut stream = connect("127.0.0.1", port, &id.fingerprint).unwrap();
        Msg::Hello(hello.clone())
            .to_frame()
            .write_to(&mut stream)
            .unwrap();
        Msg::Exit(0).to_frame().write_to(&mut stream).unwrap();
        close(stream);
        let (got, next) = server.join().unwrap();
        assert_eq!(got, hello);
        assert_eq!(next, Some(Msg::Exit(0)));

        let bad = "00".repeat(32);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let cfg = id.server_config().unwrap();
        std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let _ = accept(cfg, tcp);
        });
        assert!(connect("127.0.0.1", port, &bad).is_err());
    }
}
