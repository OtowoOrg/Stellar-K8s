//! kubectl-stellar: A kubectl plugin for managing Stellar nodes
//!
//! This plugin provides convenient commands to interact with StellarNode resources:
//! - `kubectl stellar list` - List all StellarNode resources
//! - `kubectl stellar logs <node-name>` - Get logs from pods associated with a StellarNode
//! - `kubectl stellar status [node-name]` - Get sync status of StellarNode(s)

use std::process;

use clap::{Parser, Subcommand};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};

use stellar_k8s::controller::check_node_health;
use stellar_k8s::crd::types::ReplicationRole;
use stellar_k8s::crd::StellarNode;
use stellar_k8s::error::{Error, Result};

mod explain;

/// Helper function to get phase from node status, deriving from conditions if needed
fn get_node_phase(node: &StellarNode) -> String {
    node.status
        .as_ref()
        .map(|s| s.derive_phase_from_conditions())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[derive(Parser)]
#[command(name = "kubectl-stellar")]
#[command(about = "A kubectl plugin for managing Stellar nodes")]
#[command(long_about = "\
\x1b[1;36m  ✦ Stellar-K8s kubectl Plugin\x1b[0m\n\
\x1b[1;35m  Cloud-Native Stellar Infrastructure on Kubernetes\x1b[0m\n\
\x1b[90m  Built with Rust 🦀 · Powered by kube-rs · Apache 2.0\x1b[0m\n\n\
Manage StellarNode resources from the command line.\n\n\
EXAMPLES:\n  \
kubectl stellar list\n  \
kubectl stellar status my-validator\n  \
kubectl stellar logs my-validator -f\n  \
kubectl stellar list --dry-run\n  \
kubectl stellar status --dry-run")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Kubernetes namespace (defaults to current context namespace)
    #[arg(short, long, global = true)]
    namespace: Option<String>,

    /// Output format (table, json, yaml)
    #[arg(short, long, global = true, default_value = "table")]
    output: String,

    /// Simulate the command without making any state-changing API calls.
    ///
    /// Prints a summary of actions that would be taken without executing them.
    /// Safe to run against production clusters.
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Show version information for the plugin and operator
    Version,
    /// List all StellarNode resources
    List {
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
    },
    /// Get logs from pods associated with a StellarNode
    Logs {
        /// Name of the StellarNode
        node_name: String,
        /// Container name (if multiple containers in pod)
        #[arg(short, long)]
        container: Option<String>,
        /// Follow log output
        #[arg(short, long)]
        follow: bool,
        /// Number of lines to show from the end of logs
        #[arg(short, long, default_value = "100")]
        tail: i64,
    },
    /// Get sync status of StellarNode(s)
    Status {
        /// Name of a specific StellarNode (optional, shows all if omitted)
        node_name: Option<String>,
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
    },
    /// Stream Kubernetes events related to StellarNode resources
    Events {
        /// Name of a specific StellarNode (optional)
        node_name: Option<String>,
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
        /// Follow event updates in real time
        #[arg(short, long)]
        watch: bool,
    },
    /// Alias for status command
    #[command(name = "sync-status")]
    SyncStatus {
        node_name: Option<String>,
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
    },
    /// Debug a StellarNode by exec'ing into a diagnostic pod
    Debug {
        /// Name of the StellarNode
        node_name: String,
        /// Shell to use (default: /bin/bash)
        #[arg(short, long, default_value = "/bin/bash")]
        shell: String,
        /// Use ephemeral debug container instead of exec
        #[arg(short, long)]
        ephemeral: bool,
    },
    /// Explain a Stellar error code
    Explain {
        /// The Stellar error code to explain (e.g., tx_bad_auth, op_no_destination)
        error_code: String,
    },
    /// Search the documentation for keywords
    Search {
        /// The search query
        query: String,
        /// Show full content of the match
        #[arg(short, long)]
        full: bool,
    },
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Generate an incident report for a specific time window
    IncidentReport(stellar_k8s::incident::IncidentReportArgs),
    /// Trigger a failover to a secondary cluster
    Failover {
        /// Name of the StellarNode to failover
        node_name: String,
        /// Force failover even if the node is already active
        #[arg(short, long)]
        force: bool,
    },
    /// Execute a read-only SQL query against the node's internal database
    Sql {
        /// Name of the StellarNode
        node_name: String,
        /// SQL query to execute
        query: String,
    },
    /// Inspect compliance audit trails
    Audit {
        #[command(subcommand)]
        command: AuditCommands,
    },
    /// Show a high-level aggregate summary of all managed StellarNodes and their health
    Summary {
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
    },
    /// List pods with CVE vulnerabilities detected by the background scanner
    Cve {
        #[command(subcommand)]
        command: CveCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuditCommands {
    /// List recent audit entries
    List {
        /// Number of entries to show
        #[arg(short, long, default_value = "50")]
        limit: usize,
        /// Filter by resource name
        #[arg(short, long)]
        resource: Option<String>,
        /// Filter by actor
        #[arg(short, long)]
        actor: Option<String>,
        /// Output as JSON (suitable for automated security tools)
        #[arg(short, long)]
        json: bool,
    },
    /// Show detailed diff for a specific audit entry
    Show {
        /// Audit entry ID
        id: String,
        /// Output as JSON (suitable for automated security tools)
        #[arg(short, long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum CveCommands {
    /// List all pods with CVE vulnerabilities
    List {
        /// Minimum severity to show (critical, high, medium, low)
        #[arg(short, long, default_value = "high")]
        severity: String,
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
    },
}

mod audit_report;
mod sql;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    // Emit a dry-run notice for any command that would mutate cluster state.
    if cli.dry_run {
        let action = match &cli.command {
            Commands::List { .. }
            | Commands::Status { .. }
            | Commands::SyncStatus { .. }
            | Commands::Events { .. }
            | Commands::Version
            | Commands::Explain { .. }
            | Commands::Search { .. }
            | Commands::Completions { .. }
            | Commands::Summary { .. }
            | Commands::Cve { .. } => None,
            Commands::Logs { node_name, .. } => Some(format!(
                "Stream logs from StellarNode '{node_name}' (read-only, no cluster mutation)"
            )),
            Commands::Debug {
                node_name,
                ephemeral,
                ..
            } => {
                if *ephemeral {
                    Some(format!(
                        "Attach ephemeral debug container to StellarNode '{node_name}'"
                    ))
                } else {
                    Some(format!("Exec into pod for StellarNode '{node_name}'"))
                }
            }
            Commands::IncidentReport(_) => {
                Some("Generate incident report (read-only, no cluster mutation)".to_string())
            }
            Commands::Failover { node_name, .. } => {
                Some(format!("Trigger failover for StellarNode '{node_name}'"))
            }
            Commands::Sql { node_name, .. } => Some(format!(
                "Execute SQL query against StellarNode '{node_name}' (read-only)"
            )),
            Commands::Audit { .. } => {
                Some("Inspect compliance audit trails (read-only)".to_string())
            }
        };
        if let Some(desc) = action {
            println!("[dry-run] Would: {desc}");
            println!("[dry-run] No state-changing API calls were made.");
            return Ok(());
        }
    }

    match cli.command {
        Commands::Version => {
            let operator_version = match Client::try_default().await {
                Ok(client) => {
                    let deployments: kube::Api<k8s_openapi::api::apps::v1::Deployment> =
                        kube::Api::namespaced(client, "stellar-system");
                    match deployments.get("stellar-operator").await {
                        Ok(deploy) => {
                            // Prefer the well-known label set by Helm
                            deploy
                                .metadata
                                .labels
                                .as_ref()
                                .and_then(|l| l.get("app.kubernetes.io/version"))
                                .cloned()
                                // Fall back to parsing the image tag
                                .or_else(|| {
                                    deploy
                                        .spec
                                        .and_then(|s| s.template.spec)
                                        .and_then(|p| p.containers.into_iter().next())
                                        .and_then(|c| c.image)
                                        .and_then(|img| {
                                            img.rsplit_once(':').map(|(_, tag)| tag.to_string())
                                        })
                                })
                                .unwrap_or_else(|| "unknown".to_string())
                        }
                        Err(e) => format!("not deployed ({e})"),
                    }
                }
                Err(_) => "cluster not accessible".to_string(),
            };

            println!("kubectl-stellar v{}", env!("CARGO_PKG_VERSION"));
            println!("Operator version: {operator_version}");
            println!("Build Date: {}", env!("BUILD_DATE"));
            println!("Git SHA: {}", env!("GIT_SHA"));
            println!("Rust Version: {}", env!("RUST_VERSION"));
            Ok(())
        }
        Commands::List { all_namespaces } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            let namespace = if all_namespaces {
                None
            } else {
                Some(cli.namespace.as_deref().unwrap_or("default"))
            };
            list_nodes(&client, all_namespaces, namespace, &cli.output).await
        }
        Commands::Logs {
            node_name,
            container,
            follow,
            tail,
        } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            let namespace = cli.namespace.as_deref().unwrap_or("default");
            logs(
                &client,
                namespace,
                &node_name,
                container.as_deref(),
                follow,
                tail,
            )
            .await
        }
        Commands::Status {
            node_name,
            all_namespaces,
        } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            status(
                &client,
                node_name.as_deref(),
                all_namespaces,
                cli.namespace.as_deref(),
                &cli.output,
            )
            .await
        }
        Commands::Events {
            node_name,
            all_namespaces,
            watch,
        } => events(
            node_name.as_deref(),
            all_namespaces,
            cli.namespace.as_deref(),
            watch,
        ),
        Commands::SyncStatus {
            node_name,
            all_namespaces,
        } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            status(
                &client,
                node_name.as_deref(),
                all_namespaces,
                cli.namespace.as_deref(),
                &cli.output,
            )
            .await
        }
        Commands::Debug {
            node_name,
            shell,
            ephemeral,
        } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            let namespace = cli.namespace.as_deref().unwrap_or("default");
            debug(&client, namespace, &node_name, &shell, ephemeral).await
        }
        Commands::Explain { error_code } => {
            explain::explain_error(&error_code);
            Ok(())
        }
        Commands::Search { query, full } => search_docs(&query, full),
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
        Commands::IncidentReport(args) => stellar_k8s::incident::run_incident_report(args).await,
        Commands::Failover { node_name, force } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            failover(client, &node_name, force).await
        }
        Commands::Sql { node_name, query } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            let namespace = cli.namespace.as_deref().unwrap_or("default");
            let output_format = match cli.output.to_lowercase().as_str() {
                "json" => sql::OutputFormat::Json,
                "csv" => sql::OutputFormat::Csv,
                _ => sql::OutputFormat::Table,
            };

            let executor = sql::SqlExecutor::new(client, namespace.to_string());
            executor.execute(&node_name, &query, output_format).await
        }
        Commands::Audit { command } => {
            let bucket = std::env::var("STELLAR_AUDIT_BUCKET").map_err(|_| {
                Error::ConfigError(
                    "STELLAR_AUDIT_BUCKET environment variable must be set to access audit logs"
                        .to_string(),
                )
            })?;
            let prefix =
                std::env::var("STELLAR_AUDIT_PREFIX").unwrap_or_else(|_| "audit-logs/".to_string());

            let reporter = audit_report::AuditReporter::new(bucket, prefix).await;

            match command {
                AuditCommands::List {
                    limit,
                    resource,
                    actor,
                    json,
                } => reporter.list(limit, resource, actor, json).await,
                AuditCommands::Show { id, json } => reporter.show(&id, json).await,
            }
        }
        Commands::Summary { all_namespaces } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            summary(
                &client,
                all_namespaces,
                cli.namespace.as_deref(),
                &cli.output,
            )
            .await
        }
        Commands::Cve { command } => {
            let client = Client::try_default().await.map_err(Error::KubeError)?;
            match command {
                CveCommands::List {
                    severity,
                    all_namespaces,
                } => {
                    use stellar_k8s::controller::cve::VulnerabilitySeverity;
                    use stellar_k8s::controller::{list_vulnerable_pods, CveScannerConfig};

                    let min_severity = match severity.to_lowercase().as_str() {
                        "critical" => VulnerabilitySeverity::Critical,
                        "high" => VulnerabilitySeverity::High,
                        "medium" => VulnerabilitySeverity::Medium,
                        _ => VulnerabilitySeverity::Low,
                    };

                    let namespaces = if all_namespaces {
                        vec![]
                    } else {
                        cli.namespace
                            .as_deref()
                            .map(|ns| vec![ns.to_string()])
                            .unwrap_or_default()
                    };

                    let scanner_endpoint = std::env::var("TRIVY_API_ENDPOINT")
                        .unwrap_or_else(|_| "http://trivy-api.security-scanning:8080".to_string());

                    let config = CveScannerConfig {
                        scanner_endpoint,
                        namespaces,
                        ..Default::default()
                    };

                    let pods = list_vulnerable_pods(&client, &config, min_severity).await?;

                    if pods.is_empty() {
                        println!("No vulnerable pods found (min severity: {}).", severity);
                        return Ok(());
                    }

                    println!(
                        "{:<40} {:<20} {:<50} {:>8} {:>6} {:>6} {:>6}",
                        "NAMESPACE/POD", "IMAGE", "TAG", "CRITICAL", "HIGH", "MED", "LOW"
                    );
                    println!("{}", "-".repeat(140));

                    for pod in &pods {
                        let image_parts: Vec<&str> = pod.image.splitn(2, ':').collect();
                        let (img, tag) = if image_parts.len() == 2 {
                            (image_parts[0], image_parts[1])
                        } else {
                            (pod.image.as_str(), "latest")
                        };

                        let ns_pod = format!("{}/{}", pod.namespace, pod.pod_name);
                        println!(
                            "{:<40} {:<20} {:<50} {:>8} {:>6} {:>6} {:>6}",
                            ns_pod,
                            &img[img.len().saturating_sub(20)..],
                            &tag[..tag.len().min(50)],
                            pod.cve_count.critical,
                            pod.cve_count.high,
                            pod.cve_count.medium,
                            pod.cve_count.low,
                        );
                    }

                    println!("\nTotal: {} vulnerable pods", pods.len());
                    Ok(())
                }
            }
        }
    }
}

fn search_docs(query: &str, full: bool) -> Result<()> {
    use stellar_k8s::search;
    let results = search::search(query);

    if results.is_empty() {
        println!("No results found for '{query}'");
        return Ok(());
    }

    println!("Found {} results for '{}':\n", results.len(), query);

    for (doc, snippets) in results {
        println!("\x1b[1;34m{}\x1b[0m ({})", doc.title, doc.path);
        if full {
            println!("{}\n", doc.content);
        } else {
            for snippet in snippets {
                println!("  {snippet}\n");
            }
        }
    }

    Ok(())
}

fn build_events_field_selector(node_name: Option<&str>) -> String {
    let mut selectors = vec!["involvedObject.kind=StellarNode".to_string()];
    if let Some(name) = node_name {
        selectors.push(format!("involvedObject.name={name}"));
    }
    selectors.join(",")
}

fn events(
    node_name: Option<&str>,
    all_namespaces: bool,
    namespace: Option<&str>,
    watch: bool,
) -> Result<()> {
    let field_selector = build_events_field_selector(node_name);
    let mut cmd = std::process::Command::new("kubectl");
    cmd.arg("get").arg("events");

    if all_namespaces {
        cmd.arg("-A");
    } else {
        cmd.arg("-n").arg(namespace.unwrap_or("default"));
    }

    cmd.arg("--field-selector").arg(field_selector);
    cmd.arg("-o").arg("wide");

    if watch {
        cmd.arg("--watch");
    }

    let status = cmd
        .status()
        .map_err(|e| Error::ConfigError(format!("Failed to execute kubectl get events: {e}")))?;

    if !status.success() {
        return Err(Error::ConfigError(format!(
            "kubectl get events failed with exit code: {:?}",
            status.code()
        )));
    }

    Ok(())
}

/// Helper function to format nodes as JSON
fn format_nodes_json(nodes: &[StellarNode]) -> Result<String> {
    serde_json::to_string_pretty(nodes)
        .map_err(|e| Error::ConfigError(format!("JSON serialization error: {e}")))
}

/// Helper function to format nodes as YAML
fn format_nodes_yaml(nodes: &[StellarNode]) -> Result<String> {
    serde_yaml::to_string(nodes)
        .map_err(|e| Error::ConfigError(format!("YAML serialization error: {e}")))
}

/// Helper function to format node list as table
fn format_nodes_table(nodes: &[StellarNode], show_namespace: bool) {
    if show_namespace {
        println!(
            "{:<30} {:<15} {:<15} {:<10} {:<15} {:<10}",
            "NAME", "TYPE", "NETWORK", "REPLICAS", "PHASE", "NAMESPACE"
        );
        println!("{}", "-".repeat(95));
        for node in nodes {
            let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
            let name = node.name_any();
            let node_type = format!("{:?}", node.spec.node_type);
            let network = format!("{:?}", node.spec.network);
            let replicas = node.spec.replicas;
            let phase = get_node_phase(node);
            println!(
                "{name:<30} {node_type:<15} {network:<15} {replicas:<10} {phase:<15} {namespace:<10}"
            );
        }
    } else {
        println!(
            "{:<30} {:<15} {:<15} {:<10} {:<15}",
            "NAME", "TYPE", "NETWORK", "REPLICAS", "PHASE"
        );
        println!("{}", "-".repeat(85));
        for node in nodes {
            let name = node.name_any();
            let node_type = format!("{:?}", node.spec.node_type);
            let network = format!("{:?}", node.spec.network);
            let replicas = node.spec.replicas;
            let phase = get_node_phase(node);
            println!("{name:<30} {node_type:<15} {network:<15} {replicas:<10} {phase:<15}");
        }
    }
}

/// List all StellarNode resources
async fn list_nodes(
    client: &Client,
    all_namespaces: bool,
    namespace: Option<&str>,
    output: &str,
) -> Result<()> {
    let nodes = if all_namespaces {
        let api: Api<StellarNode> = Api::all(client.clone());
        api.list(&Default::default())
            .await
            .map_err(Error::KubeError)?
            .items
    } else {
        let ns = namespace.unwrap_or("default");
        let api: Api<StellarNode> = Api::namespaced(client.clone(), ns);
        api.list(&Default::default())
            .await
            .map_err(Error::KubeError)?
            .items
    };

    match output {
        "json" => {
            println!("{}", format_nodes_json(&nodes)?);
        }
        "yaml" => {
            println!("{}", format_nodes_yaml(&nodes)?);
        }
        _ => {
            format_nodes_table(&nodes, all_namespaces);
        }
    }

    Ok(())
}

/// Get logs from pods associated with a StellarNode
async fn logs(
    client: &Client,
    namespace: &str,
    node_name: &str,
    container: Option<&str>,
    follow: bool,
    tail: i64,
) -> Result<()> {
    // First, verify the StellarNode exists
    let node_api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    let _node = node_api.get(node_name).await.map_err(Error::KubeError)?;

    // Find pods using the same label selector as the controller
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let label_selector =
        format!("app.kubernetes.io/instance={node_name},app.kubernetes.io/name=stellar-node");

    let pods = pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await
        .map_err(Error::KubeError)?;

    if pods.items.is_empty() {
        return Err(Error::ConfigError(format!(
            "No pods found for StellarNode {namespace}/{node_name}"
        )));
    }

    // Get logs from pods (if multiple pods, show logs from all)
    // For StatefulSets (Validators), there's typically one pod
    // For Deployments (Horizon/Soroban), there may be multiple pods
    if pods.items.len() > 1 && !follow {
        println!("Found {} pods, showing logs from all:", pods.items.len());
    }

    // In follow mode, only follow the first pod
    if follow {
        let pod = &pods.items[0];
        let pod_name = pod.name_any();

        let mut cmd = std::process::Command::new("kubectl");
        cmd.arg("logs");
        cmd.arg("-n").arg(namespace);
        cmd.arg(&pod_name);

        if let Some(container_name) = container {
            cmd.arg("-c").arg(container_name);
        }

        cmd.arg("-f");
        cmd.arg("--tail").arg(tail.to_string());

        let status = cmd.status().map_err(|e| {
            Error::ConfigError(format!(
                "Failed to execute kubectl logs for pod {pod_name}: {e}"
            ))
        })?;

        if !status.success() {
            return Err(Error::ConfigError(format!(
                "kubectl logs failed for pod {} with exit code: {:?}",
                pod_name,
                status.code()
            )));
        }
    } else {
        // Non-follow mode: show logs from all pods
        for (idx, pod) in pods.items.iter().enumerate() {
            let pod_name = pod.name_any();

            if pods.items.len() > 1 {
                println!("\n=== Pod: {pod_name} ===");
            }

            // Use kubectl logs command via exec since kube-rs doesn't have a direct logs API
            // This is the standard way kubectl plugins handle logs
            let mut cmd = std::process::Command::new("kubectl");
            cmd.arg("logs");
            cmd.arg("-n").arg(namespace);
            cmd.arg(&pod_name);

            if let Some(container_name) = container {
                cmd.arg("-c").arg(container_name);
            }

            cmd.arg("--tail").arg(tail.to_string());

            let output = cmd.output().map_err(|e| {
                Error::ConfigError(format!(
                    "Failed to execute kubectl logs for pod #{} ({}): {}",
                    idx + 1,
                    pod_name,
                    e
                ))
            })?;

            if !output.status.success() {
                return Err(Error::ConfigError(format!(
                    "kubectl logs failed for pod #{} ({}): {}",
                    idx + 1,
                    pod_name,
                    String::from_utf8_lossy(&output.stderr)
                )));
            }

            print!("{}", String::from_utf8_lossy(&output.stdout));
        }
    }

    Ok(())
}

/// Get sync status of StellarNode(s)
async fn status(
    client: &Client,
    node_name: Option<&str>,
    all_namespaces: bool,
    namespace: Option<&str>,
    output: &str,
) -> Result<()> {
    let nodes = if let Some(name) = node_name {
        // Get specific node
        let ns = namespace.unwrap_or("default");
        let api: Api<StellarNode> = Api::namespaced(client.clone(), ns);
        let node = api.get(name).await.map_err(Error::KubeError)?;
        vec![node]
    } else if all_namespaces {
        // Get all nodes across all namespaces
        let api: Api<StellarNode> = Api::all(client.clone());
        let list = api
            .list(&Default::default())
            .await
            .map_err(Error::KubeError)?;
        list.items
    } else {
        // Get nodes in specified or default namespace
        let ns = namespace.unwrap_or("default");
        let api: Api<StellarNode> = Api::namespaced(client.clone(), ns);
        let list = api
            .list(&Default::default())
            .await
            .map_err(Error::KubeError)?;
        list.items
    };

    if nodes.is_empty() {
        println!("No StellarNode resources found.");
        return Ok(());
    }

    match output {
        "json" => {
            let mut results = Vec::new();
            for node in nodes {
                let health_result = check_node_health(client, &node, None).await?;
                results.push(serde_json::json!({
                    "name": node.name_any(),
                    "namespace": node.namespace().unwrap_or_else(|| "default".to_string()),
                    "type": format!("{:?}", node.spec.node_type),
                    "network": format!("{:?}", node.spec.network),
                    "phase": get_node_phase(&node),
                    "healthy": health_result.healthy,
                    "synced": health_result.synced,
                    "ledger_sequence": health_result.ledger_sequence,
                    "message": health_result.message,
                }));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&results)
                    .map_err(|e| Error::ConfigError(format!("JSON serialization error: {e}")))?
            );
        }
        "yaml" => {
            let mut results = Vec::new();
            for node in nodes {
                let health_result = check_node_health(client, &node, None).await?;
                results.push(serde_json::json!({
                    "name": node.name_any(),
                    "namespace": node.namespace().unwrap_or_else(|| "default".to_string()),
                    "type": format!("{:?}", node.spec.node_type),
                    "network": format!("{:?}", node.spec.network),
                    "phase": get_node_phase(&node),
                    "healthy": health_result.healthy,
                    "synced": health_result.synced,
                    "ledger_sequence": health_result.ledger_sequence,
                    "message": health_result.message,
                }));
            }
            println!(
                "{}",
                serde_yaml::to_string(&results)
                    .map_err(|e| Error::ConfigError(format!("YAML serialization error: {e}")))?
            );
        }
        _ => {
            // Table format
            // Show namespace column when viewing all namespaces OR when no specific node/namespace is specified
            let show_namespace = all_namespaces || (node_name.is_none() && namespace.is_none());

            if show_namespace {
                println!(
                    "{:<30} {:<15} {:<15} {:<10} {:<10} {:<10} {:<15} {:<20}",
                    "NAME", "NAMESPACE", "TYPE", "HEALTHY", "SYNCED", "LEDGER", "PHASE", "MESSAGE"
                );
                println!("{}", "-".repeat(125));
            } else {
                println!(
                    "{:<30} {:<15} {:<10} {:<10} {:<15} {:<20}",
                    "NAME", "TYPE", "HEALTHY", "SYNCED", "PHASE", "MESSAGE"
                );
                println!("{}", "-".repeat(100));
            }

            for node in nodes {
                let health_result = check_node_health(client, &node, None).await?;
                let name = node.name_any();
                let node_type = format!("{:?}", node.spec.node_type);
                let phase = get_node_phase(&node);
                let healthy = if health_result.healthy { "Yes" } else { "No" };
                let synced = if health_result.synced { "Yes" } else { "No" };
                let ledger = health_result
                    .ledger_sequence
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                let message = if health_result.message.len() > 17 {
                    format!("{}...", &health_result.message[..17])
                } else {
                    health_result.message.clone()
                };

                if show_namespace {
                    let node_namespace = node.namespace().unwrap_or_else(|| "default".to_string());
                    println!(
                        "{name:<30} {node_namespace:<15} {node_type:<15} {healthy:<10} {synced:<10} {ledger:<10} {phase:<15} {message:<20}"
                    );
                } else {
                    println!(
                        "{name:<30} {node_type:<15} {healthy:<10} {synced:<10} {phase:<15} {message:<20}"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Debug a StellarNode by exec'ing into a pod with diagnostic tools
async fn debug(
    client: &Client,
    namespace: &str,
    node_name: &str,
    shell: &str,
    ephemeral: bool,
) -> Result<()> {
    // First, verify the StellarNode exists
    let node_api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    let node = node_api.get(node_name).await.map_err(Error::KubeError)?;

    // Find pods using the same label selector as the controller
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let label_selector =
        format!("app.kubernetes.io/instance={node_name},app.kubernetes.io/name=stellar-node");

    let pods = pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await
        .map_err(Error::KubeError)?;

    if pods.items.is_empty() {
        return Err(Error::ConfigError(format!(
            "No pods found for StellarNode {namespace}/{node_name}"
        )));
    }

    // Use the first pod (for StatefulSets there's typically one, for Deployments we pick one)
    let pod = &pods.items[0];
    let pod_name = pod.name_any();

    println!("🔍 Debugging StellarNode: {node_name}");
    println!("📦 Pod: {pod_name}");
    println!("🌐 Namespace: {namespace}");
    println!("🔧 Node Type: {:?}", node.spec.node_type);
    println!();

    if ephemeral {
        // Use ephemeral debug container (requires Kubernetes 1.23+)
        println!("🚀 Starting ephemeral debug container with diagnostic tools...");
        println!();

        let mut cmd = std::process::Command::new("kubectl");
        cmd.arg("debug");
        cmd.arg("-n").arg(namespace);
        cmd.arg(&pod_name);
        cmd.arg("-it");
        cmd.arg("--image=nicolaka/netshoot:latest");
        cmd.arg("--target").arg("stellar-core"); // Target the main container
        cmd.arg("--");
        cmd.arg(shell);

        let status = cmd.status().map_err(|e| {
            Error::ConfigError(format!(
                "Failed to execute kubectl debug for pod {pod_name}: {e}"
            ))
        })?;

        if !status.success() {
            return Err(Error::ConfigError(format!(
                "kubectl debug failed for pod {} with exit code: {:?}",
                pod_name,
                status.code()
            )));
        }
    } else {
        // Regular exec into the existing container
        println!("🔌 Exec'ing into pod...");
        println!("💡 Available diagnostic commands:");
        println!("   - stellar-core --version");
        println!("   - stellar-core http-command 'info'");
        println!("   - stellar-core http-command 'peers'");
        println!("   - curl http://localhost:11626/info");
        println!("   - ps aux");
        println!("   - df -h");
        println!("   - netstat -tulpn");
        println!();

        // Determine the container name based on node type
        let container_name = match node.spec.node_type {
            stellar_k8s::crd::NodeType::Validator => "stellar-core",
            stellar_k8s::crd::NodeType::Horizon => "horizon",
            stellar_k8s::crd::NodeType::SorobanRpc => "soroban-rpc",
        };

        let mut cmd = std::process::Command::new("kubectl");
        cmd.arg("exec");
        cmd.arg("-n").arg(namespace);
        cmd.arg("-it");
        cmd.arg(&pod_name);
        cmd.arg("-c").arg(container_name);
        cmd.arg("--");
        cmd.arg(shell);

        let status = cmd.status().map_err(|e| {
            Error::ConfigError(format!(
                "Failed to execute kubectl exec for pod {pod_name}: {e}"
            ))
        })?;

        if !status.success() {
            return Err(Error::ConfigError(format!(
                "kubectl exec failed for pod {} with exit code: {:?}",
                pod_name,
                status.code()
            )));
        }
    }

    Ok(())
}

async fn failover(client: Client, node_name: &str, force: bool) -> Result<()> {
    let api: Api<StellarNode> = Api::default_namespaced(client);
    let mut node = api.get(node_name).await.map_err(Error::KubeError)?;

    let has_repl_cfg = node.spec.replication_config.is_some();
    if !has_repl_cfg {
        return Err(Error::ValidationError(format!(
            "Node '{node_name}' does not have replicationConfig configured"
        )));
    }

    let repl_cfg = node.spec.replication_config.as_mut().unwrap();

    if !repl_cfg.enabled {
        return Err(Error::ValidationError(format!(
            "Replication is not enabled for node '{node_name}'"
        )));
    }

    if repl_cfg.role == ReplicationRole::Active && !force {
        println!("Node '{node_name}' is already in Active role. Use --force to re-apply.");
        return Ok(());
    }

    println!("Triggering failover for StellarNode '{node_name}'...");
    repl_cfg.role = ReplicationRole::Active;

    let patch = Patch::Merge(&node);
    api.patch(node_name, &PatchParams::default(), &patch)
        .await
        .map_err(Error::KubeError)?;

    println!("Successfully updated node '{node_name}' role to Active.");
    println!("The operator will now reconfigure the database and history archives for primary operation.");

    Ok(())
}

/// Aggregate counts used by the summary command
#[derive(Debug, Default)]
pub struct SummaryStats {
    pub total: usize,
    pub healthy: usize,
    pub synced: usize,
    pub degraded: usize,
    pub pending: usize,
    pub by_type: std::collections::HashMap<String, usize>,
    pub by_network: std::collections::HashMap<String, usize>,
}

/// Show a high-level aggregate summary of all managed StellarNodes
async fn summary(
    client: &Client,
    all_namespaces: bool,
    namespace: Option<&str>,
    output: &str,
) -> Result<()> {
    let nodes = if all_namespaces {
        let api: Api<StellarNode> = Api::all(client.clone());
        api.list(&Default::default())
            .await
            .map_err(Error::KubeError)?
            .items
    } else {
        let ns = namespace.unwrap_or("default");
        let api: Api<StellarNode> = Api::namespaced(client.clone(), ns);
        api.list(&Default::default())
            .await
            .map_err(Error::KubeError)?
            .items
    };

    let mut stats = SummaryStats {
        total: nodes.len(),
        ..Default::default()
    };

    for node in &nodes {
        let health = check_node_health(client, node, None).await?;
        if health.healthy {
            stats.healthy += 1;
        }
        if health.synced {
            stats.synced += 1;
        }
        let phase = get_node_phase(node);
        if phase == "Degraded" || phase == "Failed" {
            stats.degraded += 1;
        } else if phase == "Pending" || phase == "Creating" {
            stats.pending += 1;
        }
        *stats
            .by_type
            .entry(format!("{:?}", node.spec.node_type))
            .or_insert(0) += 1;
        *stats
            .by_network
            .entry(format!("{:?}", node.spec.network))
            .or_insert(0) += 1;
    }

    match output {
        "json" => {
            let mut by_type: Vec<_> = stats.by_type.iter().collect();
            by_type.sort_by_key(|(k, _)| k.as_str());
            let mut by_network: Vec<_> = stats.by_network.iter().collect();
            by_network.sort_by_key(|(k, _)| k.as_str());
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "total": stats.total,
                    "healthy": stats.healthy,
                    "synced": stats.synced,
                    "degraded": stats.degraded,
                    "pending": stats.pending,
                    "by_type": by_type.into_iter().collect::<std::collections::HashMap<_,_>>(),
                    "by_network": by_network.into_iter().collect::<std::collections::HashMap<_,_>>(),
                }))
                .map_err(|e| Error::ConfigError(format!("JSON serialization error: {e}")))?
            );
        }
        "yaml" => {
            let mut by_type: Vec<_> = stats.by_type.iter().collect();
            by_type.sort_by_key(|(k, _)| k.as_str());
            let mut by_network: Vec<_> = stats.by_network.iter().collect();
            by_network.sort_by_key(|(k, _)| k.as_str());
            println!(
                "{}",
                serde_yaml::to_string(&serde_json::json!({
                    "total": stats.total,
                    "healthy": stats.healthy,
                    "synced": stats.synced,
                    "degraded": stats.degraded,
                    "pending": stats.pending,
                    "by_type": by_type.into_iter().collect::<std::collections::HashMap<_,_>>(),
                    "by_network": by_network.into_iter().collect::<std::collections::HashMap<_,_>>(),
                }))
                .map_err(|e| Error::ConfigError(format!("YAML serialization error: {e}")))?
            );
        }
        _ => {
            println!("StellarNode Summary");
            println!("{}", "=".repeat(40));
            println!("  Total nodes : {}", stats.total);
            println!("  Healthy     : {}", stats.healthy);
            println!("  Synced      : {}", stats.synced);
            println!("  Degraded    : {}", stats.degraded);
            println!("  Pending     : {}", stats.pending);
            println!();
            println!("By Type:");
            let mut by_type: Vec<_> = stats.by_type.iter().collect();
            by_type.sort_by_key(|(k, _)| k.as_str());
            for (t, count) in &by_type {
                println!("  {t:<15} : {count}");
            }
            println!();
            println!("By Network:");
            let mut by_network: Vec<_> = stats.by_network.iter().collect();
            by_network.sort_by_key(|(k, _)| k.as_str());
            for (n, count) in &by_network {
                println!("  {n:<15} : {count}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::ObjectMeta;
    use stellar_k8s::controller::conditions::{CONDITION_STATUS_TRUE, CONDITION_TYPE_READY};
    use stellar_k8s::crd::{Condition, NodeType, StellarNodeSpec, StellarNodeStatus};

    #[allow(deprecated)]
    fn create_test_node(name: &str, namespace: &str, node_type: NodeType) -> StellarNode {
        use chrono::Utc;
        use stellar_k8s::crd::StellarNetwork;

        // Create a Ready condition so derive_phase_from_conditions() returns "Ready"
        let ready_condition = Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_TRUE.to_string(),
            last_transition_time: Utc::now().to_rfc3339(),
            reason: "AllSubresourcesHealthy".to_string(),
            message: "All sub-resources are healthy and operational".to_string(),
            observed_generation: None,
        };

        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                replicas: 1,
                resources: Default::default(),
                storage: Default::default(),
                validator_config: None,
                horizon_config: None,
                soroban_config: None,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                pod_anti_affinity: Default::default(),
                placement: Default::default(),
                topology_spread_constraints: None,
                cve_handling: None,
                read_replica_config: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                forensic_snapshot: None,
                label_propagation: None,
                resource_meta: None,
                read_pool_endpoint: None,
                sidecars: None,
                cert_manager: None,
                history_mode: Default::default(),
                custom_network_passphrase: None,
                nat_traversal: None,
                ..Default::default()
            },
            status: Some(StellarNodeStatus {
                #[allow(deprecated)]
                phase: "Ready".to_string(), // Keep for backward compatibility, but not used
                conditions: vec![ready_condition],
                ready_replicas: 1,
                replicas: 1,
                canary_ready_replicas: 0,
                canary_version: None,
                canary_start_time: None,
                last_migrated_version: None,
                ledger_updated_at: None,
                quorum_fragility: None,
                quorum_analysis_timestamp: None,
                vault_observed_secret_version: None,
                forensic_snapshot_phase: None,
                label_propagation_status: None,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn test_format_nodes_json() {
        let nodes = vec![
            create_test_node("node1", "default", NodeType::Validator),
            create_test_node("node2", "default", NodeType::Horizon),
        ];

        let result = format_nodes_json(&nodes);
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.contains("node1"));
        assert!(json.contains("node2"));
        assert!(json.contains("Validator"));
        assert!(json.contains("Horizon"));
    }

    #[test]
    fn test_format_nodes_yaml() {
        let nodes = vec![create_test_node("node1", "default", NodeType::Validator)];

        let result = format_nodes_yaml(&nodes);
        assert!(result.is_ok());
        let yaml = result.unwrap();
        assert!(yaml.contains("node1"));
        assert!(yaml.contains("Validator"));
    }

    #[test]
    fn test_format_nodes_table_with_namespace() {
        let nodes = vec![
            create_test_node("node1", "ns1", NodeType::Validator),
            create_test_node("node2", "ns2", NodeType::Horizon),
        ];

        // Test that function doesn't panic
        format_nodes_table(&nodes, true);
    }

    #[test]
    fn test_format_nodes_table_without_namespace() {
        let nodes = vec![create_test_node("node1", "default", NodeType::Validator)];

        format_nodes_table(&nodes, false);
    }

    #[test]
    fn test_status_table_condition_consistency() {
        // Test that the condition for showing namespace is consistent
        // show_namespace = all_namespaces || (node_name.is_none() && namespace.is_none())
        let test_cases = vec![
            (true, None, None, true),           // all_namespaces=true -> show namespace
            (false, None, None, true), // node_name=None && namespace=None -> show namespace
            (false, Some("node"), None, false), // node_name=Some && namespace=None -> hide namespace
            (false, None, Some("ns"), false), // node_name=None && namespace=Some -> hide namespace
            (false, Some("node"), Some("ns"), false), // both Some -> hide namespace
        ];

        for (all_namespaces, node_name, namespace, expected_show) in test_cases {
            let show_namespace = all_namespaces || (node_name.is_none() && namespace.is_none());
            assert_eq!(
                show_namespace, expected_show,
                "Failed for all_namespaces={all_namespaces:?}, node_name={node_name:?}, namespace={namespace:?}"
            );
        }
    }

    #[test]
    fn test_image_tag_fallback_parsing() {
        // Simulates the fallback: extract tag from image string
        let image = "ghcr.io/stellar/stellar-k8s:v1.2.3";
        let tag = image.rsplit_once(':').map(|(_, t)| t.to_string());
        assert_eq!(tag, Some("v1.2.3".to_string()));

        let no_tag = "ghcr.io/stellar/stellar-k8s";
        assert!(no_tag.rsplit_once(':').is_none());
    }

    #[test]
    fn test_build_events_field_selector_all_nodes() {
        let selector = build_events_field_selector(None);
        assert_eq!(selector, "involvedObject.kind=StellarNode");
    }

    #[test]
    fn test_build_events_field_selector_specific_node() {
        let selector = build_events_field_selector(Some("validator-a"));
        assert_eq!(
            selector,
            "involvedObject.kind=StellarNode,involvedObject.name=validator-a"
        );
    }

    // --- Summary stats aggregation tests ---

    fn make_stats_from_nodes(nodes: &[StellarNode]) -> SummaryStats {
        let mut stats = SummaryStats {
            total: nodes.len(),
            ..Default::default()
        };
        for node in nodes {
            let phase = get_node_phase(node);
            if phase == "Degraded" || phase == "Failed" {
                stats.degraded += 1;
            } else if phase == "Pending" || phase == "Creating" {
                stats.pending += 1;
            }
            *stats
                .by_type
                .entry(format!("{:?}", node.spec.node_type))
                .or_insert(0) += 1;
            *stats
                .by_network
                .entry(format!("{:?}", node.spec.network))
                .or_insert(0) += 1;
        }
        stats
    }

    #[test]
    fn test_summary_stats_empty() {
        let stats = make_stats_from_nodes(&[]);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.healthy, 0);
        assert_eq!(stats.synced, 0);
        assert_eq!(stats.degraded, 0);
        assert!(stats.by_type.is_empty());
        assert!(stats.by_network.is_empty());
    }

    #[test]
    fn test_summary_stats_counts_by_type() {
        let nodes = vec![
            create_test_node("v1", "default", NodeType::Validator),
            create_test_node("v2", "default", NodeType::Validator),
            create_test_node("h1", "default", NodeType::Horizon),
        ];
        let stats = make_stats_from_nodes(&nodes);
        assert_eq!(stats.total, 3);
        assert_eq!(stats.by_type["Validator"], 2);
        assert_eq!(stats.by_type["Horizon"], 1);
    }

    #[test]
    fn test_summary_stats_ready_nodes_not_degraded() {
        let nodes = vec![create_test_node("v1", "default", NodeType::Validator)];
        let stats = make_stats_from_nodes(&nodes);
        // Ready nodes should not be counted as degraded or pending
        assert_eq!(stats.degraded, 0);
        assert_eq!(stats.pending, 0);
    }

    #[test]
    fn test_summary_stats_by_network() {
        use stellar_k8s::crd::StellarNetwork;
        let mut node = create_test_node("v1", "default", NodeType::Validator);
        node.spec.network = StellarNetwork::Mainnet;
        let nodes = vec![
            node,
            create_test_node("v2", "default", NodeType::Validator), // Testnet
        ];
        let stats = make_stats_from_nodes(&nodes);
        assert_eq!(stats.by_network["Mainnet"], 1);
        assert_eq!(stats.by_network["Testnet"], 1);
    }
}
