use clap::Parser;
use std::process;
use stellar_k8s::cli::{Args, BackupCommands, Commands};
use stellar_k8s::commands::backup::{run_backup, run_cleanup, run_list, run_restore};
use stellar_k8s::commands::benchmark::run_benchmark_controller_cmd;
use stellar_k8s::commands::check_crd::run_check_crd;
use stellar_k8s::commands::doctor::run_doctor;
use stellar_k8s::commands::export_compliance::run_export_compliance;
use stellar_k8s::commands::health_check::run_health_check;
use stellar_k8s::commands::info::run_info;
use stellar_k8s::commands::operator::run_operator;
use stellar_k8s::commands::runbook::run_generate_runbook;
use stellar_k8s::commands::schema_compat::run_schema_compat;
use stellar_k8s::commands::simulator::run_simulator;
use stellar_k8s::commands::webhook::run_webhook;

use stellar_k8s::controller::archive_prune::prune_archive;
use stellar_k8s::controller::diff::diff;
use stellar_k8s::version_check;
use stellar_k8s::{incident, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
        Commands::Incident { command } => match command {
            incident::IncidentCommands::Collect(args) => incident::run_incident_collect(args).await,
            incident::IncidentCommands::Report(args) => incident::run_incident_report(args).await,
        },
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            let mut cmd = Args::command();
            let name = "stellar-operator".to_string();
            generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
        Commands::InstallCompletion { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate_to;
            use std::env;
            use std::path::PathBuf;

            let mut cmd = Args::command();
            let name = "stellar-operator".to_string();

            let home_dir = env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));

            let out_dir = match shell {
                clap_complete::Shell::Bash => {
                    home_dir.join(".local/share/bash-completion/completions")
                }
                clap_complete::Shell::Zsh => home_dir.join(".zsh/completions"),
                clap_complete::Shell::Fish => home_dir.join(".config/fish/completions"),
                _ => std::env::current_dir().unwrap_or_default(),
            };

            if let Err(e) = std::fs::create_dir_all(&out_dir) {
                eprintln!("Failed to create directory {}: {}", out_dir.display(), e);
                std::process::exit(1);
            }

            match generate_to(shell, &mut cmd, &name, &out_dir) {
                Ok(path) => {
                    println!(
                        "Successfully installed {} completion script at: {}",
                        shell,
                        path.display()
                    );
                    if shell == clap_complete::Shell::Zsh {
                        println!("\nNote: Make sure {} is in your $fpath.", out_dir.display());
                        println!("You may need to add this to your ~/.zshrc:");
                        println!("  fpath=({} $fpath)", out_dir.display());
                        println!("  autoload -Uz compinit && compinit");
                    } else if shell == clap_complete::Shell::Bash {
                        println!("\nNote: You may need to restart your shell or run:");
                        println!("  source {}", path.display());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to generate completion script: {}", e);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        Commands::Run(run_args) => {
            if let Err(e) = run_args.validate() {
                eprintln!("error: {e}");
                process::exit(2);
            }

            // Create a Kubernetes client for leader election.
            let k8s_client = match kube::Client::try_default().await {
                Ok(client) => client,
                Err(e) => {
                    eprintln!("Failed to create Kubernetes client for leader election: {e}");
                    process::exit(1);
                }
            };

            // Start leader election to ensure only one operator instance is active.
            let leader = match stellar_k8s::controller::leader::LeaderElectionHandle::start(
                k8s_client, None, None, None,
            ) {
                Ok(handle) => handle,
                Err(e) => {
                    eprintln!("Failed to start leader election: {e}");
                    process::exit(1);
                }
            };

            // Wait until this pod becomes the leader before starting the operator.
            leader.wait_until_leader().await;

            // Run the operator while we hold the lease. If the lease is lost (e.g.,
            // during network partition or pod failure), abort the operator so the pod
            // can restart and another replica can take over.
            tokio::select! {
                result = run_operator(run_args) => result,
                _ = leader.wait_until_lost() => {
                    eprintln!("Lost leader lease; shutting down operator");
                    Ok(())
                }
            }
        }
        Commands::Webhook(webhook_args) => return run_webhook(webhook_args).await,
        Commands::Doctor(doctor_args) => return run_doctor(doctor_args).await,
        Commands::SchemaCompat(args) => return run_schema_compat(args),
        Commands::HealthCheck(hc_args) => return run_health_check(hc_args),
        Commands::Benchmark(benchmark_args) => {
            return run_benchmark_controller_cmd(benchmark_args).await;
        }
        Commands::Simulator(cli) => return run_simulator(cli).await,
        Commands::BenchmarkCompare(compare_args) => {
            return stellar_k8s::benchmark_compare::run_benchmark_compare(compare_args)
                .await
                .map_err(|e| Error::ConfigError(e.to_string()));
        }
        Commands::ExportCompliance(export_args) => {
            return run_export_compliance(export_args).await;
        }
        Commands::Backup { command } => match command {
            BackupCommands::Create(args) => run_backup(args)
                .await
                .map_err(|e| Error::config_step("backup create", e)),
            BackupCommands::Restore(args) => run_restore(args)
                .await
                .map_err(|e| Error::config_step("backup restore", e)),
            BackupCommands::List(args) => run_list(args)
                .await
                .map_err(|e| Error::config_step("backup list", e)),
            BackupCommands::Cleanup(args) => run_cleanup(args)
                .await
                .map_err(|e| Error::config_step("backup cleanup", e)),
        },
    };

    version_check::check_and_notify(offline).await;
    result
}
