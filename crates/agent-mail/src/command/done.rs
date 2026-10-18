use anyhow::{Result, ensure};
use jmap_client::email::Property as P;

use crate::{
    auth::Auth,
    config::Config,
    mail::{HANDLED, Mail},
    show::{id, out},
};

/// Marks mail `$handled`, so `wait` neither delivers nor reminds of it.
pub async fn run(ids: Vec<String>) -> Result<()> {
    let mail = Mail::connect(Auth::new(Config::load()?)?).await?;
    let found = mail.emails(&ids, &[P::Id], |_| {}).await?;
    let missing: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|i| !found.iter().any(|m| id(m) == *i))
        .collect();
    ensure!(missing.is_empty(), "no email {}", missing.join(", "));
    let marks: Vec<(&str, Vec<&str>)> = ids.iter().map(|i| (i.as_str(), vec![HANDLED])).collect();
    mail.mark(&marks).await?;
    out(&format!("handled {}", ids.join(", ")))
}
