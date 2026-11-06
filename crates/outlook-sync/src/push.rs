//! Stalwart's JMAP push, as the sync's account: the mirror's share makes the owner's account's
//! state changes reachable.

use std::time::Duration;

use anyhow::{Context, Result};
use jmap::{Event, Events, Session};
use reqwest::Client;
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
    let session: Session = account
        .send(client.get(format!("{origin}/jmap/session")), "JMAP session")
        .await?
        .error_for_status()?
        .json()
        .await?;
    let owner = session
        .account(&policy.owner)
        .with_context(|| format!("{} isn't shared anything of {}", policy.account, policy.owner))?
        .to_owned();
    let source = format!("{origin}/jmap/eventsource/?types=CalendarEvent&closeafter=no&ping=60");
    let mut reply = account.send(client.get(source), "JMAP push").await?.error_for_status()?;
    // the mirror may have changed while no stream was open
    let _ = wake.send(Wake::Edited);
    let mut events = Events::default();
    while let Some(chunk) = reply.chunk().await? {
        for _ in events.push(&chunk).iter().filter(|e| changed(e, &owner)) {
            let _ = wake.send(Wake::Edited);
        }
    }
    anyhow::bail!("the stream ended")
}

fn changed(event: &str, owner: &str) -> bool {
    matches!(Event::read(event), Some(Event::Changed(c)) if c.changed.contains_key(owner))
}

#[cfg(test)]
#[path = "../tests/push.rs"]
mod tests;
