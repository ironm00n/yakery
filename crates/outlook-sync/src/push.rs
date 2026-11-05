//! Stalwart's JMAP push, as the sync's account: the mirror's share makes the owner's account's
//! state changes reachable.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::policy::Policy;
use crate::stalwart::Account;
use crate::sync::Wake;

const AGAIN: Duration = Duration::from_secs(5);

pub async fn listen(policy: &'static Policy, account: &Account, wake: UnboundedSender<Wake>) {
    loop {
        if let Err(e) = stream(policy, account, &wake).await {
            yakery_log::warn!("stalwart push: {e:#}; listening again in {} s", AGAIN.as_secs());
        }
        tokio::time::sleep(AGAIN).await;
    }
}

async fn stream(policy: &Policy, account: &Account, wake: &UnboundedSender<Wake>) -> Result<()> {
    let client = Client::new();
    let origin = format!("http://{}", policy.stalwart);
    let session: Value = account
        .send(client.get(format!("{origin}/jmap/session")), "JMAP session")
        .await?
        .error_for_status()?
        .json()
        .await?;
    let owner = session["accounts"]
        .as_object()
        .and_then(|accounts| accounts.iter().find(|(_, a)| a["name"] == policy.owner.as_str()))
        .map(|(id, _)| id.clone())
        .with_context(|| format!("{} isn't shared anything of {}", policy.account, policy.owner))?;
    let events = format!("{origin}/jmap/eventsource/?types=CalendarEvent&closeafter=no&ping=60");
    let mut reply = account.send(client.get(events), "JMAP push").await?.error_for_status()?;
    // the mirror may have changed while no stream was open
    let _ = wake.send(Wake::Edited);
    let mut pending = String::new();
    while let Some(chunk) = reply.chunk().await? {
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = pending.find("\n\n") {
            let event: String = pending.drain(..end + 2).collect();
            if changed(&event, &owner) {
                let _ = wake.send(Wake::Edited);
            }
        }
    }
    anyhow::bail!("the stream ended")
}

fn changed(event: &str, owner: &str) -> bool {
    event.lines().filter_map(|l| l.strip_prefix("data:")).any(|data| {
        serde_json::from_str::<Value>(data.trim())
            .is_ok_and(|v| v["@type"] == "StateChange" && v["changed"].get(owner).is_some())
    })
}

#[cfg(test)]
#[path = "../tests/push.rs"]
mod tests;
