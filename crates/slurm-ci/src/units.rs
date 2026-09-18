//! Canonical resource units. The repo toml accepts human forms; everything past
//! `run` carries integers (MiB, minutes) so `dispatch` compares numbers to caps
//! and never forwards a caller-provided Slurm string.

use std::fmt;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MiB(pub u64);

impl MiB {
    /// `64G`, `65536M`, `128000` (bare = MiB), case-insensitive, optional `iB`/`B`.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (num, unit) = s.split_at(digits_end);
        let n: u64 = num.parse().with_context(|| format!("bad mem {s:?}"))?;
        let unit = unit.trim().to_ascii_lowercase();
        let unit = unit
            .strip_suffix("ib")
            .or(unit.strip_suffix('b'))
            .unwrap_or(&unit);
        let mib = match unit {
            "" | "m" => n,
            "k" => n / 1024,
            "g" => n.checked_mul(1024).context("mem overflow")?,
            "t" => n.checked_mul(1024 * 1024).context("mem overflow")?,
            _ => bail!("bad mem unit in {s:?}"),
        };
        if mib == 0 {
            bail!("mem must be positive: {s:?}");
        }
        Ok(MiB(mib))
    }

    pub fn slurm(&self) -> String {
        format!("{}M", self.0)
    }
}

impl fmt::Display for MiB {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}MiB", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Minutes(pub u64);

impl Minutes {
    /// Slurm's forms (`45`, `1:30:00`, `1-00:00:00`, `H:MM`) plus `45m`/`2h`.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let err = || format!("bad time {s:?}");
        if let Some(m) = s.strip_suffix('m') {
            return Ok(Minutes(m.parse().with_context(err)?));
        }
        if let Some(h) = s.strip_suffix('h') {
            return Ok(Minutes(h.parse::<u64>().with_context(err)? * 60));
        }
        let (days, rest) = match s.split_once('-') {
            Some((d, r)) => (d.parse::<u64>().with_context(err)?, r),
            None => (0, s),
        };
        let parts: Vec<u64> = rest
            .split(':')
            .map(|p| p.parse::<u64>().with_context(err))
            .collect::<Result<_>>()?;
        let minutes = match (days > 0, parts.as_slice()) {
            (false, [m]) => *m,
            (true, [h]) => h * 60,
            (_, [h, m]) => h * 60 + m,
            (_, [h, m, sec]) => h * 60 + m + u64::from(*sec > 0),
            _ => bail!(err()),
        };
        let total = days * 24 * 60 + minutes;
        if total == 0 {
            bail!("time must be positive: {s:?}");
        }
        Ok(Minutes(total))
    }

    /// `sinfo %l` output: Slurm time or `infinite`/`UNLIMITED`.
    pub fn parse_sinfo(s: &str) -> Result<Option<Self>> {
        match s.trim().to_ascii_lowercase().as_str() {
            "infinite" | "unlimited" | "n/a" => Ok(None),
            _ => Self::parse(s).map(Some),
        }
    }
}

impl fmt::Display for Minutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}min", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem() {
        assert_eq!(MiB::parse("64G").unwrap(), MiB(65536));
        assert_eq!(MiB::parse("64GiB").unwrap(), MiB(65536));
        assert_eq!(MiB::parse("1500M").unwrap(), MiB(1500));
        assert_eq!(MiB::parse("1500").unwrap(), MiB(1500));
        assert_eq!(MiB::parse("2048k").unwrap(), MiB(2));
        assert!(MiB::parse("0").is_err());
        assert!(MiB::parse("64X").is_err());
        assert!(MiB::parse("-5G").is_err());
        assert_eq!(MiB(65536).slurm(), "65536M");
    }

    #[test]
    fn time() {
        assert_eq!(Minutes::parse("45").unwrap(), Minutes(45));
        assert_eq!(Minutes::parse("45m").unwrap(), Minutes(45));
        assert_eq!(Minutes::parse("2h").unwrap(), Minutes(120));
        assert_eq!(Minutes::parse("1:30").unwrap(), Minutes(90));
        assert_eq!(Minutes::parse("1:30:00").unwrap(), Minutes(90));
        assert_eq!(Minutes::parse("0:00:30").unwrap(), Minutes(1));
        assert_eq!(Minutes::parse("1-00:00:00").unwrap(), Minutes(1440));
        assert_eq!(Minutes::parse("1-2").unwrap(), Minutes(1440 + 120));
        assert!(Minutes::parse("0").is_err());
        assert!(Minutes::parse("1:2:3:4").is_err());
        assert_eq!(Minutes::parse_sinfo("infinite").unwrap(), None);
        assert_eq!(Minutes::parse_sinfo("UNLIMITED").unwrap(), None);
        assert_eq!(Minutes::parse_sinfo("1:00:00").unwrap(), Some(Minutes(60)));
    }
}
