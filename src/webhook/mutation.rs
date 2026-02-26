//! Mutating Admission Webhook Logic
//!
//! This module implements mutation logic for StellarNode resources,
//! applying sensible defaults to reduce boilerplate in manifests.

use kube::core::admission::AdmissionRequest;
use serde_json::json;
use std::collections::BTreeMap;
use tracing::{debug, info};

use crate::crd::{NodeType, StellarNetwork, StellarNode, StellarNodeSpec};
use crate::error::Result;

/// Latest stable versions for different node types
const STELLAR_CORE_VERSION: &str = "v21.3.0";
const HORIZON_VERSION: &str = "v2.31.0";
const SOROBAN_RPC_VERSION: &str = "v21.3.0";

/// Apply mutations to a StellarNode admission request
///
/// Returns Some(patch) if mutations were applied, None if no changes needed
pub fn apply_mutations(req: &AdmissionRequest<StellarNode>) -> Result<Option<serde_json::Value>> {
    let Some(object) = &req.object else {
        return Ok(None);
    };

    let mut patches = Vec::new();
    let spec = &object.spec;

    // 1. Default version if missing
    if spec.version.is_empty() {
        let default_version = get_default_version(&spec.node_type);
        patches.push(json!({
            "op": "add",
            "path": "/spec/version",
            "value": default_version
        }));
        info!(
            "Defaulting version to {} for {:?}",
            default_version, spec.node_type
        );
    }

    // 2. Default resources if missing (check if they're using defaults)
    let using_default_resources = spec.resources.requests.cpu == "500m"
        && spec.resources.requests.memory == "1Gi"
        && spec.resources.limits.cpu == "2"
        && spec.resources.limits.memory == "4Gi";

    if using_default_resources {
        let (cpu_request, memory_request, cpu_limit, memory_limit) =
            get_default_resource_values(&spec.node_type, &spec.network);

        // Add requests
        patches.push(json!({
            "op": "replace",
            "path": "/spec/resources/requests/cpu",
            "value": cpu_request
        }));
        patches.push(json!({
            "op": "replace",
            "path": "/spec/resources/requests/memory",
            "value": memory_request
        }));

        // Add limits
        patches.push(json!({
            "op": "replace",
            "path": "/spec/resources/limits/cpu",
            "value": cpu_limit
        }));
        patches.push(json!({
            "op": "replace",
            "path": "/spec/resources/limits/memory",
            "value": memory_limit
        }));

        info!(
            "Defaulting resources for {:?} on {:?}",
            spec.node_type, spec.network
        );
    }

    // 3. Add standard labels
    let labels = get_standard_labels(spec, &object.metadata.name.clone().unwrap_or_default());
    if !labels.is_empty() {
        // Check if labels exist
        let has_labels = object.metadata.labels.is_some();

        for (key, value) in labels {
            let path = if has_labels || patches.iter().any(|p| p["path"] == "/metadata/labels") {
                format!("/metadata/labels/{}", key.replace('/', "~1"))
            } else {
                patches.push(json!({
                    "op": "add",
                    "path": "/metadata/labels",
                    "value": {}
                }));
                format!("/metadata/labels/{}", key.replace('/', "~1"))
            };

            patches.push(json!({
                "op": "add",
                "path": path,
                "value": value
            }));
        }

        debug!("Added standard labels");
    }

    // 4. Add standard annotations
    let annotations = get_standard_annotations(spec);
    if !annotations.is_empty() {
        let has_annotations = object.metadata.annotations.is_some();

        for (key, value) in annotations {
            let path = if has_annotations
                || patches.iter().any(|p| p["path"] == "/metadata/annotations")
            {
                format!("/metadata/annotations/{}", key.replace('/', "~1"))
            } else {
                patches.push(json!({
                    "op": "add",
                    "path": "/metadata/annotations",
                    "value": {}
                }));
                format!("/metadata/annotations/{}", key.replace('/', "~1"))
            };

            patches.push(json!({
                "op": "add",
                "path": path,
                "value": value
            }));
        }

        debug!("Added standard annotations");
    }

    if patches.is_empty() {
        Ok(None)
    } else {
        Ok(Some(json!(patches)))
    }
}

/// Get default version based on node type
fn get_default_version(node_type: &NodeType) -> &'static str {
    match node_type {
        NodeType::Validator => STELLAR_CORE_VERSION,
        NodeType::Horizon => HORIZON_VERSION,
        NodeType::SorobanRpc => SOROBAN_RPC_VERSION,
    }
}

/// Get default resource values based on node type and network
fn get_default_resource_values(
    node_type: &NodeType,
    network: &StellarNetwork,
) -> (&'static str, &'static str, &'static str, &'static str) {
    match (node_type, network) {
        // Validators need more resources
        (NodeType::Validator, StellarNetwork::Mainnet) => ("2000m", "4Gi", "4000m", "8Gi"),
        (NodeType::Validator, _) => ("1000m", "2Gi", "2000m", "4Gi"),

        // Horizon nodes
        (NodeType::Horizon, StellarNetwork::Mainnet) => ("1000m", "2Gi", "2000m", "4Gi"),
        (NodeType::Horizon, _) => ("500m", "1Gi", "1000m", "2Gi"),

        // Soroban RPC nodes
        (NodeType::SorobanRpc, StellarNetwork::Mainnet) => ("1000m", "2Gi", "2000m", "4Gi"),
        (NodeType::SorobanRpc, _) => ("500m", "1Gi", "1000m", "2Gi"),
    }
}

/// Get standard Kubernetes labels
fn get_standard_labels(spec: &StellarNodeSpec, name: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();

    // Standard app.kubernetes.io labels
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "stellar-node".to_string(),
    );
    labels.insert("app.kubernetes.io/instance".to_string(), name.to_string());
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        format!("{:?}", spec.node_type).to_lowercase(),
    );
    labels.insert(
        "app.kubernetes.io/part-of".to_string(),
        "stellar-k8s".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "stellar-operator".to_string(),
    );

    // Stellar-specific labels
    labels.insert(
        "stellar.org/network".to_string(),
        format!("{:?}", spec.network).to_lowercase(),
    );
    labels.insert(
        "stellar.org/node-type".to_string(),
        format!("{:?}", spec.node_type).to_lowercase(),
    );

    labels
}

/// Get standard annotations
fn get_standard_annotations(spec: &StellarNodeSpec) -> BTreeMap<String, String> {
    let mut annotations = BTreeMap::new();

    // Add version annotation
    if !spec.version.is_empty() {
        annotations.insert("stellar.org/version".to_string(), spec.version.clone());
    }

    // Add network annotation
    annotations.insert(
        "stellar.org/network".to_string(),
        format!("{:?}", spec.network),
    );

    // Add mutation timestamp
    annotations.insert(
        "stellar.org/mutated-at".to_string(),
        chrono::Utc::now().to_rfc3339(),
    );

    annotations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_version() {
        assert_eq!(
            get_default_version(&NodeType::Validator),
            STELLAR_CORE_VERSION
        );
        assert_eq!(get_default_version(&NodeType::Horizon), HORIZON_VERSION);
        assert_eq!(
            get_default_version(&NodeType::SorobanRpc),
            SOROBAN_RPC_VERSION
        );
    }

    #[test]
    fn test_default_resources_validator_mainnet() {
        let (cpu_req, mem_req, cpu_lim, mem_lim) =
            get_default_resource_values(&NodeType::Validator, &StellarNetwork::Mainnet);
        assert_eq!(cpu_req, "2000m");
        assert_eq!(mem_req, "4Gi");
        assert_eq!(cpu_lim, "4000m");
        assert_eq!(mem_lim, "8Gi");
    }

    #[test]
    fn test_default_resources_horizon_testnet() {
        let (cpu_req, mem_req, _, _) =
            get_default_resource_values(&NodeType::Horizon, &StellarNetwork::Testnet);
        assert_eq!(cpu_req, "500m");
        assert_eq!(mem_req, "1Gi");
    }

    #[test]
    fn test_standard_labels() {
        use crate::crd::{HistoryMode, ResourceRequirements, StorageConfig};

        let spec = StellarNodeSpec {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            history_mode: HistoryMode::default(),
            resources: ResourceRequirements::default(),
            storage: StorageConfig::default(),
            validator_config: None,
            horizon_config: None,
            soroban_config: None,
            replicas: 1,
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
            cve_handling: None,
            snapshot_schedule: None,
            restore_from_snapshot: None,
            read_replica_config: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            resource_meta: None,
            vpa_config: None,
            read_pool_endpoint: None,
        };

        let labels = get_standard_labels(&spec, "my-validator");

        assert_eq!(
            labels.get("app.kubernetes.io/name"),
            Some(&"stellar-node".to_string())
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance"),
            Some(&"my-validator".to_string())
        );
        assert_eq!(
            labels.get("stellar.org/network"),
            Some(&"testnet".to_string())
        );
    }

    #[test]
    fn test_standard_annotations() {
        use crate::crd::{HistoryMode, ResourceRequirements, StorageConfig};

        let spec = StellarNodeSpec {
            node_type: NodeType::Horizon,
            network: StellarNetwork::Mainnet,
            version: "v2.31.0".to_string(),
            history_mode: HistoryMode::default(),
            resources: ResourceRequirements::default(),
            storage: StorageConfig::default(),
            validator_config: None,
            horizon_config: None,
            soroban_config: None,
            replicas: 1,
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
            cve_handling: None,
            snapshot_schedule: None,
            restore_from_snapshot: None,
            read_replica_config: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            resource_meta: None,
            vpa_config: None,
            read_pool_endpoint: None,
        };

        let annotations = get_standard_annotations(&spec);

        assert_eq!(
            annotations.get("stellar.org/version"),
            Some(&"v2.31.0".to_string())
        );
        assert_eq!(
            annotations.get("stellar.org/network"),
            Some(&"Mainnet".to_string())
        );
        assert!(annotations.contains_key("stellar.org/mutated-at"));
    }
}
