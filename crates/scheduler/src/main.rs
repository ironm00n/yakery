//! schedule.ironmoon.dev: a public page of when the owner is busy, and one-time links that put a
//! meeting on his calendar. One binary, run as the front, the core, or `scheduler link` as root.

mod ask;
mod checked;
mod core;
mod front;
mod link;
mod pages;
mod policy;
mod slots;
mod stalwart;
mod store;

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::ask::{AdminAnswer, AdminAsk};
use crate::link::{Issue, LinkId, Token};
use crate::policy::POLICY;

const FRONT_SOCKET: &str = "/run/scheduler/front.sock";
const ADMIN_SOCKET: &str = "/run/scheduler/admin.sock";

#[derive(Parser)]
enum Command {
    /// Serve HTTP on the socket systemd passes in, asking the core
    Front,
    /// Hold the links and the app passwords, and answer the front and `link`
    Core,
    #[command(subcommand)]
    Link(Admin),
}

#[derive(Subcommand)]
enum Admin {
    /// Issue a link and print its URL, once
    New(Issue),
    List,
    Revoke {
        id: LinkId,
    },
}

fn print(answer: AdminAnswer) -> Result<()> {
    for (id, issue, state) in answer.map_err(anyhow::Error::msg)? {
        let hours: Vec<String> = issue.hours.iter().map(ToString::to_string).collect();
        println!(
            "{id}  {} to {}  {}  {}  {state}",
            issue.from,
            issue.until,
            issue.duration,
            hours.join(", ")
        );
    }
    Ok(())
}

/// The sockets systemd passed in, by `FileDescriptorName=`.
pub struct Sockets(BTreeMap<String, OwnedFd>);

impl Sockets {
    pub fn take(&mut self, name: &str) -> Result<OwnedFd> {
        self.0.remove(name).with_context(|| format!("no {name} socket from systemd"))
    }
}

fn main() -> ExitCode {
    yakery_log::exit_code(run().map(|()| ExitCode::SUCCESS))
}

fn run() -> Result<()> {
    let command = Command::parse();
    let sockets = Sockets(private::listen_fds()?.into_iter().collect());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async {
        match command {
            Command::Front => front::run(Path::new(FRONT_SOCKET), sockets).await,
            Command::Core => core::run(&POLICY, sockets).await,
            Command::Link(Admin::New(issue)) => {
                let token = Token::random()?;
                let answer: AdminAnswer = ask::ask(
                    Path::new(ADMIN_SOCKET),
                    &AdminAsk::Issue { digest: token.digest(), limits: issue },
                )
                .await?;
                answer.map_err(anyhow::Error::msg)?;
                println!("https://{}/b/{}", POLICY.host, token.expose());
                Ok(())
            }
            Command::Link(Admin::List) => {
                print(ask::ask(Path::new(ADMIN_SOCKET), &AdminAsk::List).await?)
            }
            Command::Link(Admin::Revoke { id }) => {
                print(ask::ask(Path::new(ADMIN_SOCKET), &AdminAsk::Revoke(id)).await?)
            }
        }
    })
}
