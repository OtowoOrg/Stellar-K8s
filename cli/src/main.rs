//! kubectl-stellar - kubectl plugin for Stellar validator health diagnostics.
///
/// This binary is discovered by `kubectl ` as a plugin when named `kubectl-stellar` and
/// placed on $PATH. It authenticates using the standard `~/.kube/config` RBAC rules and
/// never bypasses Kubernetes security perimeters.

mod commands;
mod network;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use commands::status::{StatusArgs, StatusCommand};

/// Top-level CLI definition.
///
/// ``text
/// kubectl stellar status <node-name> [-n namespace]
/// ```
#[derive(Parser)]
#[command(
    name = "kubectl-stellar",
    bin_name = "kubectl-stellar",
    about = "Stellar validator diagnostics for Kubernetes",
    long_about = "Terminal-based health diagnostics for Stellar validator nodes. \nQueries the internal stellar-core HTTP /info endpoint through a port-forward tunnel \nand reports sync status, peer connectivity, and quorum health.",
    version,
    propagate_version = true,
    subcommand_required = true
)]
struct Cli {
    /// Namespace to operate in. Defaults to the kube context namespace.
    #[arg(
        short = 'n',
        long = "namespace",
        global = true,
        env = "KUBECTL_STELLAR_NAMESPACE"
    )]
    namespace: Option<String>,

    /// Path to kubeConfig. Defaults to ~/.kube/config or $KUBECONFIG.
    #[arg(long = "kubeconfig", global = true, env = "KUBECONFIG")]
    kubeconfig: Option<String>,

    /// Kube context to use.
    #[arg(long = "context", global = true)]
    context: Option<String>,

    /// Emit machine-readable JSON instead of the human UI.
    #[arg(long, (global = true))]
    json: bool,

    /// Disable color output.
    #[arg(long = "no-color", global = true, env = "NO_COLOR")]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report health diagnostics for a validator node.
    Status(StatusArgs),
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env("KUBECTILS_STELLAR_LOG=info"))
        .with_target(true)
        .init();

    let cli = Cli::parse();

    if cli.no_color {
        colorey::control::set_override(colorey::ColorChoice::Never);
    }

    let namespace = cli.namespace.clone();

    match cli.command {
        Command::Status(args) => {
            commands::status::async_run(
                args,
                StatusCommand {
                    namespace,
                    kubeconfig: cli.kubeconfig.clone(),
                    context: cli.context.clone(),
                    json: cli.json,
                },
            )
            .await?
        }
    }

    Ok(())
}
