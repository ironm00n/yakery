//! One Microsoft 365 calendar mirrored into one Stalwart calendar: Graph tells the server when the
//! mailbox's events change, and a delta query of the default calendar brings the mirror up to it.

mod assertion;
mod event;
mod file;
mod graph;
mod ical;
mod ledger;
mod local;
mod policy;
mod push;
mod serve;
mod stalwart;
mod state;
mod sync;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::Parser;
use jiff::{Timestamp, ToSpan};
use private::{Dir, Secret};
use ring::digest::{SHA256, digest};

use crate::assertion::Credential;
use crate::policy::{POLICY, Route};
use crate::state::Login;
use crate::sync::{Shared, Sync};

#[derive(Parser)]
enum Command {
    /// Serve Graph's notifications and the sign-in callback on the socket systemd passes in, and
    /// keep the mirror up to date
    Serve,
    /// Print a sign-in link, good once, and wait for the sign-in to finish
    Login,
    /// Print the certificate the app registration needs, base64
    Certificate,
}

const LOGIN_MINUTES: i64 = 10;

fn state_dir() -> Result<Dir> {
    let path = PathBuf::from(std::env::var_os("STATE_DIRECTORY").context("StateDirectory")?);
    Dir::open(&path, &[])
}

fn main() -> ExitCode {
    yakery_log::exit_code(run().map(|()| ExitCode::SUCCESS))
}

fn run() -> Result<()> {
    let command = Command::parse();
    let sockets = private::listen_fds()?;
    let state = state_dir()?;
    match command {
        Command::Certificate => {
            println!("{}", Credential::read(&state)?.certificate);
            Ok(())
        }
        Command::Login => login(&state),
        Command::Serve => {
            let (_, http) = sockets
                .into_iter()
                .find(|(name, _)| name == "http")
                .context("no http socket from systemd")?;
            // reqwest is built without a rustls provider; the workspace's is ring
            let _ = rustls::crypto::ring::default_provider().install_default();
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            tokio::task::LocalSet::new().block_on(&runtime, daemon(Rc::new(state), http))
        }
    }
}

async fn daemon(state: Rc<Dir>, http: std::os::fd::OwnedFd) -> Result<()> {
    let credentials = PathBuf::from(
        std::env::var_os("CREDENTIALS_DIRECTORY").context("the app password, by LoadCredential")?,
    );
    let account: &'static stalwart::Account =
        Box::leak(Box::new(stalwart::Account::read(&POLICY, &Dir::open(&credentials, &[])?)?));
    let mirror = ledger::Mirror::new(&POLICY, account)?;
    let graph = graph::Graph::new(&POLICY, Credential::read(&state)?, state.clone())?;
    let (wake, wakes) = tokio::sync::mpsc::unbounded_channel();
    if POLICY.write {
        tokio::task::spawn_local(push::listen(&POLICY, account, wake.clone()));
    }
    let subscription = state.load(state::SUBSCRIPTION)?;
    let shared = Rc::new(Shared {
        subscription: RefCell::new(subscription),
        validating: Cell::new(false),
        wake,
    });
    let sync: &'static Sync =
        Box::leak(Box::new(Sync { graph, mirror, state, policy: &POLICY, shared }));
    let listener = std::net::TcpListener::from(http);
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    tokio::select! {
        served = serve::serve(sync, listener) => served?,
        () = sync.run(wakes) => {}
    }
    Ok(())
}

fn login(state: &Dir) -> Result<()> {
    let mut drawn = [0; 32];
    private::random::fill(&mut drawn)?;
    let verifier = Secret::new(URL_SAFE_NO_PAD.encode(drawn));
    let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.expose().as_bytes()));
    let login_state = private::random::token(32)?;
    let until = Timestamp::now() + LOGIN_MINUTES.minutes();
    let pending = Login::Pending { state: login_state.clone(), verifier, until };
    state.save(state::LOGIN, &Some(pending))?;
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("client_id", POLICY.client_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", POLICY.public.url(Route::Callback).as_str()),
        ("response_mode", "query"),
        ("scope", graph::scope(&POLICY)),
        ("state", login_state.expose()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    println!(
        "Open this link and sign in; it works once, for {LOGIN_MINUTES} minutes:\n\n{}?{}\n",
        POLICY.authority("authorize"),
        query.finish()
    );
    while Timestamp::now() < until {
        std::thread::sleep(Duration::from_secs(1));
        match state.load::<Option<Login>>(state::LOGIN)? {
            Some(Login::Done(Ok(()))) => {
                println!("Signed in; the sync subscribes and copies the calendar now.");
                return Ok(());
            }
            Some(Login::Done(Err(e))) => anyhow::bail!("signing in failed: {e}"),
            _ => {}
        }
    }
    anyhow::bail!("no sign-in came back within {LOGIN_MINUTES} minutes")
}
