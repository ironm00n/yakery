//! Uniform draws from the OS random source.

use anyhow::Result;

pub const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

#[derive(Default)]
pub struct Draws {
    pool: Vec<u8>,
}

impl Draws {
    fn byte(&mut self) -> Result<u8> {
        if self.pool.is_empty() {
            self.pool = vec![0; 64];
            getrandom::fill(&mut self.pool)?;
        }
        Ok(self.pool.pop().expect("the pool was just filled"))
    }

    /// Rejection sampling keeps the index uniform; `n` is at most 256.
    pub fn below(&mut self, n: usize) -> Result<usize> {
        assert!((1..=256).contains(&n), "draws are from one byte");
        let unbiased = 256 - 256 % n;
        loop {
            let b = usize::from(self.byte()?);
            if b < unbiased {
                return Ok(b % n);
            }
        }
    }

    pub fn pick(&mut self, set: &[u8]) -> Result<u8> {
        Ok(set[self.below(set.len())?])
    }

    pub fn shuffle(&mut self, items: &mut [u8]) -> Result<()> {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1)?);
        }
        Ok(())
    }

    pub fn string(&mut self, set: &[u8], len: usize) -> Result<String> {
        (0..len).map(|_| self.pick(set).map(char::from)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_covers_the_range_and_nothing_else() {
        let mut draws = Draws::default();
        let mut seen = [0usize; 7];
        for _ in 0..7_000 {
            seen[draws.below(7).unwrap()] += 1;
        }
        assert!(seen.iter().all(|&n| n > 700), "{seen:?}");
    }

    #[test]
    fn shuffle_keeps_the_items() {
        let mut items = *b"abcdefgh";
        Draws::default().shuffle(&mut items).unwrap();
        items.sort();
        assert_eq!(&items, b"abcdefgh");
    }
}
