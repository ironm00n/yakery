use anyhow::{Context, Result, anyhow};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PSS_SHA256, RsaKeyPair};
use rustls_pki_types::CertificateDer;
use serde_json::json;
use webpki::EndEntityCert;

use crate::{Failed, base64url, sha256};

/// How long a client assertion is good for: Microsoft asks for 5 to 10 minutes past its `nbf` at
/// most, and enforces no bound.
const ASSERTION_FOR: i64 = 5 * 60;

pub struct Client {
    pub id: String,
    pub auth: Auth,
}

pub enum Auth {
    /// No secret: what it is asked for is bound by PKCE or the device code alone.
    Public,
    Assertion(Box<Assertion>),
}

impl Client {
    pub(crate) fn fields(
        &self,
        audience: &str,
        now: i64,
    ) -> Result<Vec<(&'static str, String)>, Failed> {
        let mut fields = vec![("client_id", self.id.clone())];
        if let Auth::Assertion(assertion) = &self.auth {
            let kind = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
            fields.push(("client_assertion_type", kind.to_owned()));
            fields.push(("client_assertion", assertion.signed(&self.id, audience, now)?));
        }
        Ok(fields)
    }
}

/// A key and the certificate the issuer holds of it, checked at construction to be one pair,
/// signing RFC 7523 client assertions as Microsoft takes them: PS256, naming the certificate by
/// its SHA-256 thumbprint.
pub struct Assertion {
    key: RsaKeyPair,
    certificate: Vec<u8>,
    thumbprint: String,
    random: SystemRandom,
}

impl Assertion {
    /// From the key in PKCS#8 DER and its certificate in DER.
    pub fn new(key: &[u8], certificate: &[u8]) -> Result<Assertion> {
        let key = RsaKeyPair::from_pkcs8(key).map_err(|e| anyhow!("the client key: {e}"))?;
        let random = SystemRandom::new();
        let held = CertificateDer::from(certificate);
        let cert = EndEntityCert::try_from(&held).context("the client certificate")?;
        let probe = b"oidc client key and certificate";
        let mut signed = vec![0; key.public().modulus_len()];
        key.sign(&RSA_PSS_SHA256, &random, probe, &mut signed)
            .map_err(|_| anyhow!("the client key signs nothing"))?;
        let alg = webpki::ring::RSA_PSS_2048_8192_SHA256_LEGACY_KEY;
        cert.verify_signature(alg, probe, &signed)
            .map_err(|_| anyhow!("the client certificate is of another key than the client key"))?;
        let thumbprint = base64url(sha256(certificate));
        Ok(Assertion { key, certificate: certificate.to_vec(), thumbprint, random })
    }

    /// The certificate, DER, as the issuer's app registration takes it.
    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }

    fn signed(&self, client: &str, audience: &str, now: i64) -> Result<String, Failed> {
        let random = || Failed::Unacceptable("no randomness for a client assertion".into());
        let jti = private::random::token(16).map_err(|_| random())?;
        let header = json!({"alg": "PS256", "typ": "JWT", "x5t#S256": self.thumbprint});
        let claims = json!({
            "aud": audience, "iss": client, "sub": client, "jti": jti.expose(),
            "nbf": now, "iat": now, "exp": now + ASSERTION_FOR,
        });
        let signing =
            format!("{}.{}", base64url(header.to_string()), base64url(claims.to_string()));
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(&RSA_PSS_SHA256, &self.random, signing.as_bytes(), &mut signature)
            .map_err(|_| Failed::Unacceptable("the client key signed nothing".into()))?;
        Ok(format!("{signing}.{}", base64url(signature)))
    }
}

#[cfg(test)]
#[path = "../tests/client.rs"]
mod tests;
