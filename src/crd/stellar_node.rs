//! StellarNode Custom Resource Definition
//!
//! The StellarNode CRD represents a managed Stellar infrastructure node.
//! Supports Validator (Core), Horizon API, and Soroban RPC node types.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{
    AutoscalingConfig, Condition, ExternalDatabaseConfig, GlobalDiscoveryConfig, HorizonConfig,
    IngressConfig, LoadBalancerConfig, NetworkPolicyConfig, NodeType, ResourceRequirements,
    RetentionPolicy, SorobanConfig, StellarNetwork, StorageConfig, ValidatorConfig,
};

// --- NEW ENUM DEFINITION ---
/// Determines if the node keeps full history (Archival) or just recent ledgers.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum HistoryMode {
    /// Keeps complete history (High storage requirement, used for Archive Nodes)
    Full,
    /// Keeps only recent history (Lower storage, standard Validator/Horizon behavior)
    Recent,
}
// ---------------------------

/// The StellarNode CRD represents a managed Stellar infrastructure node.
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

    // --- NEW FIELD ---
    /// History retention mode (Full vs Recent)
    /// Defaults to Recent if not specified.
    #[serde(default = "default_history_mode")]
    pub history_mode: HistoryMode,
    // -----------------

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator_config: Option<ValidatorConfig>,

    /// Horizon API server configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_config: Option<HorizonConfig>,

    /// Soroban RPC configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soroban_config: Option<SorobanConfig>,

    /// Number of replicas
    #[serde(default = "default_replicas")]
    pub replicas: i32,

    /// Suspend the node (scale to 0 without deleting resources)
    #[serde(default)]
    pub suspended: bool,

    /// Enable alerting via PrometheusRule or ConfigMap
    #[serde(default)]
    pub alerting: bool,

    /// External database configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<ExternalDatabaseConfig>,

    /// Horizontal Pod Autoscaling configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscaling: Option<AutoscalingConfig>,

    /// Ingress configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<IngressConfig>,

    /// Maintenance mode (skips workload updates)
    #[serde(default)]
    pub maintenance_mode: bool,

    /// Network Policy configuration for restricting ingress traffic
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_policy: Option<NetworkPolicyConfig>,

    /// Load Balancer configuration (MetalLB/BGP)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_balancer: Option<LoadBalancerConfig>,

    /// Global Discovery (GDS) configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_discovery: Option<GlobalDiscoveryConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "serde_json::Value")]
    pub topology_spread_constraints: Option<Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint>>,
}

fn default_replicas() -> i32 {
    1
}

// --- NEW DEFAULT FUNCTION ---
fn default_history_mode() -> HistoryMode {
    HistoryMode::Recent
}
// ---------------------------

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

        // Validate load balancer configuration
        if let Some(lb) = &self.load_balancer {
            validate_load_balancer(lb)?;
        }

        // Validate global discovery configuration
        if let Some(gd) = &self.global_discovery {
            validate_global_discovery(gd)?;
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
                        "ingress.hosts[].paths[].pathType must be either Prefix or Exact".to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_load_balancer(lb: &LoadBalancerConfig) -> Result<(), String> {
    use super::types::LoadBalancerMode;

    if !lb.enabled {
        return Ok(());
    }

    if lb.mode == LoadBalancerMode::BGP {
        if let Some(bgp) = &lb.bgp {
            if bgp.local_asn == 0 {
                return Err(
                    "loadBalancer.bgp.localASN must be a valid ASN (1-4294967295)".to_string(),
                );
            }
            if bgp.peers.is_empty() {
                return Err("loadBalancer.bgp.peers must not be empty when using BGP mode".to_string());
            }
            for (i, peer) in bgp.peers.iter().enumerate() {
                if peer.address.trim().is_empty() {
                    return Err(format!("loadBalancer.bgp.peers[{}].address must not be empty", i));
                }
                if peer.asn == 0 {
                    return Err(format!("loadBalancer.bgp.peers[{}].asn must be a valid ASN", i));
                }
            }
        } else {
            return Err("loadBalancer.bgp configuration is required when mode is BGP".to_string());
        }
    }

    if lb.health_check_enabled && (lb.health_check_port < 1 || lb.health_check_port > 65535) {
        return Err("loadBalancer.healthCheckPort must be between 1 and 65535".to_string());
    }

    Ok(())
}

fn validate_global_discovery(gd: &GlobalDiscoveryConfig) -> Result<(), String> {
    if !gd.enabled {
        return Ok(());
    }
    if let Some(dns) = &gd.external_dns {
        if dns.hostname.trim().is_empty() {
            return Err("globalDiscovery.externalDns.hostname must not be empty".to_string());
        }
        if dns.ttl == 0 {
            return Err("globalDiscovery.externalDns.ttl must be greater than 0".to_string());
        }
    }
    Ok(())
}

/// Status subresource for StellarNode
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeStatus {
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgp_status: Option<BGPStatus>,
    #[serde(default)]
    pub ready_replicas: i32,
    #[serde(default)]
    pub replicas: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPStatus {
    pub sessions_established: bool,
    pub active_peers: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advertised_prefixes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update: Option<String>,
}

impl StellarNodeStatus {
    pub fn with_phase(phase: &str) -> Self {
        Self {
            phase: phase.to_string(),
            ..Default::default()
        }
    }
    pub fn update(&mut self, phase: &str, message: Option<&str>) {
        self.phase = phase.to_string();
        self.message = message.map(String::from);
    }
    pub fn is_ready(&self) -> bool {
        self.phase == "Ready" && self.ready_replicas >= self.replicas
    }
}
