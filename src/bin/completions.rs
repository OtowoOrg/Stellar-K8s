use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::io;

#[derive(Parser)]
#[command(name = "stellar-operator")]
#[command(about = "Generate shell completion scripts for stellar-operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

// Minimal Args structure matching main.rs for completion generation
#[derive(Parser)]
#[command(
    author,
    version,
    about = "Stellar-K8s: Cloud-Native Kubernetes Operator for Stellar Infrastructure"
)]
struct Args {
    #[command(subcommand)]
    command: MainCommands,
}

#[derive(Subcommand)]
enum MainCommands {
    /// Run the operator reconciliation loop
    Run,
    /// Run the admission webhook server
    Webhook,
    /// Show version and build information
    Version,
    /// Show cluster information (node count) for a namespace
    Info,
    /// Local simulator (kind/k3s + operator + demo validators)
    Simulator,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Args::command();
            let bin_name = "stellar-operator";
            generate(shell, &mut cmd, bin_name, &mut io::stdout());
        }
    }
}
