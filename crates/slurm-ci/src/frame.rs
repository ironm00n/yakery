//! Length-prefixed framing shared by the callback (TLS) and the run↔daemon
//! socket: `kind:u8 len:u32be payload`. Bounded so a peer can cost at most
//! one buffer.

use std::io::{self, Read, Write};

use anyhow::{bail, Result};

pub const MAX_PAYLOAD: usize = 1 << 20;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(kind: u8, payload: impl Into<Vec<u8>>) -> Self {
        Frame {
            kind,
            payload: payload.into(),
        }
    }

    pub fn write_to(&self, w: &mut impl Write) -> Result<()> {
        if self.payload.len() > MAX_PAYLOAD {
            bail!("frame payload {} exceeds {MAX_PAYLOAD}", self.payload.len());
        }
        let mut buf = Vec::with_capacity(5 + self.payload.len());
        buf.push(self.kind);
        buf.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(&self.payload);
        w.write_all(&buf)?;
        w.flush()?;
        Ok(())
    }

    /// `Ok(None)` on a clean EOF between frames; an EOF mid-frame is an error.
    pub fn read_from(r: &mut impl Read) -> Result<Option<Frame>> {
        let mut header = [0u8; 5];
        match r.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len > MAX_PAYLOAD {
            bail!("frame payload {len} exceeds {MAX_PAYLOAD}");
        }
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload)?;
        Ok(Some(Frame {
            kind: header[0],
            payload,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut buf = Vec::new();
        Frame::new(7, b"hello".to_vec()).write_to(&mut buf).unwrap();
        Frame::new(1, Vec::new()).write_to(&mut buf).unwrap();
        let mut r = buf.as_slice();
        assert_eq!(
            Frame::read_from(&mut r).unwrap(),
            Some(Frame::new(7, b"hello".to_vec()))
        );
        assert_eq!(
            Frame::read_from(&mut r).unwrap(),
            Some(Frame::new(1, Vec::new()))
        );
        assert_eq!(Frame::read_from(&mut r).unwrap(), None);
    }

    #[test]
    fn truncated_is_error() {
        let mut buf = Vec::new();
        Frame::new(7, b"hello".to_vec()).write_to(&mut buf).unwrap();
        buf.truncate(7);
        assert!(Frame::read_from(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn oversize_is_error() {
        let header = [1u8, 0xff, 0xff, 0xff, 0xff];
        assert!(Frame::read_from(&mut header.as_slice()).is_err());
        assert!(Frame::new(1, vec![0u8; MAX_PAYLOAD + 1])
            .write_to(&mut Vec::new())
            .is_err());
    }
}
