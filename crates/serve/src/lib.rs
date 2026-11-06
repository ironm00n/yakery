use std::time::Duration;

const PAUSE: Duration = Duration::from_millis(100);

/// The next connection `accepting` takes; the pause after an error, such as running out of descriptors, keeps it
/// from spinning.
pub async fn accept<T>(mut accepting: impl AsyncFnMut() -> std::io::Result<T>) -> T {
    loop {
        match accepting().await {
            Ok(accepted) => return accepted,
            Err(e) => {
                yakery_log::warn!("accept: {e}");
                tokio::time::sleep(PAUSE).await;
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/accept.rs"]
mod tests;
