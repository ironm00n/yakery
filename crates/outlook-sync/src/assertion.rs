//! The key and certificate the sync proves itself to Entra with, from the state directory's PEM
//! files.

use anyhow::{Context, Result};
use oidc::Assertion;
use private::Dir;

use crate::state;

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

pub fn read(state: &Dir) -> Result<Assertion> {
    let key = pem(state, state::KEY, "PKCS#8 key")?;
    Assertion::new(&key, &pem(state, state::CERTIFICATE, "certificate")?)
}

#[cfg(test)]
#[path = "../tests/assertion.rs"]
mod tests;
