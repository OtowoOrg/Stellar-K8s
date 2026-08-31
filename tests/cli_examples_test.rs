// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Command-level smoke tests for documented CLI example commands.
//!
//! This test validates that all CLI examples from `docs/cli-commands-reference.md`
//! parse correctly and are executable. It ensures documentation stays in sync
//! with the actual CLI interface.
//!
//! Related: #1154 - Add pipeline stage that validates every documented CLI example command

use clap::Parser;
use stellar_k8s::cli::{Args, Commands, RunArgs, SimulatorUpArgs, WebhookArgs};

/// Parse a command as if invoking from the CLI
fn parse_command(args: &[&str]) -> Result<Args, clap::Error> {
    Args::try_parse_from(args)
}

#[test]
fn run_examples_parse() {
    // From docs/cli-commands-reference.md - "Development" section
    let examples = vec![
        vec!["stellar-operator", "run", "--namespace", "stellar-system"],
        vec![
            "stellar-operator",
            "run",
            "--enable-mtls",
            "--namespace",
            "stellar-system",
        ],
        vec!["stellar-operator", "run", "--dry-run"],
    ];

    for example in examples {
        let parsed = parse_command(&example).unwrap_or_else(|e| {
            panic!(
                "Failed to parse documented example: {:?}\nError: {}",
                example, e
            );
        });
        if let Commands::Run(args) = parsed.command {
            println!("✓ Parsed: {:?}", example);
            // Validate args are sensible defaults
            match example.as_slice() {
                ["stellar-operator", "run", "--namespace", ns] => {
                    assert_eq!(args.namespace, *ns);
                }
                ["stellar-operator", "run", "--enable-mtls", "--namespace", ns] => {
                    assert!(args.enable_mtls);
                    assert_eq!(args.namespace, *ns);
                }
                ["stellar-operator", "run", "--dry-run"] => {
                    assert!(args.dry_run);
                }
                _ => {}
            }
        } else {
            panic!("Expected Run subcommand for: {:?}", example);
        }
    }
}

#[test]
fn webhook_examples_parse() {
    // From docs/cli-commands-reference.md - "Production Deployment" section
    let example = vec![
        "stellar-operator",
        "webhook",
        "--bind",
        "0.0.0.0:8443",
        "--cert-path",
        "/tls/tls.crt",
        "--key-path",
        "/tls/tls.key",
    ];

    let parsed = parse_command(&example).unwrap();
    if let Commands::Webhook(args) = parsed.command {
        assert_eq!(args.bind, "0.0.0.0:8443");
        assert_eq!(args.cert_path.as_deref(), Some("/tls/tls.crt"));
        assert_eq!(args.key_path.as_deref(), Some("/tls/tls.key"));
        println!("✓ Webhook example parses correctly");
    } else {
        panic!("Expected Webhook subcommand");
    }
}

#[test]
fn info_example_parses() {
    let example = vec!["stellar-operator", "info", "--namespace", "stellar-system"];
    let parsed = parse_command(&example).unwrap();
    if let Commands::Info(args) = parsed.command {
        assert_eq!(args.namespace, "stellar-system");
        println!("✓ Info example parses correctly");
    } else {
        panic!("Expected Info subcommand");
    }
}

#[test]
fn generate_runbook_example_parses() {
    let example = vec![
        "stellar-operator",
        "generate-runbook",
        "validator-1",
        "--namespace",
        "stellar-system",
        "--output",
        "runbook.md",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::GenerateRunbook(args) = parsed.command {
        assert_eq!(args.node_name, "validator-1");
        assert_eq!(args.namespace, "stellar-system");
        assert_eq!(args.output.as_deref(), Some("runbook.md"));
        println!("✓ Generate-runbook example parses correctly");
    } else {
        panic!("Expected GenerateRunbook subcommand");
    }
}

#[test]
fn incident_report_example_parses() {
    let example = vec![
        "stellar-operator",
        "incident",
        "report",
        "--namespace",
        "stellar-system",
        "--from",
        "2024-01-15T10:00:00Z",
        "--to",
        "2024-01-15T11:00:00Z",
        "--output",
        "incident.zip",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::Incident { command } = parsed.command {
        if let stellar_k8s::incident::IncidentCommands::Report(_) = command {
            println!("✓ Incident report example parses correctly");
        } else {
            panic!("Expected Report subcommand");
        }
    } else {
        panic!("Expected Incident subcommand");
    }
}

#[test]
fn simulator_up_examples_parse() {
    // Example 1: defaults
    let example1 = vec!["stellar-operator", "simulator", "up"];
    let parsed = parse_command(&example1).unwrap();
    if let Commands::Simulator(sim) = parsed.command {
        if let stellar_k8s::cli::SimulatorCmd::Up(args) = sim.command {
            assert_eq!(args.cluster_name, "stellar-sim");
            assert_eq!(args.namespace, "stellar-system");
            assert!(!args.use_k3s);
            println!("✓ Simulator up (defaults) parses correctly");
        }
    }

    // Example 2: custom cluster
    let example2 = vec![
        "stellar-operator",
        "simulator",
        "up",
        "--cluster-name",
        "my-cluster",
    ];
    let parsed = parse_command(&example2).unwrap();
    if let Commands::Simulator(sim) = parsed.command {
        if let stellar_k8s::cli::SimulatorCmd::Up(args) = sim.command {
            assert_eq!(args.cluster_name, "my-cluster");
            println!("✓ Simulator up (custom cluster) parses correctly");
        }
    }

    // Example 3: k3s
    let example3 = vec!["stellar-operator", "simulator", "up", "--use-k3s"];
    let parsed = parse_command(&example3).unwrap();
    if let Commands::Simulator(sim) = parsed.command {
        if let stellar_k8s::cli::SimulatorCmd::Up(args) = sim.command {
            assert!(args.use_k3s);
            println!("✓ Simulator up (k3s) parses correctly");
        }
    }
}

#[test]
fn completions_examples_parse() {
    let shells = vec!["bash", "zsh", "fish", "powershell", "elvish"];
    for shell in shells {
        let example = vec!["stellar-operator", "completions", shell];
        let parsed = parse_command(&example).unwrap();
        if let Commands::Completions { shell: s } = parsed.command {
            assert_eq!(s.to_string(), shell);
            println!("✓ Completions {} parses correctly", shell);
        } else {
            panic!("Expected Completions subcommand for shell: {}", shell);
        }
    }
}

#[test]
fn install_completion_examples_parse() {
    let shells = vec!["bash", "zsh", "fish"];
    for shell in shells {
        let example = vec!["stellar-operator", "install-completion", shell];
        let parsed = parse_command(&example).unwrap();
        if let Commands::InstallCompletion { shell: s } = parsed.command {
            assert_eq!(s.to_string(), shell);
            println!("✓ Install-completion {} parses correctly", shell);
        } else {
            panic!("Expected InstallCompletion subcommand for shell: {}", shell);
        }
    }
}

#[test]
fn benchmark_examples_parse() {
    let example = vec![
        "stellar-operator",
        "benchmark",
        "--namespace",
        "stellar-system",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::Benchmark(args) = parsed.command {
        assert_eq!(args.namespace, "stellar-system");
        println!("✓ Benchmark example parses correctly");
    } else {
        panic!("Expected Benchmark subcommand");
    }
}

#[test]
fn benchmark_compare_examples_parse() {
    // From docs/cli-commands-reference.md
    let example = vec![
        "stellar-operator",
        "benchmark-compare",
        "--cluster-a-context",
        "prod",
        "--cluster-b-context",
        "staging",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::BenchmarkCompare(args) = parsed.command {
        assert_eq!(args.cluster_a_context, Some("prod".to_string()));
        assert_eq!(args.cluster_b_context, Some("staging".to_string()));
        println!("✓ Benchmark-compare example parses correctly");
    } else {
        panic!("Expected BenchmarkCompare subcommand");
    }
}

#[test]
fn prune_archive_example_parses() {
    let example = vec![
        "stellar-operator",
        "prune-archive",
        "--archive-url",
        "s3://stellar-history-prod/archive",
        "--min-checkpoints",
        "100",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::PruneArchive(args) = parsed.command {
        assert_eq!(args.archive_url, "s3://stellar-history-prod/archive");
        assert_eq!(args.min_checkpoints, 100);
        println!("✓ Prune-archive example parses correctly");
    } else {
        panic!("Expected PruneArchive subcommand");
    }
}

#[test]
fn diff_example_parses() {
    let example = vec![
        "stellar-operator",
        "diff",
        "--namespace",
        "stellar-system",
        "--name",
        "validator-1",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::Diff(args) = parsed.command {
        assert_eq!(args.namespace, "stellar-system");
        assert_eq!(args.name, "validator-1");
        println!("✓ Diff example parses correctly");
    } else {
        panic!("Expected Diff subcommand");
    }
}

#[test]
fn check_crd_subcommand_parses() {
    let example = vec!["stellar-operator", "check-crd"];
    let parsed = parse_command(&example).unwrap();
    assert!(matches!(parsed.command, Commands::CheckCrd));
    println!("✓ Check-crd subcommand parses correctly");
}

#[test]
fn doctor_subcommand_parses() {
    let example = vec!["stellar-operator", "doctor", "--namespace", "stellar"];
    let parsed = parse_command(&example).unwrap();
    if let Commands::Doctor(args) = parsed.command {
        assert_eq!(args.namespace, "stellar");
        println!("✓ Doctor subcommand parses correctly");
    } else {
        panic!("Expected Doctor subcommand");
    }
}

#[test]
fn version_subcommand_parses() {
    let example = vec!["stellar-operator", "version"];
    let parsed = parse_command(&example).unwrap();
    assert!(matches!(parsed.command, Commands::Version));
    println!("✓ Version subcommand parses correctly");
}

#[test]
fn export_compliance_example_parses() {
    let example = vec![
        "stellar-operator",
        "export-compliance",
        "--format",
        "json",
        "--namespace",
        "stellar-system",
        "--limit",
        "100",
    ];
    let parsed = parse_command(&example).unwrap();
    if let Commands::ExportCompliance(args) = parsed.command {
        assert_eq!(args.format, "json");
        assert_eq!(args.namespace, "stellar-system");
        assert_eq!(args.limit, 100);
        println!("✓ Export-compliance example parses correctly");
    } else {
        panic!("Expected ExportCompliance subcommand");
    }
}

#[test]
fn backup_restore_list_cleanup_parse() {
    // Test backup create
    let backup_example = vec![
        "stellar-operator",
        "backup",
        "create",
        "--source",
        "/data",
        "--backend",
        "file",
        "--destination",
        "/backups",
    ];
    let parsed = parse_command(&backup_example).unwrap();
    if let Commands::Backup { command } = parsed.command {
        if let stellar_k8s::cli::BackupCommands::Create(args) = command {
            assert_eq!(args.source, std::path::PathBuf::from("/data"));
            assert_eq!(args.backend, "file");
            assert_eq!(args.destination, "/backups");
            println!("✓ Backup create example parses correctly");
        }
    } else {
        panic!("Expected Backup subcommand");
    }

    // Test backup restore
    let restore_example = vec![
        "stellar-operator",
        "backup",
        "restore",
        "--backup",
        "backup-20240101.tar.gz",
        "--destination",
        "/restore",
    ];
    let parsed = parse_command(&restore_example).unwrap();
    if let Commands::Backup { command } = parsed.command {
        if let stellar_k8s::cli::BackupCommands::Restore(args) = command {
            assert_eq!(args.backup, "backup-20240101.tar.gz");
            assert_eq!(args.destination, std::path::PathBuf::from("/restore"));
            println!("✓ Backup restore example parses correctly");
        }
    } else {
        panic!("Expected Backup restore subcommand");
    }

    // Test backup list
    let list_example = vec![
        "stellar-operator",
        "backup",
        "list",
        "--location",
        "/backups",
    ];
    let parsed = parse_command(&list_example).unwrap();
    if let Commands::Backup { command } = parsed.command {
        if let stellar_k8s::cli::BackupCommands::List(args) = command {
            assert_eq!(args.location, "/backups");
            println!("✓ Backup list example parses correctly");
        }
    } else {
        panic!("Expected Backup list subcommand");
    }

    // Test backup cleanup
    let cleanup_example = vec![
        "stellar-operator",
        "backup",
        "cleanup",
        "--location",
        "/backups",
        "--keep",
        "5",
    ];
    let parsed = parse_command(&cleanup_example).unwrap();
    if let Commands::Backup { command } = parsed.command {
        if let stellar_k8s::cli::BackupCommands::Cleanup(args) = command {
            assert_eq!(args.location, "/backups");
            assert_eq!(args.keep, 5);
            println!("✓ Backup cleanup example parses correctly");
        }
    } else {
        panic!("Expected Backup cleanup subcommand");
    }
}

#[test]
fn offline_flag_global_position() {
    // From docs: global flag can come before or after subcommand
    let before = parse_command(&["stellar-operator", "--offline", "version"]).unwrap();
    assert!(before.offline);

    let after = parse_command(&["stellar-operator", "check-crd", "--offline"]).unwrap();
    assert!(after.offline);
    println!("✓ Offline flag works globally");
}

#[test]
fn invalid_command_fails() {
    let result = parse_command(&["stellar-operator", "nonexistent-command"]);
    assert!(result.is_err(), "Unknown commands should fail");
    println!("✓ Invalid commands are rejected");
}

use assert_cmd::Command;

#[test]
fn test_cli_help_examples() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn test_cli_run_command_accepted() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["run", "--help"])
        .assert()
        .success();
}

#[test]
fn test_cli_webhook_command_accepted() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["webhook", "--help"])
        .assert()
        .success();
}

#[test]
fn test_cli_info_command_accepted() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["info", "--help"])
        .assert()
        .success();
}

#[test]
fn test_kubectl_stellar_list_command() {
    Command::cargo_bin("kubectl-stellar")
        .unwrap()
        .args(["list", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stellar_operator_benchmark_command() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["benchmark", "--help"])
        .assert()
        .success();
}

#[test]
fn test_backup_subcommand() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "--help"])
        .assert()
        .success();
}

#[test]
fn test_backup_restore_subcommand() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "restore", "--help"])
        .assert()
        .success();
}

#[test]
fn test_simulator_subcommand() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["simulator", "--help"])
        .assert()
        .success();
}

#[test]
fn test_completions_subcommand() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["completions", "--help"])
        .assert()
        .success();
}

#[test]
fn test_cli_commands_documentation_coverage() {
    use clap::CommandFactory;
    use std::fs;

    // Get all subcommands from the clap Args struct
    let cmd = stellar_k8s::cli::Args::command();
    let subcommands: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();

    // Read the reference documentation
    let doc_content = fs::read_to_string("docs/cli-commands-reference.md")
        .expect("Failed to read docs/cli-commands-reference.md");

    let mut documented_count = 0;
    let mut undocumented = Vec::new();

    for sub in &subcommands {
        // Special case mappings where documentation name is slightly different
        let doc_names = match sub.as_str() {
            "incident" => vec!["incident", "incident-report"],
            other => vec![other],
        };

        let is_documented = doc_names.iter().any(|name| {
            // Check for markdown headers like: "### name" or "# name" or similar mentions
            let pattern1 = format!("### {}", name);
            let pattern2 = format!("`stellar-operator {}` ", name);
            let pattern3 = format!("`stellar-operator {}`\n", name);
            let pattern4 = format!("stellar-operator {} ", name);
            doc_content.contains(&pattern1)
                || doc_content.contains(&pattern2)
                || doc_content.contains(&pattern3)
                || doc_content.contains(&pattern4)
        });

        if is_documented {
            documented_count += 1;
        } else {
            undocumented.push(sub.clone());
        }
    }

    let total_subcommands = subcommands.len();
    let coverage_ratio = documented_count as f64 / total_subcommands as f64;
    println!("Documented: {}/{}", documented_count, total_subcommands);
    println!("Coverage: {:.2}%", coverage_ratio * 100.0);
    if !undocumented.is_empty() {
        println!("Undocumented subcommands: {:?}", undocumented);
    }

    // Assert that docs coverage threshold meets 75%
    assert!(
        coverage_ratio >= 0.75,
        "CLI command documentation coverage is only {:.2}%, which is below the 75% threshold. Undocumented subcommands: {:?}",
        coverage_ratio * 100.0,
        undocumented
    );
}
