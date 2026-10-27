use std::{
    io::{Read, Write},
    path::PathBuf,
    process::ExitCode,
};

use agent_slack::{
    Channel, Message, Ts,
    api::{Line, Reply, Request, SOCKET, Said},
    unescape,
};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tokio::net::UnixStream;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = SOCKET)]
    socket: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Wait for the next message not yet taken, print it, then mark it taken; one wait at a time.
    Wait,
    /// Post the Markdown on stdin; without --thread it starts a thread and prints its ts.
    Send {
        #[arg(long)]
        thread: Option<Ts>,
        /// A listed channel other than the first.
        #[arg(long)]
        to: Option<Channel>,
    },
    /// Print a thread, oldest first.
    Read {
        thread: Ts,
        #[arg(long)]
        to: Option<Channel>,
    },
}

fn shown(m: &Message) -> String {
    let thread = m.thread_ts.map_or("-".into(), |t| t.to_string());
    format!(
        "channel {} ts {} thread {thread} from {}\n{}\n",
        m.channel,
        m.ts,
        m.user,
        unescape(&m.text)
    )
}

fn said(s: &Said) -> String {
    let from = s
        .user
        .as_ref()
        .map_or_else(|| format!("bot {}", s.bot_id.as_deref().unwrap_or("?")), |u| u.to_string());
    format!("ts {} from {from}\n{}\n\n", s.ts, unescape(&s.text))
}

fn print(text: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())?;
    Ok(out.flush()?)
}

async fn run(cli: Cli) -> Result<()> {
    let stream =
        UnixStream::connect(&cli.socket).await.with_context(|| cli.socket.display().to_string())?;
    let (read, write) = stream.into_split();
    let mut line = Line::new(read, write);
    let request = match cli.command {
        Command::Wait => Request::Wait,
        Command::Send { thread, to } => {
            let mut body = String::new();
            std::io::stdin().read_to_string(&mut body)?;
            Request::Send { to, thread, body }
        }
        Command::Read { thread, to } => Request::Read { to, thread },
    };
    let new_thread = matches!(request, Request::Send { thread: None, .. });
    line.send(&request).await?;
    match line.next().await?.context("agent-slackd closed the connection")? {
        Reply::Message(m) => {
            print(&shown(&m))?;
            line.send(&Request::Taken(m.key())).await?;
            match line.next().await? {
                Some(Reply::Taken) => Ok(()),
                _ => bail!(
                    "agent-slackd did not confirm the message taken; the next wait prints it again"
                ),
            }
        }
        Reply::Sent((_, ts)) if new_thread => print(&format!("{ts}\n")),
        Reply::Sent(_) => Ok(()),
        Reply::Thread(thread) => print(&thread.iter().map(said).collect::<String>()),
        Reply::Refused(why) => bail!("{why}"),
        Reply::Taken => bail!("agent-slackd answered out of turn"),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
    yakery_log::exit_code(
        runtime
            .map_err(anyhow::Error::from)
            .and_then(|r| r.block_on(run(cli)))
            .map(|()| ExitCode::SUCCESS),
    )
}
