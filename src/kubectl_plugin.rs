//! kubectl-stellar: A kubectl plugin for managing Stellar nodes
//!
//! This plugin provides convenient commands to interact with StellarNode resources:
//! - `kubectl stellar list` - List all StellarNode resources
//! - `kubectl stellar logs <node-name>` - Get logs from pods associated with a StellarNode
//! - `kubectl stellar status [node-name]` - Get sync status of StellarNode(s)

use std::process;

use clap::{Parser, Subcommand};
use k8s_openapi::api::core::v1::Pod;
use kube::{api::Api, Client, ResourceExt};

use stellar_k8s::controller::check_node_health;
use stellar_k8s::crd::StellarNode;
use stellar_k8s::error::{Error, Result};

/// Helper function to get phase from node status, deriving from conditions if needed
fn get_node_phase(node: &StellarNode) -> String {
    node.status
        .as_ref()
        .map(|s| s.derive_phase_from_conditions())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[derive(Parser)]
#[command(name = "kubectl-stellar")]
#[command(about = "A kubectl plugin for managing Stellar nodes", long_about = None)]
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
}

#[derive(Subcommand)]
enum Commands {
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
    /// Alias for status command
    #[command(name = "sync-status")]
    SyncStatus {
        /// Name of a specific StellarNode (optional, shows all if omitted)
        node_name: Option<String>,
        /// Show all namespaces
        #[arg(short = 'A', long)]
        all_namespaces: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let client = Client::try_default().await.map_err(Error::KubeError)?;

    match cli.command {
        Commands::List { all_namespaces } => {
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
            status(
                &client,
                node_name.as_deref(),
                all_namespaces,
                cli.namespace.as_deref(),
                &cli.output,
            )
            .await
        }
        Commands::SyncStatus {
            node_name,
            all_namespaces,
        } => {
            status(
                &client,
                node_name.as_deref(),
                all_namespaces,
                cli.namespace.as_deref(),
                &cli.output,
            )
            .await
        }
    }
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
                history_mode: Default::default(),
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
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                topology_spread_constraints: None,
                resource_meta: None,
                load_balancer: None,
                global_discovery: None,
                cluster: None,
                cross_cluster: None,
            },
            status: Some(StellarNodeStatus {
                #[allow(deprecated)]
                phase: "Ready".to_string(), // Keep for backward compatibility, but not used
                conditions: vec![ready_condition],
                observed_generation: None,
                message: None,
                dr_status: None,
                ledger_sequence: None,
                endpoint: None,
                external_ip: None,
                bgp_status: None,
                ready_replicas: 1,
                replicas: 1,
                canary_ready_replicas: 0,
                canary_version: None,
                last_migrated_version: None,
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
                "Failed for all_namespaces={:?}, node_name={:?}, namespace={:?}",
                all_namespaces, node_name, namespace
            );
        }
    }
}
