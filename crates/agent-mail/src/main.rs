//! Agent mail through Stalwart's JMAP, signed in with ZITADEL's device flow.

mod address;
mod auth;
mod body;
mod command;
mod config;
mod folder;
mod home;
mod identity;
mod mail;
mod mdn;
mod mime;
mod origin;
mod outbox;
mod policy;
mod random;
mod secret;
mod show;
mod signature;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use command::wait::Waited;

/// Agent mail through Stalwart, signed in with ZITADEL.
#[derive(Parser)]
#[command(name = "agent-mail", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send the Markdown on stdin, asking for a read receipt.
    Send(command::send::Args),
    /// List recent inbox mail; `*` marks unread; read-only.
    Inbox(command::inbox::Args),
    /// Print the thread of an email, oldest first, with read receipts as a line under the mail they answer;
    /// read-only.
    Thread {
        #[arg(value_name = "EMAIL_ID")]
        id: String,
    },
    /// Save each attachment of an email into <passwd home>/.cache/agent-mail/attachments/<email id>/ and print
    /// the absolute path of each, one per line; read-only. Names are cut down to [A-Za-z0-9._-]. A file saved by
    /// an earlier run is printed again, not replaced. Exits 1 if any attachment was skipped, over 50 MiB each or
    /// 200 MiB in all, or failed; each is named on stderr with why.
    Attachments {
        #[arg(value_name = "EMAIL_ID")]
        id: String,
    },
    /// Wait for watched mail from others that is unseen, unanswered and not done; claim it as seen, print it, and
    /// send any read receipt it asks for. Exits 2 at --timeout. Seen mail still unanswered is listed on stderr.
    Wait(command::wait::Args),
    /// Mark mail handled, so wait neither delivers it nor lists it again.
    Done {
        #[arg(value_name = "EMAIL_ID", required = true)]
        ids: Vec<String>,
    },
    /// Sign in if needed and show when the access token expires.
    Login,
    /// Rotate the login's password.
    Password {
        #[command(subcommand)]
        action: Password,
    },
}

#[derive(Subcommand)]
enum Password {
    /// Replace the password with a random one and store it in the password file.
    Rotate,
}

/// TLS roots and key logging that the caller's environment could redirect: with these gone, rustls-platform-verifier
/// (through rustls-native-certs and openssl-probe) reads the system store, so a proxy can only relay a tunnel.
const TRUST_FROM_ENVIRONMENT: [&str; 4] = [
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NIX_SSL_CERT_FILE",
    "SSLKEYLOGFILE",
];

fn main() -> ExitCode {
    for name in TRUST_FROM_ENVIRONMENT {
        // SAFETY: no other thread exists yet; the runtime is built below.
        unsafe { std::env::remove_var(name) };
    }
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(run()),
        Err(e) => {
            show::warn(&format!("starting the runtime: {e}"));
            ExitCode::FAILURE
        }
    }
}

async fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(u8::from(e.use_stderr()));
        }
    };
    let ran = match cli.command {
        Command::Send(args) => command::send::run(args).await,
        Command::Inbox(args) => command::inbox::run(args).await,
        Command::Thread { id } => command::thread::run(id).await,
        Command::Attachments { id } => command::attachments::run(id).await,
        Command::Wait(args) => match command::wait::run(args).await {
            Ok(Waited::TimedOut) => return ExitCode::from(2),
            waited => waited.map(drop),
        },
        Command::Done { ids } => command::done::run(ids).await,
        Command::Login => command::login().await,
        Command::Password {
            action: Password::Rotate,
        } => command::rotate_password().await,
    };
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            show::warn(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}
