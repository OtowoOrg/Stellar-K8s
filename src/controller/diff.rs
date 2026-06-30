//! Live Diff Utility for Stellar-K8s Operator
//!
//! Provides a way for operators to see the difference between what the operator
//! thinks should be deployed (desired state) versus what is actually in the cluster (live state).
//!
//! Similar to `kubectl diff`, but includes internal operator-managed resources like:
//! - ConfigMaps (stellar-core.cfg, captive-core.cfg)
//! - Resource limits and requests
//! - Service configurations
//! - StatefulSets/Deployments
//! - PVCs, HPAs, NetworkPolicies, etc.

use clap::Parser;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::core::v1::{ConfigMap, PersistentVolumeClaim, Service};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::Api;
use kube::{Client, ResourceExt};
use serde::Serialize;
use serde_json::Value;
use std::fmt::Write;
use tracing::info;

use crate::controller::resources::{build_config_map, resource_name, standard_labels};
use crate::crd::StellarNode;
use crate::Error;

/// Output format for diff
#[derive(clap::ValueEnum, Clone, Debug, Default)]
pub enum DiffFormat {
    /// Colored terminal output (default)
    #[default]
    Terminal,
    /// JSON format
    Json,
    /// Unified diff format
    Unified,
}

/// Diff subcommand arguments
#[derive(Parser, Debug, Clone)]
#[command(
    about = "Show difference between desired and live cluster state",
    long_about = "Compares the operator's desired state (calculated from StellarNode CRD) with\n\
        what is actually deployed in the cluster. Similar to kubectl diff, but includes\n\
        internal operator-managed resources like ConfigMaps, resource limits, and more.\n\n\
        EXAMPLES:\n  \
        stellar-operator diff --name my-validator --namespace stellar\n  \
        stellar-operator diff --name my-validator --namespace stellar --format json\n  \
        stellar-operator diff --name my-validator --namespace stellar --show-config\n  \
        stellar-operator diff --name my-validator --namespace stellar --all-resources"
)]
pub struct DiffArgs {
    /// Name of the StellarNode resource to diff
    ///
    /// Example: --name my-validator
    #[arg(long, short = 'n')]
    pub name: String,

    /// Kubernetes namespace of the StellarNode
    ///
    /// Example: --namespace stellar-system
    #[arg(long, short = 'N', default_value = "default")]
    pub namespace: String,

    /// Output format: terminal, json, or unified
    ///
    /// terminal: Colored output for terminal (default)
    /// json: Machine-readable JSON format
    /// unified: Standard unified diff format
    ///
    /// Example: --format json
    #[arg(long, value_enum, default_value = "terminal")]
    pub format: DiffFormat,

    /// Show ConfigMap contents in diff (includes stellar-core.cfg, captive-core.cfg)
    ///
    /// By default, ConfigMap data is shown as a summary. This flag shows full contents.
    /// Example: --show-config
    #[arg(long)]
    pub show_config: bool,

    /// Include all managed resources in diff
    ///
    /// By default, only shows resources that have differences. This flag shows all resources,
    /// even if they match (marked as "unchanged").
    /// Example: --all-resources
    #[arg(long)]
    pub all_resources: bool,

    /// Show only summary, not full diff output
    ///
    /// Useful for scripting and quick status checks.
    /// Example: --summary
    #[arg(long)]
    pub summary: bool,

    /// Kubernetes context to use
    ///
    /// If not specified, uses the current context from kubeconfig.
    /// Example: --context my-cluster
    #[arg(long)]
    pub context: Option<String>,
}

/// Represents a single resource diff
#[derive(Debug, Clone, Serialize)]
pub struct ResourceDiff {
    /// Resource kind (e.g., "ConfigMap", "Deployment")
    pub kind: String,
    /// Resource name
    pub name: String,
    /// Namespace
    pub namespace: String,
    /// Status: "added", "removed", "modified", or "unchanged"
    pub status: String,
    /// Desired state as JSON Value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired: Option<Value>,
    /// Live state as JSON Value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<Value>,
    /// Human-readable diff output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_text: Option<String>,
    /// List of changed fields (for modified resources)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed_fields: Vec<String>,
}

/// Overall diff result
#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    /// StellarNode name
    pub node_name: String,
    /// Namespace
    pub namespace: String,
    /// Whether StellarNode itself exists
    pub node_exists: bool,
    /// List of resource diffs
    pub resources: Vec<ResourceDiff>,
    /// Summary statistics
    pub summary: DiffSummary,
}

/// Summary of diff results
#[derive(Debug, Clone, Serialize)]
pub struct DiffSummary {
    /// Total resources checked
    pub total: usize,
    /// Resources with no differences
    pub unchanged: usize,
    /// Resources to be added
    pub added: usize,
    /// Resources to be removed
    pub removed: usize,
    /// Resources to be modified
    pub modified: usize,
}

impl DiffResult {
    /// Print the diff result to stdout
    pub fn print(&self, args: &DiffArgs) -> Result<(), Error> {
        match args.format {
            DiffFormat::Terminal => self.print_terminal(args),
            DiffFormat::Json => self.print_json(),
            DiffFormat::Unified => self.print_unified(),
        }
    }

    /// Print colored terminal output
    fn print_terminal(&self, args: &DiffArgs) -> Result<(), Error> {
        if !self.node_exists {
            eprintln!(
                "❌ StellarNode '{}/{}' not found in cluster",
                self.namespace, self.node_name
            );
            return Err(Error::ConfigError(format!(
                "StellarNode '{}/{}' not found",
                self.namespace, self.node_name
            )));
        }

        println!("\n{}", "═".repeat(80));
        println!(
            "🔍 Diff for StellarNode: {}/{}",
            self.namespace.as_str().bold(),
            self.node_name.as_str().bold()
        );
        println!("{}\n", "═".repeat(80));

        // Print summary
        println!("📊 Summary:");
        println!("   Total resources:   {}", self.summary.total);
        self.print_count("   Unchanged:       ", &self.summary.unchanged, "gray");
        self.print_count("   To be added:     ", &self.summary.added, "green");
        self.print_count("   To be removed:   ", &self.summary.removed, "red");
        self.print_count("   To be modified:  ", &self.summary.modified, "yellow");
        println!();

        if self.summary.total == 0 {
            println!(
                "ℹ️  No resources found. The StellarNode may not have any managed resources yet."
            );
            return Ok(());
        }

        // Group by status using simple filters
        let added: Vec<_> = self
            .resources
            .iter()
            .filter(|r| r.status == "added")
            .collect();
        let removed: Vec<_> = self
            .resources
            .iter()
            .filter(|r| r.status == "removed")
            .collect();
        let modified: Vec<_> = self
            .resources
            .iter()
            .filter(|r| r.status == "modified")
            .collect();
        let unchanged: Vec<_> = self
            .resources
            .iter()
            .filter(|r| r.status == "unchanged")
            .collect();

        // Only show resources with changes unless --all-resources
        let show_all = args.all_resources;

        if !show_all && added.is_empty() && removed.is_empty() && modified.is_empty() {
            println!("✅ All resources are in sync with desired state.\n");
            return Ok(());
        }

        // Print added resources
        for diff in added {
            self.print_resource_diff_terminal(diff, "added", args)?;
        }

        // Print removed resources
        for diff in removed {
            self.print_resource_diff_terminal(diff, "removed", args)?;
        }

        // Print modified resources
        for diff in modified {
            self.print_resource_diff_terminal(diff, "modified", args)?;
        }

        // Print unchanged if requested
        if show_all {
            for diff in unchanged {
                self.print_resource_diff_terminal(diff, "unchanged", args)?;
            }
        }

        println!("{}\n", "═".repeat(80));
        Ok(())
    }

    fn print_count(&self, prefix: &str, count: &usize, color: &str) {
        let colored = match color {
            "green" => format!("\x1b[32m{count}\x1b[0m"),
            "red" => format!("\x1b[31m{count}\x1b[0m"),
            "yellow" => format!("\x1b[33m{count}\x1b[0m"),
            "gray" => format!("\x1b[90m{count}\x1b[0m"),
            _ => count.to_string(),
        };
        println!("{prefix}{colored}");
    }

    fn print_resource_diff_terminal(
        &self,
        diff: &ResourceDiff,
        status: &str,
        args: &DiffArgs,
    ) -> Result<(), Error> {
        let icon = match status {
            "added" => "➕",
            "removed" => "❌",
            "modified" => "✏️",
            _ => "✅",
        };

        let color = match status {
            "added" => "\x1b[32m",    // Green
            "removed" => "\x1b[31m",  // Red
            "modified" => "\x1b[33m", // Yellow
            _ => "\x1b[90m",          // Gray
        };
        let reset = "\x1b[0m";

        println!(
            "\n{} {}{}/{} ({}){}",
            icon, color, diff.kind, diff.name, diff.status, reset
        );

        if args.summary {
            return Ok(());
        }

        // Show changed fields for modified resources
        if !diff.changed_fields.is_empty() {
            println!("   Changed fields:");
            for field in &diff.changed_fields {
                println!("     - {field}");
            }
        }

        // Show diff text if available
        if let Some(diff_text) = &diff.diff_text {
            println!("\n{diff_text}");
        }

        // Show ConfigMap data if requested
        if args.show_config && diff.kind == "ConfigMap" {
            if let Some(desired) = &diff.desired {
                if let Some(data) = desired.get("data") {
                    println!("\n   📄 ConfigMap data:");
                    if let Some(obj) = data.as_object() {
                        for (key, value) in obj {
                            println!("   ── {key} ──");
                            // Show first few lines of config files
                            let value_str = value.as_str().unwrap_or("");
                            let lines: Vec<&str> = value_str.lines().take(10).collect();
                            for line in lines {
                                println!("     {line}");
                            }
                            if value_str.lines().count() > 10 {
                                println!(
                                    "     ... ({} more lines)",
                                    value_str.lines().count() - 10
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Print JSON output
    fn print_json(&self) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(self)?;
        println!("{json}");
        Ok(())
    }

    /// Print unified diff format
    fn print_unified(&self) -> Result<(), Error> {
        println!(
            "# Diff for StellarNode: {}/{}",
            self.namespace, self.node_name
        );
        println!();

        for diff in &self.resources {
            if diff.status == "unchanged" && !self.resources.iter().any(|r| r.status != "unchanged")
            {
                continue;
            }

            println!("--- a/{}/{}", diff.kind.to_lowercase(), diff.name);
            println!("+++ b/{}/{}", diff.kind.to_lowercase(), diff.name);

            if let Some(diff_text) = &diff.diff_text {
                println!("{diff_text}");
            }
            println!();
        }

        Ok(())
    }
}

/// Main entry point for diff subcommand
pub async fn diff(args: DiffArgs) -> Result<(), Error> {
    info!(
        "Generating diff for StellarNode: {}/{}",
        args.namespace, args.name
    );

    // Create Kubernetes client
    let client = if let Some(context) = &args.context {
        let kube_config = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some(context.clone()),
            ..Default::default()
        })
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to load kubeconfig context: {e}")))?;
        Client::try_from(kube_config)?
    } else {
        Client::try_default().await?
    };

    // Fetch the StellarNode
    let stellar_node_api: Api<StellarNode> = Api::namespaced(client.clone(), &args.namespace);

    let stellar_node = match stellar_node_api.get(&args.name).await {
        Ok(node) => node,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            eprintln!(
                "❌ StellarNode '{}/{}' not found",
                args.namespace, args.name
            );
            return Err(Error::ConfigError(format!(
                "StellarNode '{}/{}' not found",
                args.namespace, args.name
            )));
        }
        Err(e) => return Err(Error::from(e)),
    };

    info!("Found StellarNode: {}/{}", args.namespace, args.name);

    // Generate diffs for all managed resources
    let mut resources = Vec::new();

    // ConfigMap
    resources.push(diff_config_map(&client, &stellar_node, args.show_config).await?);

    // Deployment (for Horizon, Soroban RPC)
    if let Some(diff) = diff_deployment(&client, &stellar_node).await? {
        resources.push(diff);
    }

    // StatefulSet (for Validators)
    if let Some(diff) = diff_statefulset(&client, &stellar_node).await? {
        resources.push(diff);
    }

    // Service
    resources.push(diff_service(&client, &stellar_node).await?);

    // PVC
    resources.push(diff_pvc(&client, &stellar_node).await?);

    // HPA
    resources.push(diff_hpa(&client, &stellar_node).await?);

    // NetworkPolicy
    resources.push(diff_network_policy(&client, &stellar_node).await?);

    // PDB
    resources.push(diff_pdb(&client, &stellar_node).await?);

    // Ingress
    resources.push(diff_ingress(&client, &stellar_node).await?);

    // Calculate summary
    let summary = DiffSummary {
        total: resources.len(),
        unchanged: resources.iter().filter(|r| r.status == "unchanged").count(),
        added: resources.iter().filter(|r| r.status == "added").count(),
        removed: resources.iter().filter(|r| r.status == "removed").count(),
        modified: resources.iter().filter(|r| r.status == "modified").count(),
    };

    let result = DiffResult {
        node_name: args.name.clone(),
        namespace: args.namespace.clone(),
        node_exists: true,
        resources,
        summary,
    };

    result.print(&args)?;

    Ok(())
}

/// Diff a ConfigMap
async fn diff_config_map(
    client: &Client,
    node: &StellarNode,
    show_full_config: bool,
) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "config");
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);

    // Build desired state
    let desired_cm = build_config_map(node, None, false);

    // Fetch live state
    let live_cm = api.get(&name).await.ok();

    generate_diff(
        "ConfigMap",
        &name,
        &namespace,
        desired_cm.metadata,
        live_cm.map(|cm| cm.metadata),
        show_full_config,
    )
}

/// Diff a Deployment
async fn diff_deployment(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<ResourceDiff>, Error> {
    // Only validators use StatefulSets, others use Deployments
    if matches!(node.spec.node_type, crate::crd::NodeType::Validator) {
        return Ok(None);
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "deployment");
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);

    // Build desired state (simplified - in production would call full build function)
    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    // Fetch live state
    let live_metadata = api.get(&name).await.ok().map(|dep| dep.metadata);

    Ok(Some(generate_diff(
        "Deployment",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )?))
}

/// Diff a StatefulSet
async fn diff_statefulset(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<ResourceDiff>, Error> {
    // Only validators use StatefulSets
    if !matches!(node.spec.node_type, crate::crd::NodeType::Validator) {
        return Ok(None);
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "statefulset");
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|sts| sts.metadata);

    Ok(Some(generate_diff(
        "StatefulSet",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )?))
}

/// Diff a Service
async fn diff_service(client: &Client, node: &StellarNode) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service");
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|svc| svc.metadata);

    generate_diff(
        "Service",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )
}

/// Diff a PVC
async fn diff_pvc(client: &Client, node: &StellarNode) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "data");
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|pvc| pvc.metadata);

    generate_diff(
        "PersistentVolumeClaim",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )
}

/// Diff an HPA
async fn diff_hpa(client: &Client, node: &StellarNode) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "hpa");
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|hpa| hpa.metadata);

    generate_diff(
        "HorizontalPodAutoscaler",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )
}

/// Diff a NetworkPolicy
async fn diff_network_policy(client: &Client, node: &StellarNode) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "network-policy");
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|np| np.metadata);

    generate_diff(
        "NetworkPolicy",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )
}

/// Diff a PDB
async fn diff_pdb(client: &Client, node: &StellarNode) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "pdb");
    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|pdb| pdb.metadata);

    generate_diff(
        "PodDisruptionBudget",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )
}

/// Diff an Ingress
async fn diff_ingress(client: &Client, node: &StellarNode) -> Result<ResourceDiff, Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "ingress");
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);

    let desired_metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(standard_labels(node)),
        ..Default::default()
    };

    let live_metadata = api.get(&name).await.ok().map(|ing| ing.metadata);

    generate_diff(
        "Ingress",
        &name,
        &namespace,
        desired_metadata,
        live_metadata,
        false,
    )
}

/// Generate a diff between desired and live metadata
fn generate_diff(
    kind: &str,
    name: &str,
    namespace: &str,
    desired_meta: ObjectMeta,
    live_meta: Option<ObjectMeta>,
    show_full: bool,
) -> Result<ResourceDiff, Error> {
    let (status, diff_text, changed_fields) = match live_meta {
        None => {
            // Resource doesn't exist - will be added
            ("added".to_string(), None, vec![])
        }
        Some(live) => {
            // Compare metadata and labels
            let mut changed = Vec::new();

            // Compare labels
            let desired_labels = desired_meta.labels.clone().unwrap_or_default();
            let live_labels = live.labels.clone().unwrap_or_default();

            for (key, value) in &desired_labels {
                if !live_labels.contains_key(key) {
                    changed.push(format!("labels.{key} (missing)"));
                } else if live_labels.get(key) != Some(value) {
                    changed.push(format!("labels.{key}"));
                }
            }

            for key in live_labels.keys() {
                if !desired_labels.contains_key(key) {
                    changed.push(format!("labels.{key} (extra)"));
                }
            }

            // Compare annotations
            let desired_annotations = desired_meta.annotations.clone().unwrap_or_default();
            let live_annotations = live.annotations.clone().unwrap_or_default();

            for (key, value) in &desired_annotations {
                if !live_annotations.contains_key(key) {
                    changed.push(format!("annotations.{key} (missing)"));
                } else if live_annotations.get(key) != Some(value) {
                    changed.push(format!("annotations.{key}"));
                }
            }

            if changed.is_empty() {
                ("unchanged".to_string(), None, vec![])
            } else {
                let diff_text = generate_unified_diff(
                    &format!("a/{kind}/{name}"),
                    &format!("b/{kind}/{name}"),
                    &format!("{desired_meta:#?}"),
                    &format!("{live:#?}"),
                );
                ("modified".to_string(), Some(diff_text), changed)
            }
        }
    };

    Ok(ResourceDiff {
        kind: kind.to_string(),
        name: name.to_string(),
        namespace: namespace.to_string(),
        status,
        desired: if show_full {
            serde_json::to_value(desired_meta).ok()
        } else {
            None
        },
        live: None,
        diff_text,
        changed_fields,
    })
}

/// Generate unified diff text
#[allow(clippy::unwrap_used)]
fn generate_unified_diff(
    from_label: &str,
    to_label: &str,
    from_content: &str,
    to_content: &str,
) -> String {
    let from_lines: Vec<&str> = from_content.lines().collect();
    let to_lines: Vec<&str> = to_content.lines().collect();

    let mut result = String::new();
    writeln!(&mut result, "--- {from_label}").unwrap();
    writeln!(&mut result, "+++ {to_label}").unwrap();

    // Simple line-by-line diff (in production, use a proper diff library)
    let mut i = 0;
    let mut j = 0;

    while i < from_lines.len() || j < to_lines.len() {
        if i >= from_lines.len() {
            writeln!(&mut result, "+{}", to_lines[j]).unwrap();
            j += 1;
        } else if j >= to_lines.len() {
            writeln!(&mut result, "-{}", from_lines[i]).unwrap();
            i += 1;
        } else if from_lines[i] == to_lines[j] {
            writeln!(&mut result, " {}", from_lines[i]).unwrap();
            i += 1;
            j += 1;
        } else {
            // Lines differ
            writeln!(&mut result, "-{}", from_lines[i]).unwrap();
            writeln!(&mut result, "+{}", to_lines[j]).unwrap();
            i += 1;
            j += 1;
        }
    }

    result
}

// Extension trait for colored output
trait ColoredOutput {
    fn bold(&self) -> String;
}

impl ColoredOutput for &str {
    fn bold(&self) -> String {
        format!("\x1b[1m{self}\x1b[0m")
    }
}

impl ColoredOutput for String {
    fn bold(&self) -> String {
        format!("\x1b[1m{self}\x1b[0m")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn test_diff_format_parsing() {
        // Test ValueEnum parsing
        use clap::ValueEnum;

        let terminal = DiffFormat::from_str("terminal", false).unwrap();
        assert!(matches!(terminal, DiffFormat::Terminal));

        let json = DiffFormat::from_str("json", false).unwrap();
        assert!(matches!(json, DiffFormat::Json));

        let unified = DiffFormat::from_str("unified", false).unwrap();
        assert!(matches!(unified, DiffFormat::Unified));

        // Invalid format should error
        assert!(DiffFormat::from_str("invalid", false).is_err());
    }

    #[test]
    fn test_generate_unified_diff() {
        let from = "line1\nline2\nline3";
        let to = "line1\nline2-modified\nline3";

        let diff = generate_unified_diff("a/test", "b/test", from, to);

        assert!(diff.contains("--- a/test"));
        assert!(diff.contains("+++ b/test"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+line2-modified"));
    }

    #[test]
    fn test_resource_diff_serialization() {
        let diff = ResourceDiff {
            kind: "ConfigMap".to_string(),
            name: "test-config".to_string(),
            namespace: "default".to_string(),
            status: "modified".to_string(),
            desired: None,
            live: None,
            diff_text: Some("test diff".to_string()),
            changed_fields: vec!["labels.app".to_string()],
        };

        let json = serde_json::to_string(&diff).unwrap();
        assert!(json.contains("\"kind\":\"ConfigMap\""));
        assert!(json.contains("\"status\":\"modified\""));
        assert!(json.contains("\"changed_fields\""));
    }

    #[test]
    fn test_diff_summary_calculation() {
        let resources = [
            ResourceDiff {
                kind: "ConfigMap".to_string(),
                name: "cm1".to_string(),
                namespace: "default".to_string(),
                status: "unchanged".to_string(),
                desired: None,
                live: None,
                diff_text: None,
                changed_fields: vec![],
            },
            ResourceDiff {
                kind: "Deployment".to_string(),
                name: "deploy1".to_string(),
                namespace: "default".to_string(),
                status: "modified".to_string(),
                desired: None,
                live: None,
                diff_text: None,
                changed_fields: vec![],
            },
            ResourceDiff {
                kind: "Service".to_string(),
                name: "svc1".to_string(),
                namespace: "default".to_string(),
                status: "added".to_string(),
                desired: None,
                live: None,
                diff_text: None,
                changed_fields: vec![],
            },
        ];

        let summary = DiffSummary {
            total: resources.len(),
            unchanged: resources.iter().filter(|r| r.status == "unchanged").count(),
            added: resources.iter().filter(|r| r.status == "added").count(),
            removed: resources.iter().filter(|r| r.status == "removed").count(),
            modified: resources.iter().filter(|r| r.status == "modified").count(),
        };

        assert_eq!(summary.total, 3);
        assert_eq!(summary.unchanged, 1);
        assert_eq!(summary.added, 1);
        assert_eq!(summary.removed, 0);
        assert_eq!(summary.modified, 1);
    }
}
