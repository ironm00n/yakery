//! The wrapper's exit status *is* the verdict (§3). One place maps exit codes
//! to outcomes so the callback path and the sacct fallback agree by
//! construction.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::sacct::SacctRecord;

/// Distinguished wrapper exit codes. Kept clear of nix's own (1, 100–104).
pub mod exit {
    /// Node sanity gate refused the node: infrastructure retry, not CI red.
    pub const GATE_FAILED: i32 = 75;
    /// Build succeeded, cache push did not: green with a warning.
    pub const PUSH_FAILED: i32 = 76;
    /// Store tamper detected before push: red, never a warning.
    pub const TAMPER: i32 = 77;
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Verdict {
    Green,
    GreenPushFailed,
    Red(String),
    /// Absent or conflicting evidence: reported red, distinguishable in the log.
    Inconclusive(String),
}

impl Verdict {
    pub fn from_exit(code: i32) -> Self {
        match code {
            0 => Verdict::Green,
            exit::PUSH_FAILED => Verdict::GreenPushFailed,
            exit::TAMPER => Verdict::Red("store tamper detected before push".into()),
            exit::GATE_FAILED => Verdict::Red("node sanity gate failed on every attempt".into()),
            c => Verdict::Red(format!("build exited {c}")),
        }
    }

    /// Fallback verdict over the trusted SSH path. `State` decides whether
    /// `ExitCode` means anything; `ExitCode` then supplies the verdict.
    pub fn from_sacct(rec: &SacctRecord) -> Option<Self> {
        if !rec.is_terminal() {
            return None;
        }
        let exit = rec.wrapper_exit();
        Some(match rec.state.as_str() {
            "COMPLETED" => Verdict::Green,
            "FAILED" => match exit {
                Some(e) if e.signal == 0 => Verdict::from_exit(e.code),
                Some(e) => Verdict::Red(format!("build killed by signal {}", e.signal)),
                None => Verdict::Inconclusive("FAILED with no exit code".into()),
            },
            s => Verdict::Red(format!(
                "Slurm state {s} ({})",
                exit.map_or("no exit code".to_owned(), |e| e.to_string())
            )),
        })
    }

    pub fn is_green(&self) -> bool {
        matches!(self, Verdict::Green | Verdict::GreenPushFailed)
    }

    pub fn exit_status(&self) -> i32 {
        if self.is_green() {
            0
        } else {
            1
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Green => f.write_str("green"),
            Verdict::GreenPushFailed => {
                f.write_str("green (cache push failed; outputs not cached)")
            }
            Verdict::Red(why) => write!(f, "red: {why}"),
            Verdict::Inconclusive(why) => write!(f, "inconclusive (reported red): {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sacct::ExitCode;

    fn rec(state: &str, alloc: &str, batch: Option<&str>) -> SacctRecord {
        SacctRecord {
            job_id: "1".into(),
            state: state.into(),
            alloc_exit: Some(alloc.parse().unwrap()),
            batch_exit: batch.map(|b| b.parse::<ExitCode>().unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn sacct_rule_is_a_conjunction() {
        assert_eq!(
            Verdict::from_sacct(&rec("COMPLETED", "0:0", Some("0:0"))),
            Some(Verdict::Green)
        );
        assert_eq!(
            Verdict::from_sacct(&rec("FAILED", "76:0", Some("76:0"))),
            Some(Verdict::GreenPushFailed)
        );
        assert!(!Verdict::from_sacct(&rec("FAILED", "1:0", Some("1:0")))
            .unwrap()
            .is_green());
        assert!(!Verdict::from_sacct(&rec("FAILED", "77:0", Some("77:0")))
            .unwrap()
            .is_green());
        // job 9442675: the allocation row's 0:0 must not read green
        assert!(!Verdict::from_sacct(&rec("TIMEOUT", "0:0", Some("0:15")))
            .unwrap()
            .is_green());
        assert!(!Verdict::from_sacct(&rec("CANCELLED", "0:0", Some("0:15")))
            .unwrap()
            .is_green());
        assert_eq!(Verdict::from_sacct(&rec("RUNNING", "0:0", None)), None);
        assert_eq!(Verdict::from_sacct(&rec("PENDING", "0:0", None)), None);
    }

    #[test]
    fn callback_and_sacct_agree() {
        for code in [0, 1, 75, 76, 77, 100] {
            let via_sacct = Verdict::from_sacct(&rec(
                if code == 0 { "COMPLETED" } else { "FAILED" },
                &format!("{code}:0"),
                Some(&format!("{code}:0")),
            ))
            .unwrap();
            assert_eq!(via_sacct, Verdict::from_exit(code), "code {code}");
        }
    }
}
