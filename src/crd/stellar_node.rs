//! StellarNode Custom Resource Definition
//!
//! The StellarNode CRD represents a managed Stellar infrastructure node.
//! Supports Validator (Core), Horizon API, and Soroban RPC node types.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{
    AutoscalingConfig, Condition, ExternalDatabaseConfig, HorizonConfig, IngressConfig, NodeType,
    ResourceRequirements, RetentionPolicy, SorobanConfig, StellarNetwork, StorageConfig,
    ValidatorConfig,
};

/// The StellarNode CRD represents a managed Stellar infrastructure node.
///
/// # Example
///
/// ```yaml
/// apiVersion: stellar.org/v1alpha1
/// kind: StellarNode
/// metadata:
///   name: my-validator
///   namespace: stellar-nodes
/// spec:
///   nodeType: Validator
///   network: Testnet
///   version: "v21.0.0"
///   replicas: 1
///   resources:
///     requests:
///       cpu: "2"
///       memory: "8Gi"
///     limits:
///       cpu: "4"
///       memory: "16Gi"
///   storage:
///     storageClass: "ssd"
///     size: "500Gi"
///     retentionPolicy: Retain
///   validatorConfig:
///     seedSecretRef: "validator-seed"
///     enableHistoryArchive: true
/// ```
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarNode",
    namespaced,
    status = "StellarNodeStatus",
    shortname = "sn",
    printcolumn = r#"{"name":"Type","type":"string","jsonPath":".spec.nodeType"}"#,
    printcolumn = r#"{"name":"Network","type":"string","jsonPath":".spec.network"}"#,
    printcolumn = r#"{"name":"Replicas","type":"integer","jsonPath":".spec.replicas"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeSpec {
    /// Type of Stellar node to deploy (Validator, Horizon, or SorobanRpc)
    pub node_type: NodeType,

    /// Target Stellar network (Mainnet, Testnet, Futurenet, or Custom)
    pub network: StellarNetwork,

    /// Container image version to use (e.g., "v21.0.0")
    pub version: String,

    /// Compute resource requirements (CPU and memory)
    #[serde(default)]
    pub resources: ResourceRequirements,

    /// Storage configuration for persistent data
    #[serde(default)]
    pub storage: StorageConfig,

    /// Validator-specific configuration
    /// Required when nodeType is Validator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator_config: Option<ValidatorConfig>,

    /// Horizon API server configuration
    /// Required when nodeType is Horizon
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_config: Option<HorizonConfig>,

    /// Soroban RPC configuration
    /// Required when nodeType is SorobanRpc
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soroban_config: Option<SorobanConfig>,

    /// Number of replicas (only valid for Horizon and SorobanRpc nodes)
    /// Validators must always have exactly 1 replica
    #[serde(default = "default_replicas")]
    pub replicas: i32,

    /// Suspend the node (scale to 0 without deleting resources)
    /// The operator still manages the resources, but keeps them inactive.
    #[serde(default)]
    pub suspended: bool,

    /// Enable alerting via PrometheusRule or ConfigMap
    #[serde(default)]
    pub alerting: bool,

    /// External database configuration for managed Postgres databases
    /// When provided, database credentials will be fetched from the specified Secret
    /// and injected as environment variables into the container
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<ExternalDatabaseConfig>,

    /// Horizontal Pod Autoscaling configuration
    /// Only applicable to Horizon and SorobanRpc nodes
    /// Validators do not support autoscaling (always 1 replica)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscaling: Option<AutoscalingConfig>,

    /// Ingress configuration for HTTPS exposure via an ingress controller and cert-manager
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<IngressConfig>,

    /// Maintenance mode (skips workload updates)
    #[serde(default)]
    pub maintenance_mode: bool,
}

fn default_replicas() -> i32 {
    1
}

impl StellarNodeSpec {
    /// Validate the spec based on node type
    pub fn validate(&self) -> Result<(), String> {
        match self.node_type {
            NodeType::Validator => {
                if self.validator_config.is_none() {
                    return Err("validatorConfig is required for Validator nodes".to_string());
                }
                if let Some(vc) = &self.validator_config {
                    if vc.enable_history_archive && vc.history_archive_urls.is_empty() {
                        return Err(
                            "historyArchiveUrls must not be empty when enableHistoryArchive is true"
                                .to_string(),
                        );
                    }
                }
                if self.replicas != 1 {
                    return Err("Validator nodes must have exactly 1 replica".to_string());
                }
                if self.autoscaling.is_some() {
                    return Err("autoscaling is not supported for Validator nodes".to_string());
                }
                if self.ingress.is_some() {
                    return Err("ingress is not supported for Validator nodes".to_string());
                }
            }
            NodeType::Horizon => {
                if self.horizon_config.is_none() {
                    return Err("horizonConfig is required for Horizon nodes".to_string());
                }
                if let Some(ref autoscaling) = self.autoscaling {
                    if autoscaling.min_replicas < 1 {
                        return Err("autoscaling.minReplicas must be at least 1".to_string());
                    }
                    if autoscaling.max_replicas < autoscaling.min_replicas {
                        return Err("autoscaling.maxReplicas must be >= minReplicas".to_string());
                    }
                }
                if let Some(ingress) = &self.ingress {
                    validate_ingress(ingress)?;
                }
            }
            NodeType::SorobanRpc => {
                if self.soroban_config.is_none() {
                    return Err("sorobanConfig is required for SorobanRpc nodes".to_string());
                }
                if let Some(ref autoscaling) = self.autoscaling {
                    if autoscaling.min_replicas < 1 {
                        return Err("autoscaling.minReplicas must be at least 1".to_string());
                    }
                    if autoscaling.max_replicas < autoscaling.min_replicas {
                        return Err("autoscaling.maxReplicas must be >= minReplicas".to_string());
                    }
                }
                if let Some(ingress) = &self.ingress {
                    validate_ingress(ingress)?;
                }
            }
        }
        Ok(())
    }

    /// Get the container image for this node type and version
    pub fn container_image(&self) -> String {
        match self.node_type {
            NodeType::Validator => format!("stellar/stellar-core:{}", self.version),
            NodeType::Horizon => format!("stellar/stellar-horizon:{}", self.version),
            NodeType::SorobanRpc => format!("stellar/soroban-rpc:{}", self.version),
        }
    }

    /// Check if PVC should be deleted on node deletion
    pub fn should_delete_pvc(&self) -> bool {
        self.storage.retention_policy == RetentionPolicy::Delete
    }
}

fn validate_ingress(ingress: &IngressConfig) -> Result<(), String> {
    if ingress.hosts.is_empty() {
        return Err("ingress.hosts must not be empty".to_string());
    }

    for host in &ingress.hosts {
        if host.host.trim().is_empty() {
            return Err("ingress.hosts[].host must not be empty".to_string());
        }
        if host.paths.is_empty() {
            return Err("ingress.hosts[].paths must not be empty".to_string());
        }
        for path in &host.paths {
            if path.path.trim().is_empty() {
                return Err("ingress.hosts[].paths[].path must not be empty".to_string());
            }
            if let Some(path_type) = &path.path_type {
                let allowed = path_type == "Prefix" || path_type == "Exact";
                if !allowed {
                    return Err(
                        "ingress.hosts[].paths[].pathType must be either Prefix or Exact"
                            .to_string(),
                    );
                }
            }
        }
    }

    Ok(())
}

/// Status subresource for StellarNode
///
/// Reports the current state of the managed Stellar node.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeStatus {
    /// Current phase of the node lifecycle
    /// (Pending, Creating, Running, Syncing, Ready, Failed, Terminating)
    pub phase: String,

    /// Human-readable message about current state
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Observed generation for status sync detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,

    /// Readiness conditions following Kubernetes conventions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,

    /// For validators: current ledger sequence number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_sequence: Option<u64>,

    /// Endpoint where the node is accessible (Service ClusterIP or external)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Current number of ready replicas
    #[serde(default)]
    pub ready_replicas: i32,

    /// Total number of desired replicas
    #[serde(default)]
    pub replicas: i32,
}

impl StellarNodeStatus {
    /// Create a new status with the given phase
    pub fn with_phase(phase: &str) -> Self {
        Self {
            phase: phase.to_string(),
            ..Default::default()
        }
    }

    /// Update the phase and message
    pub fn update(&mut self, phase: &str, message: Option<&str>) {
        self.phase = phase.to_string();
        self.message = message.map(String::from);
    }

    /// Check if the node is ready
    pub fn is_ready(&self) -> bool {
        self.phase == "Ready" && self.ready_replicas >= self.replicas
    }
}
