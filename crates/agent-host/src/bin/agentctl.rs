//! The host's end of the control connection: start or nudge an agent, and print what the daemon replies, one JSON
//! line each, and exit 0 only if the wake went well (`Outcome`).

use std::{io::Read, process::ExitCode};

use agent_host::{Outcome, Vm};
use agent_proto::{Request, wake};
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    vm: Vm,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Give an agent a message; `-` reads it from stdin.
    Wake { agent: String, message: String },
}

fn main() -> ExitCode {
    yakery_log::exit_code(wake_agent(Args::parse()))
}

#[tokio::main]
async fn wake_agent(args: Args) -> Result<ExitCode> {
    let Cmd::Wake { agent, mut message } = args.command;
    if message == "-" {
        message.clear();
        std::io::stdin().read_to_string(&mut message)?;
    }
    let mut replies = wake(args.vm.cid, &args.vm.key()?, &Request::Wake { agent, message }).await?;
    let mut outcome = Outcome::default();
    while let Some(line) = replies.next_line().await? {
        if let Ok(reply) = serde_json::from_str(&line) {
            outcome.read(&reply);
        }
        println!("{line}");
    }
    Ok(if outcome.well { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}
