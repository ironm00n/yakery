use std::{
    env, fs,
    os::fd::{BorrowedFd, FromRawFd, OwnedFd, RawFd},
};

use anyhow::{Context, Result, ensure};
use rustix::io::{FdFlags, fcntl_setfd};

const VARS: [&str; 3] = ["LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"];

/// The fds systemd passed this process (sd_listen_fds(3)), each with its `FileDescriptorName=`, close-on-exec and
/// out of the environment, so no child takes them up; none without `$LISTEN_PID`. Clearing the environment is sound
/// only while one thread runs, so with more it refuses.
#[allow(unsafe_code)]
pub fn listen_fds() -> Result<Vec<(String, OwnedFd)>> {
    let [pid, count, names] = VARS.map(|var| env::var(var).ok());
    let Some(passed) =
        passed(pid.as_deref(), count.as_deref(), names.as_deref(), std::process::id())?
    else {
        return Ok(Vec::new());
    };
    let threads =
        fs::read_dir("/proc/self/task").context("counting this process's threads")?.count();
    ensure!(
        threads == 1,
        "socket activation is taken up before a second thread starts, not with {threads}"
    );
    for (_, fd) in &passed {
        fs::symlink_metadata(format!("/proc/self/fd/{fd}"))
            .with_context(|| format!("fd {fd} from systemd"))?;
        // SAFETY: the fd is open, as /proc shows, and systemd passed it to this process alone.
        fcntl_setfd(unsafe { BorrowedFd::borrow_raw(*fd) }, FdFlags::CLOEXEC)?;
    }
    for var in VARS {
        // SAFETY: this process runs one thread, so nothing reads the environment meanwhile.
        unsafe { env::remove_var(var) };
    }
    // SAFETY: each fd is open and becomes the one OwnedFd that closes it.
    Ok(passed.into_iter().map(|(name, fd)| (name, unsafe { OwnedFd::from_raw_fd(fd) })).collect())
}

fn passed(
    pid: Option<&str>,
    count: Option<&str>,
    names: Option<&str>,
    own: u32,
) -> Result<Option<Vec<(String, RawFd)>>> {
    let Some(pid) = pid else { return Ok(None) };
    ensure!(pid.parse() == Ok(own), "LISTEN_PID={pid} is not this process, {own}");
    let count: usize = count
        .and_then(|c| c.parse().ok())
        .with_context(|| format!("LISTEN_FDS={count:?} is not a count"))?;
    let end = RawFd::try_from(count)
        .ok()
        .and_then(|c| c.checked_add(3))
        .context("LISTEN_FDS is past every fd")?;
    let names: Vec<String> = match names {
        None => vec!["unknown".into(); count],
        Some("") if count == 0 => Vec::new(),
        Some(names) => names.split(':').map(String::from).collect(),
    };
    ensure!(
        names.len() == count,
        "LISTEN_FDNAMES names {} fds, but LISTEN_FDS passes {count}",
        names.len()
    );
    Ok(Some(names.into_iter().zip(3..end).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(pid: Option<&str>, count: Option<&str>, names: Option<&str>) -> String {
        match passed(pid, count, names, 41) {
            Ok(None) => "none".into(),
            Ok(Some(fds)) => {
                fds.iter().map(|(name, fd)| format!("{name}={fd}")).collect::<Vec<_>>().join(" ")
            }
            Err(e) => format!("error: {e:#}"),
        }
    }

    #[test]
    fn passed_fds_start_at_3_under_their_names() {
        assert_eq!(shown(None, Some("2"), Some("a:b")), "none");
        assert_eq!(shown(Some("41"), Some("2"), Some("api:control")), "api=3 control=4");
        assert_eq!(shown(Some("41"), Some("2"), None), "unknown=3 unknown=4");
        assert_eq!(shown(Some("41"), Some("0"), Some("")), "");
        assert_eq!(shown(Some("41"), Some("0"), None), "");
    }

    #[test]
    fn another_process_or_a_malformed_count_is_refused() {
        assert_eq!(
            shown(Some("42"), Some("1"), None),
            "error: LISTEN_PID=42 is not this process, 41"
        );
        assert_eq!(
            shown(Some("x"), Some("1"), None),
            "error: LISTEN_PID=x is not this process, 41"
        );
        for count in [None, Some(""), Some("-1"), Some("1x")] {
            assert!(shown(Some("41"), count, None).starts_with("error: LISTEN_FDS="), "{count:?}");
        }
        assert_eq!(
            shown(Some("41"), Some("4294967295"), None),
            "error: LISTEN_FDS is past every fd"
        );
        for names in ["api", "a:b:c", ""] {
            let refused = shown(Some("41"), Some("2"), Some(names));
            assert!(refused.ends_with("but LISTEN_FDS passes 2"), "{names}: {refused}");
        }
    }
}
