//! The certificate credential: the app proves itself to Entra with a short JWT signed by a key
//! that never leaves the host, in the form Microsoft's certificate-credentials page gives.

use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;
use ring::digest::{SHA256, digest};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PSS_SHA256, RsaKeyPair};
use serde_json::json;

pub struct Credential {
    key: RsaKeyPair,
    /// Base64url SHA-256 of the certificate's DER, the header's `x5t#S256`.
    thumbprint: String,
    /// The certificate's DER, base64, as the app registration takes it.
    pub certificate: String,
}

fn pem(path: &Path, label: &str) -> Result<Vec<u8>> {
    let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let item = rustls_pemfile::read_one(&mut text.as_slice())?
        .with_context(|| format!("no PEM in {}", path.display()))?;
    Ok(match item {
        rustls_pemfile::Item::Pkcs8Key(key) => key.secret_pkcs8_der().to_vec(),
        rustls_pemfile::Item::X509Certificate(cert) => cert.to_vec(),
        _ => anyhow::bail!("{} holds no {label}", path.display()),
    })
}

impl Credential {
    pub fn read(key: &Path, certificate: &Path) -> Result<Credential> {
        let key = RsaKeyPair::from_pkcs8(&pem(key, "PKCS#8 key")?)
            .map_err(|e| anyhow::anyhow!("the key is not an RSA PKCS#8 key: {e}"))?;
        let der = pem(certificate, "certificate")?;
        let thumbprint = URL_SAFE_NO_PAD.encode(digest(&SHA256, &der));
        let certificate = base64::engine::general_purpose::STANDARD.encode(&der);
        Ok(Credential { key, thumbprint, certificate })
    }

    /// A client assertion for the token endpoint `audience`, good for five minutes.
    pub fn assertion(&self, client_id: &str, audience: &str, now: Timestamp) -> Result<String> {
        let header = json!({"alg": "PS256", "typ": "JWT", "x5t#S256": self.thumbprint});
        let at = now.as_second();
        let claims = json!({
            "aud": audience, "iss": client_id, "sub": client_id,
            "jti": private::random::token(16)?.expose(),
            "nbf": at, "iat": at, "exp": at + 300,
        });
        let part = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let signed = format!("{}.{}", part(&header), part(&claims));
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(&RSA_PSS_SHA256, &SystemRandom::new(), signed.as_bytes(), &mut signature)
            .map_err(|_| anyhow::anyhow!("signing the client assertion failed"))?;
        Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

#[cfg(test)]
#[path = "../tests/assertion.rs"]
mod tests;
