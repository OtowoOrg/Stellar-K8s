//! StellarNode Custom Resource Definition
//!
//! The StellarNode CRD represents a managed Stellar infrastructure node.
//! Supports Validator (Core), Horizon API, and Soroban RPC node types.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{
    AutoscalingConfig, Condition, DisasterRecoveryConfig, DisasterRecoveryStatus,
    ExternalDatabaseConfig, GlobalDiscoveryConfig, HorizonConfig, IngressConfig,
    LoadBalancerConfig, NetworkPolicyConfig, NodeType, ResourceRequirements, RetentionPolicy,
    SorobanConfig, StellarNetwork, StorageConfig, ValidatorConfig,
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
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Replicas","type":"integer","jsonPath":".spec.replicas"}"#,
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
    /// Network Policy configuration for restricting ingress traffic
    /// When enabled, creates a deny-all policy with explicit allow rules
    /// for peer-to-peer (Validators), API access (Horizon/Soroban), and metrics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_policy: Option<NetworkPolicyConfig>,

    /// Configuration for cross-region multi-cluster disaster recovery (DR)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dr_config: Option<DisasterRecoveryConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "serde_json::Value")]
    pub topology_spread_constraints:
        Option<Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint>>,
}

fn default_replicas() -> i32 {
    1
}

impl StellarNodeSpec {
    /// Validate the spec based on node type
    ///
    /// Performs comprehensive validation of the StellarNodeSpec including:
    /// - Checking that required config for node type is present
    /// - Validating replica counts
    /// - Ensuring node-type-specific constraints (e.g., Validators can't autoscale)
    /// - Validating ingress configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the spec fails validation.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use stellar_k8s::crd::StellarNodeSpec;
    ///
    /// let spec = StellarNodeSpec {
    ///     // ... configuration
    /// # node_type: Default::default(),
    /// # network: Default::default(),
    /// # version: "v21".to_string(),
    /// # resources: Default::default(),
    /// # storage: Default::default(),
    /// # validator_config: None,
    /// # horizon_config: None,
    /// # soroban_config: None,
    /// # replicas: 1,
    /// # suspended: false,
    /// # alerting: false,
    /// # database: None,
    /// # autoscaling: None,
    /// # ingress: None,
    /// # network_policy: None,
    /// };
    /// match spec.validate() {
    ///     Ok(_) => println!("Valid spec"),
    ///     Err(e) => eprintln!("Validation error: {}", e),
    /// }
    /// ```
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

        // Validate load balancer configuration (all node types)
        // TODO: load_balancer field not yet implemented in StellarNodeSpec
        /*
        if let Some(lb) = &self.load_balancer {
            validate_load_balancer(lb)?;
        }
        */

        // Validate global discovery configuration
        // TODO: global_discovery field not yet implemented in StellarNodeSpec
        /*
        if let Some(gd) = &self.global_discovery {
            validate_global_discovery(gd)?;
        }
        */

        Ok(())
    }

    /// Get the container image for this node type and version
    ///
    /// Constructs the fully qualified container image URI based on the node type and version.
    /// The operator uses this image when creating Kubernetes Deployments and StatefulSets.
    ///
    /// # Returns
    ///
    /// A string in the format `stellar/{component}:{version}` where component is:
    /// - `stellar-core` for Validator nodes
    /// - `stellar-horizon` for Horizon nodes  
    /// - `soroban-rpc` for SorobanRpc nodes
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use stellar_k8s::crd::{StellarNodeSpec, NodeType};
    ///
    /// let spec = StellarNodeSpec {
    ///     node_type: NodeType::Validator,
    ///     version: "v21.0.0".to_string(),
    /// # network: Default::default(),
    /// # resources: Default::default(),
    /// # storage: Default::default(),
    /// # validator_config: None,
    /// # horizon_config: None,
    /// # soroban_config: None,
    /// # replicas: 1,
    /// # suspended: false,
    /// # alerting: false,
    /// # database: None,
    /// # autoscaling: None,
    /// # ingress: None,
    /// # network_policy: None,
    /// };
    /// assert_eq!(spec.container_image(), "stellar/stellar-core:v21.0.0");
    /// ```
    pub fn container_image(&self) -> String {
        match self.node_type {
            NodeType::Validator => format!("stellar/stellar-core:{}", self.version),
            NodeType::Horizon => format!("stellar/stellar-horizon:{}", self.version),
            NodeType::SorobanRpc => format!("stellar/soroban-rpc:{}", self.version),
        }
    }

    /// Check if PVC should be deleted on node deletion
    ///
    /// Returns true if the storage retention policy is set to Delete,
    /// indicating that the PersistentVolumeClaim should be deleted when the StellarNode is deleted.
    ///
    /// # Returns
    ///
    /// - `true` if `retention_policy == RetentionPolicy::Delete`
    /// - `false` if `retention_policy == RetentionPolicy::Retain`
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use stellar_k8s::crd::{StellarNodeSpec, RetentionPolicy, StorageConfig};
    ///
    /// let spec = StellarNodeSpec {
    ///     storage: StorageConfig {
    ///         retention_policy: RetentionPolicy::Delete,
    /// # storage_class: "standard".to_string(),
    /// # size: "100Gi".to_string(),
    /// # annotations: None,
    ///     },
    /// # node_type: Default::default(),
    /// # network: Default::default(),
    /// # version: "v21".to_string(),
    /// # resources: Default::default(),
    /// # validator_config: None,
    /// # horizon_config: None,
    /// # soroban_config: None,
    /// # replicas: 1,
    /// # suspended: false,
    /// # alerting: false,
    /// # database: None,
    /// # autoscaling: None,
    /// # ingress: None,
    /// # network_policy: None,
    /// };
    /// assert!(spec.should_delete_pvc());
    /// ```
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

fn validate_load_balancer(lb: &LoadBalancerConfig) -> Result<(), String> {
    use super::types::LoadBalancerMode;

    if !lb.enabled {
        return Ok(());
    }

    // BGP mode requires peers configuration
    if lb.mode == LoadBalancerMode::BGP {
        if let Some(bgp) = &lb.bgp {
            if bgp.local_asn == 0 {
                return Err(
                    "loadBalancer.bgp.localASN must be a valid ASN (1-4294967295)".to_string(),
                );
            }
            if bgp.peers.is_empty() {
                return Err(
                    "loadBalancer.bgp.peers must not be empty when using BGP mode".to_string(),
                );
            }
            for (i, peer) in bgp.peers.iter().enumerate() {
                if peer.address.trim().is_empty() {
                    return Err(format!(
                        "loadBalancer.bgp.peers[{}].address must not be empty",
                        i
                    ));
                }
                if peer.asn == 0 {
                    return Err(format!(
                        "loadBalancer.bgp.peers[{}].asn must be a valid ASN",
                        i
                    ));
                }
            }
        } else {
            return Err("loadBalancer.bgp configuration is required when mode is BGP".to_string());
        }
    }

    // Validate health check port range
    if lb.health_check_enabled && (lb.health_check_port < 1 || lb.health_check_port > 65535) {
        return Err("loadBalancer.healthCheckPort must be between 1 and 65535".to_string());
    }

    Ok(())
}

fn validate_global_discovery(gd: &GlobalDiscoveryConfig) -> Result<(), String> {
    if !gd.enabled {
        return Ok(());
    }

    // Validate external DNS if configured
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
///
/// Reports the current state of the managed Stellar node using Kubernetes conventions.
/// The operator continuously updates this status as the node progresses through its lifecycle.
///
/// # Node Phases
///
/// - `Pending` - Resource creation is queued but not started
/// - `Creating` - Infrastructure (Pod, Service, etc.) is being created
/// - `Running` - Pod is running but not yet synced
/// - `Syncing` - Node is syncing blockchain data (validators)
/// - `Ready` - Node is fully synced and operational
/// - `Failed` - Node encountered an unrecoverable error
/// - `Degraded` - Node is running but not fully healthy
/// - `Remediating` - Operator is attempting to recover the node
/// - `Terminating` - Node resources are being cleaned up
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeStatus {
    /// Current phase of the node lifecycle
    /// (Pending, Creating, Running, Syncing, Ready, Failed, Degraded, Remediating, Terminating)
    ///
    /// DEPRECATED: Use the conditions array instead. This field is maintained for backward compatibility
    /// and will be removed in a future version. The phase is now derived from the conditions.
    #[deprecated(
        since = "0.2.0",
        note = "Use conditions array instead. Phase is now derived from Ready/Progressing/Degraded conditions."
    )]
    pub phase: String,

    /// Human-readable message about current state
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Observed generation for status sync detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,

    /// Status of the cross-region disaster recovery setup (if enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dr_status: Option<DisasterRecoveryStatus>,

    /// Readiness conditions following Kubernetes conventions
    ///
    /// Standard conditions include:
    /// - Ready: True when all sub-resources are healthy and the node is operational
    /// - Progressing: True when the node is being created, updated, or syncing
    /// - Degraded: True when the node is operational but experiencing issues
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,

    /// For validators: current ledger sequence number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_sequence: Option<u64>,

    /// Endpoint where the node is accessible (Service ClusterIP or external)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// External load balancer IP assigned by MetalLB
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ip: Option<String>,

    /// BGP advertisement status (when using BGP mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgp_status: Option<BGPStatus>,

    /// Current number of ready replicas
    #[serde(default)]
    pub ready_replicas: i32,

    /// Total number of desired replicas
    #[serde(default)]
    pub replicas: i32,
}

/// BGP advertisement status information
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPStatus {
    /// Whether BGP sessions are established
    pub sessions_established: bool,

    /// Number of active BGP peers
    pub active_peers: i32,

    /// Advertised IP prefixes
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advertised_prefixes: Vec<String>,

    /// Last BGP update time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update: Option<String>,
}

impl StellarNodeStatus {
    /// Create a new status with the given phase
    ///

    /// Initializes a StellarNodeStatus with the provided phase and all other fields
    /// set to their defaults (empty message, no conditions, etc.).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use stellar_k8s::crd::StellarNodeStatus;
    ///
    /// let status = StellarNodeStatus::with_phase("Creating");
    /// assert_eq!(status.phase, "Creating");
    /// assert_eq!(status.message, None);
    /// ```

    /// DEPRECATED: Use `with_conditions` instead
    #[deprecated(since = "0.2.0", note = "Use with_conditions instead")]

    pub fn with_phase(phase: &str) -> Self {
        Self {
            phase: phase.to_string(),
            ..Default::default()
        }
    }

    /// Update the phase and message
    ///

    /// Updates both the phase and message fields atomically.
    /// This is typically called during reconciliation to report progress.
    ///
    /// # Arguments
    ///
    /// * `phase` - The new phase name (e.g., "Ready", "Syncing", "Failed")
    /// * `message` - Optional human-readable message explaining the phase
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use stellar_k8s::crd::StellarNodeStatus;
    ///
    /// let mut status = StellarNodeStatus::with_phase("Creating");
    /// status.update("Ready", Some("Node is fully synced"));
    /// assert_eq!(status.phase, "Ready");
    /// assert_eq!(status.message, Some("Node is fully synced".to_string()));
    /// ```

    /// DEPRECATED: Use condition helpers instead
    #[deprecated(since = "0.2.0", note = "Use set_condition helpers instead")]

    pub fn update(&mut self, phase: &str, message: Option<&str>) {
        self.phase = phase.to_string();
        self.message = message.map(String::from);
    }


    /// Check if the node is ready
    ///
    /// Returns true only if both:
    /// - The node phase is "Ready"
    /// - All desired replicas are reporting ready
    ///
    /// This is used by controllers and monitoring systems to determine if the node
    /// is fully operational.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use stellar_k8s::crd::StellarNodeStatus;
    ///
    /// let mut status = StellarNodeStatus::with_phase("Ready");
    /// status.ready_replicas = 1;
    /// status.replicas = 1;
    /// assert!(status.is_ready());
    ///
    /// // Not ready if replicas don't match
    /// status.ready_replicas = 0;
    /// assert!(!status.is_ready());
    /// ```

    /// Check if the node is ready based on conditions
    ///
    /// A node is considered ready when:
    /// - Ready condition is True
    /// - ready_replicas >= replicas (all replicas are ready)

    pub fn is_ready(&self) -> bool {
        let has_ready_condition = self
            .conditions
            .iter()
            .any(|c| c.type_ == "Ready" && c.status == "True");

        has_ready_condition && self.ready_replicas >= self.replicas
    }

    /// Check if the node is degraded
    pub fn is_degraded(&self) -> bool {
        self.conditions
            .iter()
            .any(|c| c.type_ == "Degraded" && c.status == "True")
    }

    /// Check if the node is progressing
    pub fn is_progressing(&self) -> bool {
        self.conditions
            .iter()
            .any(|c| c.type_ == "Progressing" && c.status == "True")
    }

    /// Get a condition by type
    pub fn get_condition(&self, condition_type: &str) -> Option<&Condition> {
        self.conditions.iter().find(|c| c.type_ == condition_type)
    }

    /// Derive phase from conditions for backward compatibility
    ///
    /// This allows existing code to continue using phase while we transition
    /// to conditions-based status reporting
    pub fn derive_phase_from_conditions(&self) -> String {
        if self.is_ready() {
            "Ready".to_string()
        } else if self.is_degraded() {
            "Degraded".to_string()
        } else if self.is_progressing() {
            "Progressing".to_string()
        } else {
            // Check for specific reasons
            if let Some(ready_cond) = self.get_condition("Ready") {
                if ready_cond.status == "False" {
                    match ready_cond.reason.as_str() {
                        "PodsPending" => "Pending".to_string(),
                        "Creating" => "Creating".to_string(),
                        _ => "NotReady".to_string(),
                    }
                } else {
                    "Unknown".to_string()
                }
            } else {
                "Pending".to_string()
            }
        }
    }
}
