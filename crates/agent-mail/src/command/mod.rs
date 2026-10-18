pub mod attachments;
pub mod done;
pub mod inbox;
pub mod send;
pub mod thread;
pub mod wait;

use anyhow::Result;

use crate::{
    auth::Auth,
    config::Config,
    show::{out, utc},
};

pub async fn login() -> Result<()> {
    let auth = Auth::new(Config::load()?)?;
    let access = auth.access().await?;
    out(&format!(
        "signed in as {}; the access token expires at {}",
        auth.login,
        utc(access.expires_at)
    ))
}

pub async fn rotate_password() -> Result<()> {
    let auth = Auth::new(Config::load()?)?;
    auth.rotate_password().await?;
    out(&format!("rotated the password of {}", auth.login))
}
