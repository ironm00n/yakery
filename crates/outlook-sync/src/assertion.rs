//! The client assertion as Microsoft's certificate-credentials page gives it.

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;
use private::Dir;
use ring::digest::{SHA256, digest};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PSS_SHA256, RsaKeyPair};
use serde_json::json;

use crate::state;

pub struct Credential {
    key: RsaKeyPair,
    thumbprint: String,
    pub certificate: String,
}

fn pem(dir: &Dir, name: &str, label: &str) -> Result<Vec<u8>> {
    let shown = || dir.path(name).display().to_string();
    let text = dir.read(name)?.with_context(|| format!("{} is missing", shown()))?;
    let item = rustls_pemfile::read_one(&mut text.as_slice())?
        .with_context(|| format!("no PEM in {}", shown()))?;
    Ok(match item {
        rustls_pemfile::Item::Pkcs8Key(key) => key.secret_pkcs8_der().to_vec(),
        rustls_pemfile::Item::X509Certificate(cert) => cert.to_vec(),
        _ => anyhow::bail!("{} holds no {label}", shown()),
    })
}

impl Credential {
    pub fn read(state: &Dir) -> Result<Credential> {
        let key = RsaKeyPair::from_pkcs8(&pem(state, state::KEY, "PKCS#8 key")?)
            .map_err(|e| anyhow::anyhow!("the key is not an RSA PKCS#8 key: {e}"))?;
        let der = pem(state, state::CERTIFICATE, "certificate")?;
        let thumbprint = URL_SAFE_NO_PAD.encode(digest(&SHA256, &der));
        let certificate = base64::engine::general_purpose::STANDARD.encode(&der);
        Ok(Credential { key, thumbprint, certificate })
    }

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
