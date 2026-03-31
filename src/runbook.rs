//! Runbook generation for StellarNode troubleshooting
//!
//! This module generates context-aware troubleshooting runbooks tailored to
//! specific StellarNode configurations, providing L1 support teams with
//! instant, actionable troubleshooting steps.
//!
//! # Features
//!
//! - **Configuration-Aware**: Generates commands specific to the node's setup
//! - **KMS Integration**: Includes KMS key status checks if configured
//! - **Archive Support**: S3/GCS CLI commands for archive verification
//! - **Network Info**: Lists expected peers based on quorum set
//! - **Markdown Output**: Human-readable format suitable for documentation
//!
//! # Example
//!
//! ```rust,ignore
//! use stellar_k8s::runbook::generate_runbook;
//! use stellar_k8s::crd::StellarNode;
//! use kube::ResourceExt;
//!
//! let node: StellarNode = /* ... */;
//! let runbook = generate_runbook(&node)?;
//! println!("{}", runbook);
//! ```

use crate::crd::StellarNode;
use crate::error::Result;
use chrono::Utc;
use kube::ResourceExt;

/// Generates a comprehensive troubleshooting runbook for a StellarNode
///
/// # Arguments
///
/// * `node` - The StellarNode resource to generate a runbook for
///
/// # Returns
///
/// A formatted Markdown string containing troubleshooting steps
///
/// # Example
///
/// ```rust,ignore
/// let runbook = generate_runbook(&my_node)?;
/// std::fs::write("runbook.md", runbook)?;
/// ```
pub fn generate_runbook(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let spec = &node.spec;

    let mut runbook = String::new();

    // Header
    runbook.push_str(&format!(
        "# Troubleshooting Runbook: {}/{}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "**Generated**: {}\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));
    runbook.push_str(&format!("**Node Type**: {}\n", spec.node_type));
    runbook.push_str(&format!("**Network**: {:?}\n\n", spec.network));

    // Quick Status Section
    runbook.push_str("## Quick Status Check\n\n");
    runbook.push_str(&generate_status_commands(&name, &namespace));

    // Node-specific sections
    match spec.node_type.to_string().as_str() {
        "Validator" => {
            runbook.push_str(&generate_validator_runbook(node)?);
        }
        "Horizon" => {
            runbook.push_str(&generate_horizon_runbook(node)?);
        }
        "SorobanRpc" => {
            runbook.push_str(&generate_soroban_runbook(node)?);
        }
        _ => {}
    }

    // Common troubleshooting
    runbook.push_str(&generate_common_troubleshooting(&name, &namespace));

    // Archive section if applicable
    if spec.validator_config.as_ref().map(|v| v.enable_history_archive).unwrap_or(false) {
        runbook.push_str(&generate_archive_troubleshooting(node)?);
    }

    // KMS section if applicable
    if spec
        .validator_config
        .as_ref()
        .and_then(|v| v.kms_config.as_ref())
        .is_some()
    {
        runbook.push_str(&generate_kms_troubleshooting(node)?);
    }

    // Network section
    runbook.push_str(&generate_network_troubleshooting(node)?);

    // Resource section
    runbook.push_str(&generate_resource_troubleshooting(&name, &namespace));

    Ok(runbook)
}

/// Generate quick status check commands
fn generate_status_commands(name: &str, namespace: &str) -> String {
    let mut commands = String::new();

    commands.push_str("```bash\n");
    commands.push_str(&format!(
        "# Check node status\nkubectl get stellarnode {}/{}\n\n",
        namespace, name
    ));
    commands.push_str(&format!(
        "# Check pod status\nkubectl get pods -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    commands.push_str(&format!(
        "# Check node conditions\nkubectl describe stellarnode {}/{}\n\n",
        namespace, name
    ));
    commands.push_str(&format!(
        "# View recent events\nkubectl get events -n {} --sort-by='.lastTimestamp' | grep {}\n",
        namespace, name
    ));
    commands.push_str("```\n\n");

    commands
}

/// Generate validator-specific troubleshooting steps
fn generate_validator_runbook(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let spec = &node.spec;

    let mut runbook = String::new();

    runbook.push_str("## Validator-Specific Checks\n\n");

    // Core container logs
    runbook.push_str("### 1. Check Stellar Core Logs\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Stream core logs\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c core -f\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Get last 100 lines\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c core --tail=100\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Get logs from last hour\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c core --since=1h\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    // Database checks
    runbook.push_str("### 2. Check Database Status\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check database pod\nkubectl get pods -n {} -l app.kubernetes.io/name=stellar-db,app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check database logs\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-db,app.kubernetes.io/instance={} --tail=50\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check PVC status\nkubectl get pvc -n {} -l app.kubernetes.io/instance={}\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    // Quorum set info
    if let Some(validator_config) = &spec.validator_config {
        if let Some(quorum_set) = &validator_config.quorum_set {
            runbook.push_str("### 3. Quorum Set Configuration\n\n");
            runbook.push_str("```toml\n");
            runbook.push_str(quorum_set);
            runbook.push_str("\n```\n\n");
        }
    }

    // Sync status
    runbook.push_str("### 4. Check Sync Status\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check if node is synced\nkubectl exec -n {} -it $(kubectl get pods -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -o jsonpath='{{.items[0].metadata.name}}') -c core -- stellar-core info\n",
        namespace, namespace, name
    ));
    runbook.push_str("```\n\n");

    Ok(runbook)
}

/// Generate Horizon-specific troubleshooting steps
fn generate_horizon_runbook(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    let mut runbook = String::new();

    runbook.push_str("## Horizon-Specific Checks\n\n");

    // Horizon logs
    runbook.push_str("### 1. Check Horizon Logs\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Stream Horizon logs\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c horizon -f\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Get last 100 lines\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c horizon --tail=100\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    // Health check
    runbook.push_str("### 2. Check Horizon Health\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Port-forward to Horizon\nkubectl port-forward -n {} svc/{} 8000:8000 &\n\n",
        namespace, name
    ));
    runbook.push_str("# Check health endpoint\ncurl http://localhost:8000/health\n\n");
    runbook.push_str("# Check sync status\ncurl http://localhost:8000/\n");
    runbook.push_str("```\n\n");

    // Database sync
    runbook.push_str("### 3. Check Database Sync\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check if Horizon database is synced\nkubectl exec -n {} -it $(kubectl get pods -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -o jsonpath='{{.items[0].metadata.name}}') -c horizon -- horizon db status\n",
        namespace, namespace, name
    ));
    runbook.push_str("```\n\n");

    Ok(runbook)
}

/// Generate Soroban RPC-specific troubleshooting steps
fn generate_soroban_runbook(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    let mut runbook = String::new();

    runbook.push_str("## Soroban RPC-Specific Checks\n\n");

    // RPC logs
    runbook.push_str("### 1. Check Soroban RPC Logs\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Stream RPC logs\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c soroban-rpc -f\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Get last 100 lines\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c soroban-rpc --tail=100\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    // RPC health
    runbook.push_str("### 2. Check RPC Health\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Port-forward to RPC\nkubectl port-forward -n {} svc/{} 8000:8000 &\n\n",
        namespace, name
    ));
    runbook.push_str("# Check RPC health\ncurl -X POST http://localhost:8000 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getHealth\"}'\n\n");
    runbook.push_str("# Check ledger info\ncurl -X POST http://localhost:8000 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getLedgerEntries\",\"params\":{\"keys\":[]}}'\n");
    runbook.push_str("```\n\n");

    Ok(runbook)
}

/// Generate common troubleshooting steps
fn generate_common_troubleshooting(name: &str, namespace: &str) -> String {
    let mut runbook = String::new();

    runbook.push_str("## Common Troubleshooting Steps\n\n");

    runbook.push_str("### 1. Check Pod Status\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Get detailed pod information\nkubectl describe pods -n {} -l app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check for pod restart loops\nkubectl get pods -n {} -l app.kubernetes.io/instance={} -o jsonpath='{{range .items[*]}}{{.metadata.name}}{{\"\\t\"}}{{.status.containerStatuses[0].restartCount}}{{\"\\n\"}}{{end}}'\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    runbook.push_str("### 2. Check Resource Usage\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check CPU and memory usage\nkubectl top pods -n {} -l app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check node resource availability\nkubectl top nodes\n"
    ));
    runbook.push_str("```\n\n");

    runbook.push_str("### 3. Check Storage\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check PVC status\nkubectl get pvc -n {} -l app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check PVC usage\nkubectl exec -n {} -it $(kubectl get pods -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -o jsonpath='{{.items[0].metadata.name}}') -- df -h /data\n",
        namespace, namespace, name
    ));
    runbook.push_str("```\n\n");

    runbook.push_str("### 4. Check Network Connectivity\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Test DNS resolution\nkubectl exec -n {} -it $(kubectl get pods -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -o jsonpath='{{.items[0].metadata.name}}') -- nslookup kubernetes.default\n\n",
        namespace, namespace, name
    ));
    runbook.push_str(&format!(
        "# Check service endpoints\nkubectl get endpoints -n {} {}\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    runbook
}

/// Generate archive-specific troubleshooting steps
fn generate_archive_troubleshooting(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let spec = &node.spec;

    let mut runbook = String::new();

    runbook.push_str("## History Archive Troubleshooting\n\n");

    if let Some(validator_config) = &spec.validator_config {
        if !validator_config.history_archive_urls.is_empty() {
            runbook.push_str("### Archive URLs\n\n");
            for url in &validator_config.history_archive_urls {
                runbook.push_str(&format!("- `{}`\n", url));
            }
            runbook.push_str("\n");
        }
    }

    runbook.push_str("### 1. Check Archive Health\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check archive connectivity\nkubectl exec -n {} -it $(kubectl get pods -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -o jsonpath='{{.items[0].metadata.name}}') -c core -- curl -I https://history.stellar.org/pyx/history-00.json.gz\n\n",
        namespace, namespace, name
    ));
    runbook.push_str(&format!(
        "# Check archive lag\nkubectl logs -n {} -l app.kubernetes.io/name=stellar-node,app.kubernetes.io/instance={} -c core | grep 'archive lag'\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    // S3/GCS specific commands
    runbook.push_str("### 2. Verify Archive Bucket Access\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str("# For S3 archives:\naws s3 ls s3://your-archive-bucket/ --recursive | head -20\n\n");
    runbook.push_str("# For GCS archives:\ngsutil ls -r gs://your-archive-bucket/ | head -20\n");
    runbook.push_str("```\n\n");

    Ok(runbook)
}

/// Generate KMS-specific troubleshooting steps
fn generate_kms_troubleshooting(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let spec = &node.spec;

    let mut runbook = String::new();

    runbook.push_str("## KMS Troubleshooting\n\n");

    if let Some(validator_config) = &spec.validator_config {
        if let Some(kms_config) = &validator_config.kms_config {
            runbook.push_str("### KMS Configuration\n\n");
            runbook.push_str(&format!("- **Provider**: {}\n", kms_config.provider));
            runbook.push_str(&format!("- **Key ID**: {}\n", kms_config.key_id));
            if let Some(region) = &kms_config.region {
                runbook.push_str(&format!("- **Region**: {}\n", region));
            }
            runbook.push_str("\n");
        }
    }

    runbook.push_str("### 1. Check KMS Key Status\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str("# For AWS KMS:\naws kms describe-key --key-id <KEY_ID> --region <REGION>\n\n");
    runbook.push_str("# Check key rotation status:\naws kms get-key-rotation-status --key-id <KEY_ID> --region <REGION>\n\n");
    runbook.push_str("# For GCP KMS:\ngcloud kms keys describe <KEY_NAME> --location <LOCATION> --keyring <KEYRING>\n");
    runbook.push_str("```\n\n");

    runbook.push_str("### 2. Check KMS Permissions\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str("# For AWS KMS:\naws kms get-public-key --key-id <KEY_ID> --region <REGION>\n\n");
    runbook.push_str("# For GCP KMS:\ngcloud kms keys get-iam-policy <KEY_NAME> --location <LOCATION> --keyring <KEYRING>\n");
    runbook.push_str("```\n\n");

    runbook.push_str("### 3. Check Pod IAM/Service Account\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check service account\nkubectl get sa -n {} -l app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check service account annotations (for IRSA/Workload Identity)\nkubectl describe sa -n {} $(kubectl get sa -n {} -l app.kubernetes.io/instance={} -o jsonpath='{{.items[0].metadata.name}}')\n",
        namespace, namespace, name
    ));
    runbook.push_str("```\n\n");

    Ok(runbook)
}

/// Generate network troubleshooting steps
fn generate_network_troubleshooting(node: &StellarNode) -> Result<String> {
    let name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let spec = &node.spec;

    let mut runbook = String::new();

    runbook.push_str("## Network Troubleshooting\n\n");

    runbook.push_str("### Network Information\n\n");
    runbook.push_str(&format!("- **Network**: {:?}\n", spec.network));
    runbook.push_str(&format!("- **Network Passphrase**: {}\n\n", spec.network.passphrase()));

    runbook.push_str("### 1. Check Service\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Get service details\nkubectl get svc -n {} {}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check service endpoints\nkubectl get endpoints -n {} {}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Test service connectivity\nkubectl run -n {} -it --rm debug --image=busybox --restart=Never -- wget -O- http://{}:8000/health\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    runbook.push_str("### 2. Check Ingress (if configured)\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Get ingress status\nkubectl get ingress -n {} -l app.kubernetes.io/instance={}\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check ingress details\nkubectl describe ingress -n {} -l app.kubernetes.io/instance={}\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    // Expected peers from quorum set
    if let Some(validator_config) = &spec.validator_config {
        if let Some(quorum_set) = &validator_config.quorum_set {
            runbook.push_str("### 3. Expected Peer Connections\n\n");
            runbook.push_str("Based on the quorum set configuration, this node should connect to:\n\n");
            runbook.push_str("```toml\n");
            runbook.push_str(quorum_set);
            runbook.push_str("\n```\n\n");
            runbook.push_str("Verify these peers are reachable and responding.\n\n");
        }
    }

    Ok(runbook)
}

/// Generate resource troubleshooting steps
fn generate_resource_troubleshooting(name: &str, namespace: &str) -> String {
    let mut runbook = String::new();

    runbook.push_str("## Resource Troubleshooting\n\n");

    runbook.push_str("### 1. Check Resource Requests and Limits\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# View resource configuration\nkubectl get stellarnode {}/{} -o jsonpath='{{.spec.resources}}' | jq\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check actual resource usage\nkubectl top pods -n {} -l app.kubernetes.io/instance={}\n",
        namespace, name
    ));
    runbook.push_str("```\n\n");

    runbook.push_str("### 2. Check Node Affinity\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check pod node assignment\nkubectl get pods -n {} -l app.kubernetes.io/instance={} -o wide\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check node labels\nkubectl get nodes --show-labels\n"
    ));
    runbook.push_str("```\n\n");

    runbook.push_str("### 3. Check for Resource Constraints\n\n");
    runbook.push_str("```bash\n");
    runbook.push_str(&format!(
        "# Check for pending pods\nkubectl get pods -n {} -l app.kubernetes.io/instance={} --field-selector=status.phase=Pending\n\n",
        namespace, name
    ));
    runbook.push_str(&format!(
        "# Check node capacity\nkubectl describe nodes | grep -A 5 'Allocated resources'\n"
    ));
    runbook.push_str("```\n\n");

    runbook
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_commands_generation() {
        let commands = generate_status_commands("test-node", "default");
        assert!(commands.contains("kubectl get stellarnode"));
        assert!(commands.contains("kubectl get pods"));
        assert!(commands.contains("kubectl describe stellarnode"));
        assert!(commands.contains("kubectl get events"));
    }

    #[test]
    fn test_common_troubleshooting_generation() {
        let runbook = generate_common_troubleshooting("test-node", "default");
        assert!(runbook.contains("Check Pod Status"));
        assert!(runbook.contains("Check Resource Usage"));
        assert!(runbook.contains("Check Storage"));
        assert!(runbook.contains("Check Network Connectivity"));
    }

    #[test]
    fn test_resource_troubleshooting_generation() {
        let runbook = generate_resource_troubleshooting("test-node", "default");
        assert!(runbook.contains("Check Resource Requests and Limits"));
        assert!(runbook.contains("Check Node Affinity"));
        assert!(runbook.contains("Check for Resource Constraints"));
    }
}
