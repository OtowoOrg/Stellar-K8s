mod cli;
mod commands;

use crate::cli::{Args, Commands};
use crate::commands::benchmark::run_benchmark_controller_cmd;
use crate::commands::check_crd::run_check_crd;
use crate::commands::info::run_info;
use crate::commands::operator::run_operator;
use crate::commands::runbook::run_generate_runbook;
use crate::commands::simulator::run_simulator;
use crate::commands::webhook::run_webhook;
use clap::Parser;
use std::process;

use stellar_k8s::controller::archive_prune::prune_archive;
use stellar_k8s::controller::diff::diff;
use stellar_k8s::version_check;
use stellar_k8s::{incident, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::parse();

    let offline = args.offline;

    let result = match args.command {
        Commands::Version => {
            println!("Stellar-K8s Operator v{}", env!("CARGO_PKG_VERSION"));
            println!("Build Date: {}", env!("BUILD_DATE"));
            println!("Git SHA: {}", env!("GIT_SHA"));
            println!("Rust Version: {}", env!("RUST_VERSION"));
            Ok(())
        }
        Commands::Info(info_args) => run_info(info_args).await,
        Commands::CheckCrd => run_check_crd().await,
        Commands::PruneArchive(prune_args) => prune_archive(prune_args).await,
        Commands::Diff(diff_args) => diff(diff_args).await,
        Commands::GenerateRunbook(runbook_args) => run_generate_runbook(runbook_args).await,
        Commands::IncidentReport(report_args) => incident::run_incident_report(report_args).await,
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            let mut cmd = Args::command();
            let name = cmd.get_name().to_string();
            generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
        Commands::Run(run_args) => {
            if let Err(e) = run_args.validate() {
                eprintln!("error: {e}");
                process::exit(2);
            }
            return run_operator(run_args).await;
        }
        Commands::Webhook(webhook_args) => return run_webhook(webhook_args).await,
        Commands::Benchmark(benchmark_args) => {
            return run_benchmark_controller_cmd(benchmark_args).await
        }
        Commands::Simulator(cli) => return run_simulator(cli).await,
    };

    version_check::check_and_notify(offline).await;
    result
}
