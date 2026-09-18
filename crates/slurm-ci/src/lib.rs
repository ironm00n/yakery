//! `slurm-ci` — Forgejo Actions CI on the Explorer Slurm cluster. One static
//! binary, every role; `DESIGN.md` is the spec.
//!
//! VPS:      `listen` (daemon: port 443, registry, SSH key) and `run` (what
//!           the patched runner execs per task).
//! Login:    `dispatch` (SSH forced command; verbs on argv, token on stdin).
//! Compute:  `build <run>` and `rebake`, which re-exec `__fetch`, `__build`
//!           and `__stub` to enter namespaces single-threaded.

pub mod callback;
pub mod cluster;
pub mod frame;
pub mod ids;
pub mod proc;
pub mod report;
pub mod sacct;
pub mod spec;
pub mod units;
pub mod verbs;
pub mod verdict;
pub mod vps;
