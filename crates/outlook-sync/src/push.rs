//! Stalwart's JMAP push, as the sync's account: a state change in the owner's account, which the
//! mirror's share makes reachable, wakes the loop to look at the mirror for his edits.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::policy::Policy;
use crate::sync::Wake;

/// How long after a dropped stream the listener connects again.
const AGAIN: Duration = Duration::from_secs(5);

pub async fn listen(policy: &'static Policy, password: &Path, wake: UnboundedSender<Wake>) {
    loop {
        if let Err(e) = stream(policy, password, &wake).await {
            yakery_log::warn!("stalwart push: {e:#}; listening again in {} s", AGAIN.as_secs());
        }
        tokio::time::sleep(AGAIN).await;
    }
}

async fn stream(policy: &Policy, password: &Path, wake: &UnboundedSender<Wake>) -> Result<()> {
    let password = std::fs::read_to_string(password)?.trim().to_string();
    let client = Client::new();
    let origin = format!("http://{}", policy.stalwart);
    let session: Value = client
        .get(format!("{origin}/jmap/session"))
        .basic_auth(&policy.account, Some(&password))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let owner = session["accounts"]
        .as_object()
        .and_then(|accounts| accounts.iter().find(|(_, a)| a["name"] == policy.owner.as_str()))
        .map(|(id, _)| id.clone())
        .with_context(|| format!("{} isn't shared anything of {}", policy.account, policy.owner))?;
    let mut reply = client
        .get(format!("{origin}/jmap/eventsource/?types=CalendarEvent&closeafter=no&ping=60"))
        .basic_auth(&policy.account, Some(&password))
        .send()
        .await?
        .error_for_status()?;
    // the mirror may have changed while no stream was open
    let _ = wake.send(Wake::Local);
    let mut pending = String::new();
    while let Some(chunk) = reply.chunk().await? {
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = pending.find("\n\n") {
            let event: String = pending.drain(..end + 2).collect();
            if changed(&event, &owner) {
                let _ = wake.send(Wake::Local);
            }
        }
    }
    anyhow::bail!("the stream ended")
}

/// Whether a server-sent event is a StateChange naming the owner's account.
fn changed(event: &str, owner: &str) -> bool {
    event.lines().filter_map(|l| l.strip_prefix("data:")).any(|data| {
        serde_json::from_str::<Value>(data.trim())
            .is_ok_and(|v| v["@type"] == "StateChange" && v["changed"].get(owner).is_some())
    })
}

#[cfg(test)]
#[path = "../tests/push.rs"]
mod tests;
