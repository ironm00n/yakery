use std::time::Duration;

use anyhow::{Context, Result};
use rustix::io::Errno;

const PAUSE: Duration = Duration::from_millis(100);

/// The errors that clear by themselves: accept(2)'s for running short of descriptors or memory or for a connection
/// that failed on its way in, and epoll_ctl(2)'s ENOSPC, met registering the connection.
const PASSING: [Errno; 14] = [
    Errno::MFILE,
    Errno::NFILE,
    Errno::NOBUFS,
    Errno::NOMEM,
    Errno::NOSPC,
    Errno::INTR,
    Errno::CONNABORTED,
    Errno::PROTO,
    Errno::NETDOWN,
    Errno::NOPROTOOPT,
    Errno::HOSTDOWN,
    Errno::NONET,
    Errno::HOSTUNREACH,
    Errno::NETUNREACH,
];

/// The next connection `accepting` takes on `what`, or the error that says it can't take one again. An error that
/// clears by itself is waited out, one pause after each, and logged when the wait starts and when it ends.
pub async fn accept<T, F: Future<Output = std::io::Result<T>>>(
    what: &str,
    mut accepting: impl FnMut() -> F,
) -> Result<T> {
    let mut waited = false;
    loop {
        match accepting().await {
            Ok(accepted) => {
                if waited {
                    yakery_log::note!("accepting on {what} again");
                }
                return Ok(accepted);
            }
            Err(e) if Errno::from_io_error(&e).is_some_and(|n| PASSING.contains(&n)) => {
                if !waited {
                    yakery_log::warn!("accepting on {what}: {e}; trying again every {PAUSE:?}");
                }
                waited = true;
                tokio::time::sleep(PAUSE).await;
            }
            Err(e) => return Err(e).with_context(|| format!("accepting on {what}")),
        }
    }
}

#[cfg(test)]
#[path = "../tests/accept.rs"]
mod tests;
