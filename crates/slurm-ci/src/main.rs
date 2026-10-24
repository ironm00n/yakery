use std::io::Write;

use anyhow::Result;
use clap::{Parser, Subcommand};

use slurm_ci::ids::RunId;
use slurm_ci::{cluster, vps};

#[derive(Parser)]
#[command(
    name = "slurm-ci",
    about = "Forgejo Actions → Explorer Slurm CI",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// VPS daemon: callback listener, run registry, cluster client.
    Listen,
    /// VPS per-task client, exec'd by the patched forgejo-runner.
    Run,
    /// Login node: the SSH forced-command entry.
    Dispatch,
    /// Compute node: build one run (the `sbatch --wrap` command).
    Build { run: String },
    /// Compute node: materialise the next bootstrap store.
    Rebake,
    /// VPS: ask the cluster to queue a rebake (zero arguments by design).
    TriggerRebake,
    #[command(name = "__ns", hide = true)]
    Ns(cluster::sandbox::NsArgs),
    #[command(name = "__fetch", hide = true)]
    Fetch(cluster::sandbox::FetchArgs),
    #[command(name = "__build", hide = true)]
    NsBuild(cluster::sandbox::BuildArgs),
    #[command(name = "__stub", hide = true)]
    Stub(cluster::sandbox::StubArgs),
}

fn main() -> Result<()> {
    let code = match Cli::parse().cmd {
        Cmd::Listen => vps::listen::main().map(|()| 0)?,
        Cmd::Run => vps::run::main()?,
        Cmd::Dispatch => cluster::dispatch::main()?,
        Cmd::Build { run } => cluster::build::main(RunId::parse(&run)?)?,
        Cmd::Rebake => cluster::rebake::main()?,
        Cmd::TriggerRebake => {
            let job = vps::cluster::Cluster::from_env()?.rebake()?;
            println!("queued rebake as Slurm job {job}");
            0
        }
        Cmd::Ns(a) => cluster::sandbox::ns_main(a)?,
        Cmd::Fetch(a) => cluster::sandbox::fetch_main(a)?,
        Cmd::NsBuild(a) => cluster::sandbox::build_main(a)?,
        Cmd::Stub(a) => cluster::sandbox::stub_main(a)?,
    };
    std::io::stdout().flush().ok();
    std::process::exit(code);
}
