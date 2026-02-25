//! StellarNode Custom Resource Definition
//!
//! The StellarNode CRD represents a managed Stellar infrastructure node.
//! Supports Validator (Core), Horizon API, and Soroban RPC node types.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{
    AutoscalingConfig, Condition, CrossClusterConfig, DisasterRecoveryConfig,
    DisasterRecoveryStatus, ExternalDatabaseConfig, GlobalDiscoveryConfig, HistoryMode,
    HorizonConfig, IngressConfig, LoadBalancerConfig, ManagedDatabaseConfig, NetworkPolicyConfig,
    NodeType, ResourceRequirements, RetentionPolicy, RolloutStrategy, SorobanConfig,
    StellarNetwork, StorageConfig, ValidatorConfig, VpaConfig,
};

/// Structured validation error for `StellarNodeSpec`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecValidationError {
    pub field: String,
    pub message: String,
    pub how_to_fix: String,
}

impl SpecValidationError {
    pub fn new(
        field: impl Into<String>,
        message: impl Into<String>,
        how_to_fix: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            how_to_fix: how_to_fix.into(),
        }
    }
}

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
    pub node_type: NodeType,
    pub network: StellarNetwork,
    pub version: String,

    #[serde(default)]
    pub history_mode: HistoryMode,

    #[serde(default)]
    pub resources: ResourceRequirements,

    #[serde(default)]
    pub storage: StorageConfig,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator_config: Option<ValidatorConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_config: Option<HorizonConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub soroban_config: Option<SorobanConfig>,

    #[serde(default = "default_replicas")]
    pub replicas: i32,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub min_available: Option<IntOrString>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub max_unavailable: Option<IntOrString>,

    #[serde(default)]
    pub suspended: bool,

    #[serde(default)]
    pub alerting: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<ExternalDatabaseConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_database: Option<ManagedDatabaseConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscaling: Option<AutoscalingConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpa_config: Option<VpaConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<IngressConfig>,

    /// Load balancer configuration for external access (e.g. MetalLB)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_balancer: Option<LoadBalancerConfig>,

    /// Global discovery configuration for cross-cluster discovery
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_discovery: Option<GlobalDiscoveryConfig>,

    /// Cross-cluster configuration for multi-cluster federation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_cluster: Option<CrossClusterConfig>,

    /// Rollout strategy for updates (RollingUpdate or Canary)
    #[serde(default)]
    pub strategy: RolloutStrategy,

    #[serde(default)]
    pub maintenance_mode: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_policy: Option<NetworkPolicyConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dr_config: Option<DisasterRecoveryConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Vec<serde_json::Value>>")]
    pub topology_spread_constraints:
        Option<Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint>>,

    /// CVE handling configuration for automated patching
    /// Enables scanning for vulnerabilities and automatic rollout of patched versions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cve_handling: Option<super::types::CVEHandlingConfig>,

    /// Read replica pool configuration for horizontal scaling
    /// Enables creating read-only replicas with traffic routing strategies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_replica_config: Option<super::read_replica::ReadReplicaConfig>,

    #[schemars(skip)]
    pub resource_meta: Option<ObjectMeta>,
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
    /// # history_mode: Default::default(),
    /// # resources: Default::default(),
    /// # storage: Default::default(),
    /// # validator_config: None,
    /// # horizon_config: None,
    /// # soroban_config: None,
    /// # replicas: 1,
    /// # min_available: None,
    /// # max_unavailable: None,
    /// # suspended: false,
    /// # alerting: false,
    /// # database: None,
    /// # managed_database: None,
    /// # autoscaling: None,
    /// # ingress: None,
    /// # load_balancer: None,
    /// # global_discovery: None,
    /// # cross_cluster: None,
    /// # strategy: Default::default(),
    /// # maintenance_mode: false,
    /// # network_policy: None,
    /// # dr_config: None,
    /// # topology_spread_constraints: None,
    /// # cve_handling: None,
    /// # read_replica_config: None,
    /// # vpa_config: None,
    /// # resource_meta: None,
    /// };
    /// match spec.validate() {
    ///     Ok(_) => println!("Valid spec"),
    ///     Err(errors) => {
    ///         for e in errors {
    ///             eprintln!("Validation error in {}: {}", e.field, e.message);
    ///         }
    ///     }
    /// }
    /// ```
    pub fn validate(&self) -> Result<(), Vec<SpecValidationError>> {
        let mut errors: Vec<SpecValidationError> = Vec::new();

        // 1. Database Mutual Exclusion
        if self.database.is_some() && self.managed_database.is_some() {
            errors.push(SpecValidationError::new(
                "spec.database / spec.managedDatabase",
                "Cannot specify both database (external) and managedDatabase",
                "Choose either an external database using spec.database or a managed one using spec.managedDatabase.",
            ));
        }

        // 2. PDB Conflict Check
        if self.min_available.is_some() && self.max_unavailable.is_some() {
            errors.push(SpecValidationError::new(
                "spec.minAvailable / spec.maxUnavailable",
                "Cannot specify both minAvailable and maxUnavailable in PDB configuration",
                "Set either spec.minAvailable or spec.maxUnavailable in the spec, but not both at the same time.",
            ));
        }

        // 3. Node Type Specific Logic
        match self.node_type {
            NodeType::Validator => {
                // Validator config required
                if self.validator_config.is_none() {
                    errors.push(SpecValidationError::new(
                        "spec.validatorConfig",
                        "validatorConfig is required for Validator nodes",
                        "Add a spec.validatorConfig section with the required validator settings when nodeType is Validator.",
                    ));
                } else if let Some(vc) = &self.validator_config {
                    if vc.enable_history_archive && vc.history_archive_urls.is_empty() {
                        errors.push(SpecValidationError::new(
                            "spec.validatorConfig.historyArchiveUrls",
                            "historyArchiveUrls must not be empty when enableHistoryArchive is true",
                            "Provide at least one valid history archive URL in spec.validatorConfig.historyArchiveUrls when enableHistoryArchive is true.",
                        ));
                    }
                }

                // Exactly 1 replica required
                if self.replicas != 1 {
                    errors.push(SpecValidationError::new(
                        "spec.replicas",
                        "Validator nodes must have exactly 1 replica",
                        "Set spec.replicas to 1 for Validator nodes.",
                    ));
                }
                if self.min_available.is_some() || self.max_unavailable.is_some() {
                    errors.push(SpecValidationError::new(
                        "spec.minAvailable / spec.maxUnavailable",
                        "PDB configuration is not supported for Validator nodes (replicas must be 1)",
                        "Remove PodDisruptionBudget fields (minAvailable/maxUnavailable) for Validator nodes; they must always have exactly 1 replica.",
                    ));
                }
                if self.autoscaling.is_some() {
                    errors.push(SpecValidationError::new(
                        "spec.autoscaling",
                        "autoscaling is not supported for Validator nodes",
                        "Remove spec.autoscaling when nodeType is Validator; autoscaling is only supported for Horizon and SorobanRpc.",
                    ));
                }

                // Ingress not supported
                if self.ingress.is_some() {
                    errors.push(SpecValidationError::new(
                        "spec.ingress",
                        "ingress is not supported for Validator nodes",
                        "Remove spec.ingress for Validator nodes; expose Validator nodes using peer discovery or other supported mechanisms.",
                    ));
                }
                // Canary strategy not supported
                if matches!(self.strategy, RolloutStrategy::Canary(_)) {
                    errors.push(SpecValidationError::new(
                        "spec.strategy",
                        "canary rollout strategy is not supported for Validator nodes",
                        "Use RollingUpdate strategy for Validator nodes; canary is only supported for Horizon and SorobanRpc.",
                    ));
                }
            }
            NodeType::Horizon => {
                // Horizon config required
                if self.horizon_config.is_none() {
                    errors.push(SpecValidationError::new(
                        "spec.horizonConfig",
                        "horizonConfig is required for Horizon nodes",
                        "Add a spec.horizonConfig section with the required Horizon settings when nodeType is Horizon.",
                    ));
                }
                if let Some(ref autoscaling) = self.autoscaling {
                    if autoscaling.min_replicas < 1 {
                        errors.push(SpecValidationError::new(
                            "spec.autoscaling.minReplicas",
                            "autoscaling.minReplicas must be at least 1",
                            "Set spec.autoscaling.minReplicas to 1 or greater.",
                        ));
                    }
                    if autoscaling.max_replicas < autoscaling.min_replicas {
                        errors.push(SpecValidationError::new(
                            "spec.autoscaling.maxReplicas",
                            "autoscaling.maxReplicas must be >= minReplicas",
                            "Set spec.autoscaling.maxReplicas to be greater than or equal to minReplicas.",
                        ));
                    }
                }
                if let Some(ingress) = &self.ingress {
                    validate_ingress(ingress, &mut errors);
                }
            }
            NodeType::SorobanRpc => {
                // Soroban config required
                if self.soroban_config.is_none() {
                    errors.push(SpecValidationError::new(
                        "spec.sorobanConfig",
                        "sorobanConfig is required for SorobanRpc nodes",
                        "Add a spec.sorobanConfig section with the required Soroban RPC settings when nodeType is SorobanRpc.",
                    ));
                }
                if let Some(ref autoscaling) = self.autoscaling {
                    if autoscaling.min_replicas < 1 {
                        errors.push(SpecValidationError::new(
                            "spec.autoscaling.minReplicas",
                            "autoscaling.minReplicas must be at least 1",
                            "Set spec.autoscaling.minReplicas to 1 or greater.",
                        ));
                    }
                    if autoscaling.max_replicas < autoscaling.min_replicas {
                        errors.push(SpecValidationError::new(
                            "spec.autoscaling.maxReplicas",
                            "autoscaling.maxReplicas must be >= minReplicas",
                            "Set spec.autoscaling.maxReplicas to be greater than or equal to minReplicas.",
                        ));
                    }
                }
                if let Some(ingress) = &self.ingress {
                    validate_ingress(ingress, &mut errors);
                }
            }
        }

        // Validate optional features if present
        if let Some(ref lb) = self.load_balancer {
            validate_load_balancer(lb, &mut errors);
        }
        if let Some(ref gd) = self.global_discovery {
            validate_global_discovery(gd, &mut errors);
        }
        if let Some(ref cc) = self.cross_cluster {
            validate_cross_cluster(cc, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn container_image(&self) -> String {
        let name = match self.node_type {
            NodeType::Validator => "stellar-core",
            _ => "horizon",
        };
        format!("stellar/{}:{}", name, self.version)
    }

    pub fn should_delete_pvc(&self) -> bool {
        self.storage.retention_policy == RetentionPolicy::Delete
    }
}
#[allow(dead_code)]
fn validate_ingress(ingress: &IngressConfig, errors: &mut Vec<SpecValidationError>) {
    if ingress.hosts.is_empty() {
        errors.push(SpecValidationError::new(
            "spec.ingress.hosts",
            "ingress.hosts must not be empty",
            "Provide at least one host entry under spec.ingress.hosts.",
        ));
        return;
    }

    for host in &ingress.hosts {
        if host.host.trim().is_empty() {
            errors.push(SpecValidationError::new(
                "spec.ingress.hosts[].host",
                "ingress.hosts[].host must not be empty",
                "Set a non-empty hostname for each ingress host entry.",
            ));
            continue;
        }
        if host.paths.is_empty() {
            errors.push(SpecValidationError::new(
                "spec.ingress.hosts[].paths",
                "ingress.hosts[].paths must not be empty",
                "Provide at least one path under spec.ingress.hosts[].paths for each host.",
            ));
            continue;
        }
        for path in &host.paths {
            if path.path.trim().is_empty() {
                errors.push(SpecValidationError::new(
                    "spec.ingress.hosts[].paths[].path",
                    "ingress.hosts[].paths[].path must not be empty",
                    "Set a non-empty HTTP path for each ingress path entry.",
                ));
            }
            if let Some(path_type) = &path.path_type {
                let allowed = path_type == "Prefix" || path_type == "Exact";
                if !allowed {
                    errors.push(SpecValidationError::new(
                        "spec.ingress.hosts[].paths[].pathType",
                        "ingress.hosts[].paths[].pathType must be either Prefix or Exact",
                        "Set pathType to either \"Prefix\" or \"Exact\" for each ingress path.",
                    ));
                }
            }
        }
    }
}

#[allow(dead_code)]
fn validate_load_balancer(lb: &LoadBalancerConfig, errors: &mut Vec<SpecValidationError>) {
    use super::types::LoadBalancerMode;

    if !lb.enabled {
        return;
    }

    // BGP mode requires peers configuration
    if lb.mode == LoadBalancerMode::BGP {
        if let Some(bgp) = &lb.bgp {
            if bgp.local_asn == 0 {
                errors.push(SpecValidationError::new(
                    "spec.loadBalancer.bgp.localASN",
                    "loadBalancer.bgp.localASN must be a valid ASN (1-4294967295)",
                    "Set spec.loadBalancer.bgp.localASN to a value between 1 and 4294967295.",
                ));
            }
            if bgp.peers.is_empty() {
                errors.push(SpecValidationError::new(
                    "spec.loadBalancer.bgp.peers",
                    "loadBalancer.bgp.peers must not be empty when using BGP mode",
                    "Provide at least one BGP peer under spec.loadBalancer.bgp.peers when mode is BGP.",
                ));
            }
            for (i, peer) in bgp.peers.iter().enumerate() {
                if peer.address.trim().is_empty() {
                    errors.push(SpecValidationError::new(
                        format!("spec.loadBalancer.bgp.peers[{i}].address"),
                        "loadBalancer.bgp.peers[].address must not be empty",
                        "Set a valid IP or hostname for each BGP peer address.",
                    ));
                }
                if peer.asn == 0 {
                    errors.push(SpecValidationError::new(
                        format!("spec.loadBalancer.bgp.peers[{i}].asn"),
                        "loadBalancer.bgp.peers[].asn must be a valid ASN",
                        "Set spec.loadBalancer.bgp.peers[].asn to a value between 1 and 4294967295.",
                    ));
                }
            }
        } else {
            errors.push(SpecValidationError::new(
                "spec.loadBalancer.bgp",
                "loadBalancer.bgp configuration is required when mode is BGP",
                "Add a spec.loadBalancer.bgp section when using BGP load balancer mode.",
            ));
        }
    }

    // Validate health check port range
    if lb.health_check_enabled && (lb.health_check_port < 1 || lb.health_check_port > 65535) {
        errors.push(SpecValidationError::new(
            "spec.loadBalancer.healthCheckPort",
            "loadBalancer.healthCheckPort must be between 1 and 65535",
            "Set spec.loadBalancer.healthCheckPort to a value between 1 and 65535.",
        ));
    }
}

#[allow(dead_code)]
fn validate_global_discovery(gd: &GlobalDiscoveryConfig, errors: &mut Vec<SpecValidationError>) {
    if !gd.enabled {
        return;
    }

    // Validate external DNS if configured
    if let Some(dns) = &gd.external_dns {
        if dns.hostname.trim().is_empty() {
            errors.push(SpecValidationError::new(
                "spec.globalDiscovery.externalDns.hostname",
                "globalDiscovery.externalDns.hostname must not be empty",
                "Set a non-empty hostname for spec.globalDiscovery.externalDns.hostname.",
            ));
        }
        if dns.ttl == 0 {
            errors.push(SpecValidationError::new(
                "spec.globalDiscovery.externalDns.ttl",
                "globalDiscovery.externalDns.ttl must be greater than 0",
                "Set spec.globalDiscovery.externalDns.ttl to a value greater than 0.",
            ));
        }
    }
}

#[allow(dead_code)]
fn validate_cross_cluster(cc: &CrossClusterConfig, errors: &mut Vec<SpecValidationError>) {
    use super::types::{CrossClusterMeshType, CrossClusterMode};

    if !cc.enabled {
        return;
    }

    // Validate service mesh configuration
    if cc.mode == CrossClusterMode::ServiceMesh {
        if let Some(mesh) = &cc.service_mesh {
            if mesh.cluster_set_id.is_none()
                && (mesh.mesh_type == CrossClusterMeshType::Submariner
                    || mesh.mesh_type == CrossClusterMeshType::Istio)
            {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.serviceMesh.clusterSetId",
                    "crossCluster.serviceMesh.clusterSetId is required for Submariner and Istio",
                    "Set spec.crossCluster.serviceMesh.clusterSetId when using Submariner or Istio mesh types.",
                ));
            }
        } else {
            errors.push(SpecValidationError::new(
                "spec.crossCluster.serviceMesh",
                "crossCluster.serviceMesh configuration is required when mode is ServiceMesh",
                "Add a spec.crossCluster.serviceMesh section when crossCluster.mode is ServiceMesh.",
            ));
        }
    }

    // Validate ExternalName configuration
    if cc.mode == CrossClusterMode::ExternalName {
        if let Some(ext) = &cc.external_name {
            if ext.external_dns_name.trim().is_empty() {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.externalName.externalDnsName",
                    "crossCluster.externalName.externalDnsName must not be empty",
                    "Set a non-empty DNS name for spec.crossCluster.externalName.externalDnsName.",
                ));
            }
        } else {
            errors.push(SpecValidationError::new(
                "spec.crossCluster.externalName",
                "crossCluster.externalName configuration is required when mode is ExternalName",
                "Add a spec.crossCluster.externalName section when crossCluster.mode is ExternalName.",
            ));
        }
    }

    // Validate peer clusters
    for (i, peer) in cc.peer_clusters.iter().enumerate() {
        if peer.cluster_id.trim().is_empty() {
            errors.push(SpecValidationError::new(
                format!("spec.crossCluster.peerClusters[{i}].clusterId"),
                "crossCluster.peerClusters[].clusterId must not be empty",
                "Set a non-empty identifier for each entry in spec.crossCluster.peerClusters[].clusterId.",
            ));
        }
        if peer.endpoint.trim().is_empty() {
            errors.push(SpecValidationError::new(
                format!("spec.crossCluster.peerClusters[{i}].endpoint"),
                "crossCluster.peerClusters[].endpoint must not be empty",
                "Set a non-empty endpoint URL for each entry in spec.crossCluster.peerClusters[].endpoint.",
            ));
        }
        if let Some(threshold) = peer.latency_threshold_ms {
            if threshold == 0 {
                errors.push(SpecValidationError::new(
                    format!(
                        "spec.crossCluster.peerClusters[{i}].latencyThresholdMs"
                    ),
                    "crossCluster.peerClusters[].latencyThresholdMs must be greater than 0",
                    "Set spec.crossCluster.peerClusters[].latencyThresholdMs to a value greater than 0.",
                ));
            }
        }
    }

    // Validate latency threshold
    if cc.latency_threshold_ms == 0 {
        errors.push(SpecValidationError::new(
            "spec.crossCluster.latencyThresholdMs",
            "crossCluster.latencyThresholdMs must be greater than 0",
            "Set spec.crossCluster.latencyThresholdMs to a value greater than 0.",
        ));
    }

    // Validate health check configuration
    if let Some(hc) = &cc.health_check {
        if hc.enabled {
            if hc.interval_seconds == 0 {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.healthCheck.intervalSeconds",
                    "crossCluster.healthCheck.intervalSeconds must be greater than 0",
                    "Set spec.crossCluster.healthCheck.intervalSeconds to a value greater than 0.",
                ));
            }
            if hc.timeout_seconds == 0 {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.healthCheck.timeoutSeconds",
                    "crossCluster.healthCheck.timeoutSeconds must be greater than 0",
                    "Set spec.crossCluster.healthCheck.timeoutSeconds to a value greater than 0.",
                ));
            }
            if hc.timeout_seconds >= hc.interval_seconds {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.healthCheck.timeoutSeconds",
                    "crossCluster.healthCheck.timeoutSeconds must be less than intervalSeconds",
                    "Set spec.crossCluster.healthCheck.timeoutSeconds to a value lower than intervalSeconds.",
                ));
            }
            if hc.failure_threshold == 0 {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.healthCheck.failureThreshold",
                    "crossCluster.healthCheck.failureThreshold must be greater than 0",
                    "Set spec.crossCluster.healthCheck.failureThreshold to a value greater than 0.",
                ));
            }
            if hc.success_threshold == 0 {
                errors.push(SpecValidationError::new(
                    "spec.crossCluster.healthCheck.successThreshold",
                    "crossCluster.healthCheck.successThreshold must be greater than 0",
                    "Set spec.crossCluster.healthCheck.successThreshold to a value greater than 0.",
                ));
            }

            // Validate latency measurement
            if let Some(lm) = &hc.latency_measurement {
                if lm.enabled {
                    if lm.sample_count == 0 {
                        errors.push(SpecValidationError::new(
                            "spec.crossCluster.healthCheck.latencyMeasurement.sampleCount",
                            "crossCluster.healthCheck.latencyMeasurement.sampleCount must be greater than 0",
                            "Set sampleCount to a value greater than 0 in spec.crossCluster.healthCheck.latencyMeasurement.",
                        ));
                    }
                    if lm.percentile == 0 || lm.percentile > 100 {
                        errors.push(SpecValidationError::new(
                            "spec.crossCluster.healthCheck.latencyMeasurement.percentile",
                            "crossCluster.healthCheck.latencyMeasurement.percentile must be between 1 and 100",
                            "Set percentile to a value between 1 and 100 in spec.crossCluster.healthCheck.latencyMeasurement.",
                        ));
                    }
                }
            }
        }
    }
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

    /// Current number of ready canary replicas (for canary deployments)
    #[serde(default)]
    pub canary_ready_replicas: i32,

    /// Version deployed in the canary deployment (if active)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canary_version: Option<String>,

    /// Version of the database schema after last successful migration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_migrated_version: Option<String>,
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
    #[allow(deprecated)]
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
    #[allow(deprecated)]
    #[deprecated(since = "0.2.0", note = "Use set_condition helpers instead")]
    #[allow(deprecated)]
    pub fn update(&mut self, phase: &str, message: Option<&str>) {
        self.phase = phase.to_string();
        self.message = message.map(String::from);
    }
    #[allow(clippy::empty_line_after_doc_comments)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::types::{CanaryConfig, RolloutStrategy};

    #[test]
    fn test_validator_with_canary_should_fail() {
        let spec = StellarNodeSpec {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            history_mode: Default::default(),
            resources: Default::default(),
            storage: Default::default(),
            validator_config: Some(ValidatorConfig {
                seed_secret_ref: "test".to_string(),
                quorum_set: None,
                enable_history_archive: false,
                history_archive_urls: vec![],
                catchup_complete: false,
                key_source: Default::default(),
                kms_config: None,
                vl_source: None,
                hsm_config: None,
            }),
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
            strategy: RolloutStrategy::Canary(CanaryConfig {
                weight: 10,
                check_interval_seconds: 300,
            }),
            maintenance_mode: false,
            network_policy: None,
            dr_config: None,
            topology_spread_constraints: None,
            cross_cluster: None,
            cve_handling: None,
            read_replica_config: None,
            resource_meta: None,
            vpa_config: None,
        };

        assert!(spec.validate().is_err());
    }

    #[test] // Ensure this attribute is there
    fn test_horizon_with_canary_should_pass() {
        let spec = StellarNodeSpec {
            node_type: NodeType::Horizon,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            history_mode: Default::default(),
            resources: Default::default(),
            storage: Default::default(),
            validator_config: None,
            horizon_config: Some(HorizonConfig {
                database_secret_ref: "test".to_string(),
                enable_ingest: true,
                stellar_core_url: "http://core".to_string(),
                ingest_workers: 1,
                enable_experimental_ingestion: false,
                auto_migration: false,
            }),
            soroban_config: None,
            replicas: 3,
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
            strategy: RolloutStrategy::Canary(CanaryConfig {
                weight: 20,
                check_interval_seconds: 300,
            }),
            maintenance_mode: false,
            network_policy: None,
            dr_config: None,
            topology_spread_constraints: None,
            cross_cluster: None,
            cve_handling: None,
            read_replica_config: None,
            resource_meta: None,
            vpa_config: None,
        };

        assert!(spec.validate().is_ok());
    }
}
