mod address;
mod auth;
mod cmd;
mod jmap;
mod mail;
mod out;
mod policy;
mod prelude;
mod show;
mod store;
mod web;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cmd::{read, send, wait};

macro_rules! deployment {
    ($($name:ident = $var:literal | $fixture:literal $(| $empty:literal)?;)*) => {$(
        #[cfg(not(test))]
        pub const $name: &str = env!($var);
        #[cfg(test)]
        pub const $name: &str = $fixture;
        const _: () = assert!(!$name.is_empty() $(|| $empty)?, concat!($var, " is empty"));
    )*};
}

deployment! {
    AUTH_HOST = "AGENT_MAIL_AUTH_HOST" | "auth.example.org";
    CLIENT_ID = "AGENT_MAIL_CLIENT_ID" | "agent-mail-test";
    JMAP_HOST = "AGENT_MAIL_JMAP_HOST" | "mail.example.org";
    DOMAIN = "AGENT_MAIL_DOMAIN" | "example.org";
    AGENT_DOMAIN = "AGENT_MAIL_AGENT_DOMAIN" | "agent.example.org";
    SHARED_LOCAL_PART = "AGENT_MAIL_SHARED_LOCAL_PART" | "agent";
    CORRESPONDENTS = "AGENT_MAIL_CORRESPONDENTS" | "" | true;
    CORRESPONDENTS_FOR = "AGENT_MAIL_CORRESPONDENTS_FOR"
        | r#"{"agent+unboca@example.org": ["colleague@example.com"]}"#;
}

/// What went wrong, as far as retries and outcomes care; only the edges tag, and anything untagged is transient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Config,
    PasswordRefused,
    TokenRefused,
    Busy,
    Unprocessed,
    Transient,
    Idle,
    Dropped,
}

#[derive(Debug)]
struct Tagged(Failure, String);

impl std::fmt::Display for Tagged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

impl std::error::Error for Tagged {}

impl Failure {
    pub fn with(self, message: impl Into<String>) -> anyhow::Error {
        Tagged(self, message.into()).into()
    }

    pub fn of(e: &anyhow::Error) -> Failure {
        e.chain().find_map(|c| c.downcast_ref::<Tagged>()).map_or(Failure::Transient, |t| t.0)
    }

    pub fn permanent(self) -> bool {
        matches!(self, Failure::Config | Failure::PasswordRefused)
    }

    pub fn nothing_ran(self) -> bool {
        matches!(self, Failure::TokenRefused | Failure::Busy | Failure::Unprocessed)
    }
}

/// Agent mail through Stalwart, signed in with ZITADEL.
#[derive(Parser)]
#[command(name = "agent-mail", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

macro_rules! commands {
    ($($(#[$doc:meta])* $name:ident($args:ty) => $run:path,)*) => {
        #[derive(Subcommand)]
        enum Command { $($(#[$doc])* $name($args),)* }

        impl Command {
            async fn run(self) -> anyhow::Result<ExitCode> {
                match self { $(Command::$name(args) => $run(args).await,)* }
            }
        }
    };
}

commands! {
    /// Send the Markdown on stdin, asking for a read receipt.
    Send(send::Args) => send::run,
    /// List recent inbox mail; `*` marks unread; read-only.
    Inbox(read::Inbox) => read::inbox,
    /// Print the thread of an email, oldest first, with read receipts as a line under the mail they answer;
    /// read-only.
    Thread(read::One) => read::thread,
    #[command(about = read::ATTACHMENTS_HELP)]
    Attachments(read::One) => read::attachments,
    /// Wait for watched mail from others that is unseen, unanswered and not done; claim it as seen, print it, and
    /// send any read receipt it asks for. Exits 2 at --timeout. Seen mail still unanswered is listed on stderr.
    Wait(wait::Args) => wait::run,
    /// Mark mail handled, so wait neither delivers it nor lists it again.
    Done(read::Many) => read::done,
    /// Sign in if needed and show when the access token expires.
    Login(read::Nothing) => read::login,
    /// Rotate the login's password.
    Password(read::Password) => read::rotate,
}

fn main() -> ExitCode {
    // With these gone, rustls-platform-verifier reads the system store, so a proxy can only relay a tunnel.
    #[allow(unsafe_code)]
    for name in ["SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE", "SSLKEYLOGFILE"] {
        // SAFETY: no other thread exists yet; the runtime is built below.
        unsafe { std::env::remove_var(name) };
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(u8::from(e.use_stderr()));
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
    yakery_log::exit_code(
        runtime.map_err(anyhow::Error::from).and_then(|r| r.block_on(cli.command.run())),
    )
}

#[cfg(test)]
mod tests;
