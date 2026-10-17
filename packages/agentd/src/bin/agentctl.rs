//! The host's end of the control connection: start or nudge an agent, and
//! print what the daemon replies, one JSON line each.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use agentd::{CONTROL_PORT, Reply, Request, Service, WAKE, prove, read_key};
use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_vsock::{VsockAddr, VsockStream};

#[derive(Parser)]
struct Args {
    /// The agent VM's vsock CID.
    #[arg(long, default_value_t = 4)]
    cid: u32,
    /// The key shared with the VM's daemon.
    #[arg(long)]
    key: PathBuf,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Give an agent a message; `-` reads it from stdin.
    Wake { agent: String, message: String },
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let Cmd::Wake { agent, mut message } = args.command;
    if message == "-" {
        message.clear();
        std::io::stdin().read_to_string(&mut message)?;
    }
    let mut stream = VsockStream::connect(VsockAddr::new(args.cid, CONTROL_PORT)).await?;
    prove(&mut stream, &read_key(&args.key)?, Service::Control, WAKE).await?;
    let (read, mut write) = tokio::io::split(stream);
    let mut request = serde_json::to_string(&Request::Wake { agent, message })?;
    request.push('\n');
    write.write_all(request.as_bytes()).await?;
    let mut lines = BufReader::new(read).lines();
    let mut status = ExitCode::SUCCESS;
    while let Some(line) = lines.next_line().await? {
        if let Ok(Reply::Refused { .. } | Reply::Exited { code: Some(1..) }) =
            serde_json::from_str(&line)
        {
            status = ExitCode::FAILURE;
        }
        println!("{line}");
    }
    Ok(status)
}
