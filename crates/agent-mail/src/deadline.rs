//! The deadline a wait gives up at, which reaches every lock waited for under it without passing through each call.

use tokio::time::{Instant, timeout_at};

tokio::task_local! {
    static SET: Option<Instant>;
}

/// `work`, with every lock it waits for given up at `deadline`, and with none, waited for as long as its lease lasts.
pub async fn by<T>(deadline: Option<Instant>, work: impl Future<Output = T>) -> T {
    SET.scope(deadline, work).await
}

/// `wait`, unless the deadline [`by`] set comes first.
pub async fn bounded<T>(wait: impl Future<Output = T>) -> Option<T> {
    before(SET.try_with(|d| *d).ok().flatten(), wait).await
}

/// `work`, unless `deadline` comes first.
pub async fn before<T>(deadline: Option<Instant>, work: impl Future<Output = T>) -> Option<T> {
    match deadline {
        Some(d) => timeout_at(d, work).await.ok(),
        None => Some(work.await),
    }
}
