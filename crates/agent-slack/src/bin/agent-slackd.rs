use std::{path::PathBuf, process::ExitCode};

use agent_slack::{daemon::Daemon, slack::Slack};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use private::{Dir, Secret, listen_fds};
use reqwest::Url;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "https://slack.com/api/")]
    api: Url,
    /// Channel ids, one per line; reread on every event and every request.
    #[arg(long, default_value = "/etc/agent-slack/channels")]
    channels: PathBuf,
    /// Where the inbox is kept.
    #[arg(long)]
    state: PathBuf,
    /// The directory holding bot_token and app_token, systemd's credentials.
    #[arg(long)]
    credentials: PathBuf,
}

fn token(args: &Args, name: &str) -> Result<Secret> {
    let path = args.credentials.join(name);
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Ok(Secret::new(text.trim()))
}

/// systemd holds the socket (agent-slack.socket), so its owner and mode are in Nix and clients queue while the
/// daemon restarts; it is taken up before the runtime starts a thread, as `listen_fds` requires.
fn main() -> ExitCode {
    #[allow(unsafe_code)]
    for name in ["SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE", "SSLKEYLOGFILE"] {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::remove_var(name) };
    }
    let args = Args::parse();
    let started = listen_fds().and_then(|passed| {
        let [(_, api)] = <[_; 1]>::try_from(passed).map_err(|p| {
            anyhow!("systemd passed {} sockets; agent-slackd runs from agent-slack.socket", p.len())
        })?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let api = std::os::unix::net::UnixListener::from(api);
        api.set_nonblocking(true)?;
        tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
            let slack = Slack::new(
                args.api.clone(),
                token(&args, "bot_token")?,
                token(&args, "app_token")?,
            )?;
            let daemon = Daemon::start(slack, Dir::open(&args.state, &[])?, args.channels).await?;
            daemon.run(tokio::net::UnixListener::from_std(api)?).await;
            Ok(())
        })
    });
    yakery_log::exit_code(started.map(|()| ExitCode::SUCCESS))
}
