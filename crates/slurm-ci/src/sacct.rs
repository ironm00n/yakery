//! `sacct` as data. One query fetches the allocation row and the `.batch` step
//! and merges them: state and timestamps come from the allocation, the exit
//! code and utilisation from `.batch` (§3, §9 — `-X` zeroes the memory axis,
//! and `TIMEOUT` carries `ExitCode=0:0` on the allocation row).

use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const FIELDS: &str = "JobID,State,ExitCode,NodeList,Partition,Submit,Start,End,Elapsed,Timelimit,AllocCPUS,ReqMem,TotalCPU,MaxRSS";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ExitCode {
    pub code: i32,
    pub signal: i32,
}

impl FromStr for ExitCode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let (c, sig) = s
            .trim()
            .split_once(':')
            .with_context(|| format!("bad ExitCode {s:?}"))?;
        Ok(ExitCode {
            code: c.parse()?,
            signal: sig.parse()?,
        })
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.code, self.signal)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct SacctRecord {
    pub job_id: String,
    /// Leading word only: sacct prints `CANCELLED by 123`.
    pub state: String,
    pub alloc_exit: Option<ExitCode>,
    pub batch_exit: Option<ExitCode>,
    pub node_list: Option<String>,
    pub partition: Option<String>,
    pub submit: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub elapsed: Option<String>,
    pub timelimit: Option<String>,
    pub alloc_cpus: Option<u64>,
    pub req_mem: Option<String>,
    pub total_cpu: Option<String>,
    pub max_rss: Option<String>,
}

impl SacctRecord {
    /// Parse `sacct -n -P -o FIELDS` output for one job. `Ok(None)` when sacct
    /// has no rows yet — slurmdbd lands a beat after submit and after completion.
    pub fn parse(job_id: &str, out: &str) -> Result<Option<Self>> {
        let mut alloc = None;
        let mut batch = None;
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            let cols: Vec<&str> = line.split('|').collect();
            if cols.len() < 14 {
                bail!("unparseable sacct row {line:?}");
            }
            if cols[0] == job_id {
                alloc = Some(cols);
            } else if cols[0] == format!("{job_id}.batch") {
                batch = Some(cols);
            }
        }
        let Some(a) = alloc else { return Ok(None) };
        let opt =
            |s: &str| (!s.trim().is_empty() && s.trim() != "Unknown").then(|| s.trim().to_owned());
        let exit = |s: &str| opt(s).map(|s| s.parse::<ExitCode>()).transpose();
        let util = batch.as_ref().unwrap_or(&a);
        Ok(Some(SacctRecord {
            job_id: job_id.to_owned(),
            state: a[1].split_whitespace().next().unwrap_or("").to_owned(),
            alloc_exit: exit(a[2])?,
            batch_exit: batch.as_ref().map(|b| exit(b[2])).transpose()?.flatten(),
            node_list: opt(a[3]).filter(|n| n != "None assigned"),
            partition: opt(a[4]),
            submit: opt(a[5]),
            start: opt(a[6]),
            end: opt(a[7]),
            elapsed: opt(a[8]),
            timelimit: opt(a[9]),
            alloc_cpus: opt(a[10]).map(|s| s.parse()).transpose()?,
            req_mem: opt(a[11]),
            total_cpu: opt(util[12]),
            max_rss: opt(util[13]),
        }))
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(
            self.state.as_str(),
            "" | "PENDING"
                | "RUNNING"
                | "SUSPENDED"
                | "COMPLETING"
                | "CONFIGURING"
                | "REQUEUED"
                | "RESIZING"
                | "STAGE_OUT"
                | "SIGNALING"
                | "STOPPED"
        )
    }

    /// The wrapper's exit status, which is the verdict (§3): `.batch` when
    /// present, else the allocation row.
    pub fn wrapper_exit(&self) -> Option<ExitCode> {
        self.batch_exit.or(self.alloc_exit)
    }

    /// First node of the allocation, for `--exclude` on a gate retry.
    pub fn first_node(&self) -> Option<&str> {
        self.node_list.as_deref()?.split(',').next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: &str = "\
9442675|TIMEOUT|0:0|d3206|sharing|2026-08-21T22:01:10|2026-08-21T22:01:12|2026-08-21T23:01:30|01:00:18|01:00:00|128|126000M|4-06:52:31|
9442675.batch|CANCELLED|0:15|d3206||2026-08-21T22:01:12|2026-08-21T22:01:12|2026-08-21T23:01:30|01:00:18||128||4-06:52:31|183917204K
9442675.extern|COMPLETED|0:0|d3206||2026-08-21T22:01:10|2026-08-21T22:01:12|2026-08-21T23:01:30|01:00:18||128||00:00:00|0
";

    #[test]
    fn merges_alloc_and_batch() {
        let r = SacctRecord::parse("9442675", TIMEOUT).unwrap().unwrap();
        assert_eq!(r.state, "TIMEOUT");
        assert_eq!(r.alloc_exit, Some(ExitCode { code: 0, signal: 0 }));
        assert_eq!(
            r.batch_exit,
            Some(ExitCode {
                code: 0,
                signal: 15
            })
        );
        assert_eq!(r.max_rss.as_deref(), Some("183917204K"));
        assert_eq!(r.total_cpu.as_deref(), Some("4-06:52:31"));
        assert_eq!(r.req_mem.as_deref(), Some("126000M"));
        assert_eq!(r.alloc_cpus, Some(128));
        assert_eq!(r.first_node(), Some("d3206"));
        assert!(r.is_terminal());
    }

    #[test]
    fn pending_has_no_batch() {
        let out = "5|PENDING|0:0|None assigned|sharing|2026-09-13T10:00:00|Unknown|Unknown|00:00:00|01:00:00|32|65536M||\n";
        let r = SacctRecord::parse("5", out).unwrap().unwrap();
        assert_eq!(r.state, "PENDING");
        assert!(!r.is_terminal());
        assert_eq!(r.node_list, None);
        assert_eq!(r.start, None);
        assert_eq!(r.batch_exit, None);
        assert_eq!(r.wrapper_exit(), Some(ExitCode { code: 0, signal: 0 }));
    }

    #[test]
    fn cancelled_by_user() {
        let out = "5|CANCELLED by 1000|0:0|d1|short|a|b|c|d|e|1|1M||\n5.batch|CANCELLED|0:15|d1||a|b|c|d||1||x|y\n";
        let r = SacctRecord::parse("5", out).unwrap().unwrap();
        assert_eq!(r.state, "CANCELLED");
        assert_eq!(
            r.wrapper_exit(),
            Some(ExitCode {
                code: 0,
                signal: 15
            })
        );
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(SacctRecord::parse("5", "\n").unwrap(), None);
        assert_eq!(
            SacctRecord::parse("5", "6|COMPLETED|0:0|d1|p|a|b|c|d|e|1|1M|x|y\n").unwrap(),
            None
        );
    }
}
