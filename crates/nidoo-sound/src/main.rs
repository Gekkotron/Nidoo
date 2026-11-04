use clap::{Parser, Subcommand};

/// A Sendspin-compatible server: plays one PCM input on every satellite, in sync.
#[derive(Debug, Parser)]
#[command(name = "nidoo-sound", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the sound service.
    Run,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run => anyhow::bail!("`run` is not implemented yet"),
    }
}
