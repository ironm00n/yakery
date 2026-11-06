mod address;
mod auth;
mod authority;
mod cmd;
mod jmap;
mod listeners;
mod mail;
mod out;
mod policy;
mod prelude;
mod show;
mod store;
mod undefined;
mod urgent;
mod web;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cmd::{invite, read, send, wait};

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
    TIME_ZONE = "AGENT_MAIL_TIME_ZONE" | "America/New_York";
    CORRESPONDENTS = "AGENT_MAIL_CORRESPONDENTS" | "" | true;
    SUBMISSION_KEY = "AGENT_MAIL_SUBMISSION_KEY" | "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw" | true;
    CORRESPONDENTS_FOR = "AGENT_MAIL_CORRESPONDENTS_FOR"
        | r#"{"agent+project@example.org": ["colleague@example.com"]}"#;
}

/// A wait's periods (`periods.nix`), in whole seconds from the build.
macro_rules! periods {
    ($($name:ident = $var:literal | $fixture:literal;)*) => {$(
        #[cfg(not(test))]
        pub const $name: std::time::Duration = std::time::Duration::from_secs(
            match u64::from_str_radix(env!($var), 10) {
                Ok(secs) => secs,
                Err(_) => panic!(concat!($var, " is not a whole number of seconds")),
            },
        );
        #[cfg(test)]
        pub const $name: std::time::Duration = std::time::Duration::from_secs($fixture);
    )*};
}

periods! {
    RECHECK = "AGENT_MAIL_RECHECK_SECS" | 150;
    BACKOFF = "AGENT_MAIL_BACKOFF_SECS" | 5;
    STALE = "AGENT_MAIL_STALE_SECS" | 300;
    REOPEN = "AGENT_MAIL_REOPEN_SECS" | 30;
}

const _: () = assert!(
    STALE.as_secs() > RECHECK.as_secs(),
    "an idle wait scans once a recheck period, so a shorter stale bound fails it while healthy"
);

/// What went wrong, as far as retries and outcomes care; only the edges tag, and anything untagged is transient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Config,
    PasswordRefused,
    TokenRefused,
    /// Over one of Stalwart's limits, with how long it asked us to wait.
    Busy(Option<std::time::Duration>),
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

    /// The tag `e` carries, if any; [`Failure::of`] reads an untagged error as transient.
    pub fn tag(e: &anyhow::Error) -> Option<Failure> {
        e.chain().find_map(|c| c.downcast_ref::<Tagged>()).map(|t| t.0)
    }

    pub fn of(e: &anyhow::Error) -> Failure {
        Failure::tag(e).unwrap_or(Failure::Transient)
    }

    pub fn permanent(self) -> bool {
        matches!(self, Failure::Config | Failure::PasswordRefused)
    }

    pub fn nothing_ran(self) -> bool {
        matches!(self, Failure::TokenRefused | Failure::Busy(_) | Failure::Unprocessed)
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
    #[command(about = wait::WAIT_HELP)]
    Wait(wait::Args) => wait::run,
    /// Record topics and threads for this Claude Code session's `wait --session`, without sending.
    Subscribe(wait::Subscribe) => wait::subscribe,
    /// List the topics and threads a Claude Code session's `wait --session` watches, this one's by default; read-only.
    Subscriptions(wait::Subscriptions) => wait::subscriptions,
    /// Take exactly the topics and threads named out of what a Claude Code session's `wait --session` watches, this
    /// one's by default. Until that session's mail-listener next scans, other waits still leave it Owen's word to them.
    Unsubscribe(wait::Unsubscribe) => wait::unsubscribe,
    /// Mark mail handled, so wait neither delivers it nor lists it again.
    Done(read::Many) => read::done,
    /// Sign in if needed and show when the access token expires.
    Login(read::Nothing) => read::login,
    /// Rotate the login's password.
    Password(read::Password) => read::rotate,
    /// Print the references in the Markdown on stdin that `send` warns of, those a reader who wasn't in the session
    /// can't resolve; read-only.
    Lint(read::Nothing) => send::lint,
    /// Invite people to events from this login's calendar; Stalwart sends the invitations, updates and cancels.
    #[command(subcommand)]
    Invite(invite::Args) => invite::run,
    /// List the invitations this login sent, one line each: id, start, title and each answer; read-only.
    Invites(invite::List) => invite::list,
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
