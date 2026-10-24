use anyhow::{Result, ensure};

use crate::{Secret, hex};

pub const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

pub fn fill(bytes: &mut [u8]) -> Result<()> {
    Ok(getrandom::fill(bytes)?)
}

pub fn below(n: usize) -> Result<usize> {
    ensure!(n > 0, "nothing is below 0");
    let n = n as u64;
    let fair = u64::MAX - u64::MAX % n;
    loop {
        let mut draw = [0; 8];
        fill(&mut draw)?;
        let draw = u64::from_le_bytes(draw);
        if draw < fair {
            return Ok((draw % n) as usize);
        }
    }
}

/// Public text, such as a message id or a MIME boundary.
pub fn chars(alphabet: &[u8], len: usize) -> Result<String> {
    (0..len).map(|_| Ok(char::from(alphabet[below(alphabet.len())?]))).collect()
}

/// `bytes` random bytes as lowercase hex.
pub fn token(bytes: usize) -> Result<Secret> {
    let mut drawn = vec![0; bytes];
    fill(&mut drawn)?;
    Ok(Secret::new(hex::encode(&drawn)))
}
