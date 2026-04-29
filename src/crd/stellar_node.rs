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
    AutoscalingConfig, CertManagerConfig, Condition, CoreSyncState, CrossClusterConfig,
    DisasterRecoveryConfig, DisasterRecoveryStatus, ExternalDatabaseConfig, ForensicSnapshotConfig,
    GasAutoscalingConfig, GlobalDiscoveryConfig, HistoryMode, HorizonConfig, IngressConfig,
    LabelPropagationConfig, LoadBalancerConfig, LogShipperConfig, ManagedDatabaseConfig,
    NetworkPolicyConfig, NodeType, OciSnapshotConfig, OperatorRole, PlacementConfig,
    PodAntiAffinityStrength, PolicyConfig, ProbeConfig, RbacConfig, ResourceRequirements,
    RestoreFromSnapshotConfig, RetentionPolicy, RolloutStrategy, SnapshotScheduleConfig,
    SorobanConfig, StellarNetwork, StorageConfig, SyncStateScalingConfig, ValidatorConfig,
    VpaConfig, AuditConfig,
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
    /// The role of this node in the network (Validator, Horizon, SorobanRPC).
    pub node_type: NodeType,

    /// The network this node connects to (Public, Testnet, or Futurenet).
    pub network: StellarNetwork,

    /// Custom network passphrase (required if network is 'Custom').
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_network_passphrase: Option<String>,

    /// Version of the Stellar software to run (e.g., "v21.0.0").
    pub version: String,

    /// How the node should handle history archives (Full, Fast, or Minimal).
    #[serde(default)]
    pub history_mode: HistoryMode,

    /// Resource limits and requests for the node container.
    #[serde(default)]
    pub resources: ResourceRequirements,

    /// Storage configuration, including PVC size and StorageClass.
    #[serde(default)]
    pub storage: StorageConfig,

    /// Configuration specific to Validator nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator_config: Option<ValidatorConfig>,

    /// DNS endpoint for the read-replica pool Service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_pool_endpoint: Option<String>,

    /// Configuration specific to Horizon nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_config: Option<HorizonConfig>,

    /// Configuration specific to Soroban RPC nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soroban_config: Option<SorobanConfig>,

    #[serde(default = "default_replicas")]
    pub replicas: i32,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "super::schema_utils::int_or_string_schema")]
    pub min_available: Option<IntOrString>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "super::schema_utils::int_or_string_schema")]
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

    /// Durable log-to-S3 sidecar configuration.
    /// When set and `enabled: true`, a `stellar-log-shipper` sidecar is injected
    /// into every managed pod to stream compressed logs directly to S3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_shipper: Option<LogShipperConfig>,
    /// Dynamic resource scaling based on Stellar Core sync state.
    ///
    /// When enabled, the operator boosts CPU/memory while the node is catching up
    /// on historical ledgers, then scales back once it reaches `Synced` state.
    /// Uses in-place pod resource updates (requires Kubernetes 1.27+ with
    /// `InPlacePodVerticalScaling` feature gate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_state_scaling: Option<SyncStateScalingConfig>,

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

    /// Optimize pod placement based on network latency to quorum peers
    #[serde(default)]
    pub proximity_aware: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_policy: Option<NetworkPolicyConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dr_config: Option<DisasterRecoveryConfig>,

    /// Multi-region ledger replication configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replication_config: Option<super::types::ReplicationConfig>,

    /// When not `Disabled`, the operator adds default pod anti-affinity so pods with the same
    /// `stellar-network` label (and same component) are not co-located on one node.
    #[serde(default)]
    pub pod_anti_affinity: PodAntiAffinityStrength,

    /// Intelligent pod placement configuration (e.g. SCP-aware anti-affinity)
    #[serde(default)]
    pub placement: PlacementConfig,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "super::schema_utils::array_of_objects_schema")]
    pub topology_spread_constraints:
        Option<Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint>>,

    /// CVE handling configuration for automated patching
    /// Enables scanning for vulnerabilities and automatic rollout of patched versions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cve_handling: Option<super::types::CVEHandlingConfig>,

    /// Schedule and options for taking CSI VolumeSnapshots of the node's data PVC (Validator only).
    /// Enables zero-downtime backups and creating new nodes from snapshots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_schedule: Option<SnapshotScheduleConfig>,

    /// Bootstrap this node from an existing VolumeSnapshot instead of an empty volume (Validator only).
    /// The PVC will be created from the specified snapshot for near-instant startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_from_snapshot: Option<RestoreFromSnapshotConfig>,

    /// Read replica pool configuration for horizontal scaling
    /// Enables creating read-only replicas with traffic routing strategies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_replica_config: Option<super::read_replica::ReadReplicaConfig>,

    /// Database maintenance configuration for automated vacuum and reindexing
    /// Enables periodic maintenance windows for performance optimization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_maintenance_config: Option<super::types::DbMaintenanceConfig>,
    /// OCI-based ledger snapshot sync for multi-region bootstrapping
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oci_snapshot: Option<OciSnapshotConfig>,

    /// Service mesh configuration (Istio/Linkerd) for mTLS and advanced traffic control
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_mesh: Option<super::service_mesh::ServiceMeshConfig>,

    /// cert-manager integration for automatic mTLS certificate issuance and rotation.
    ///
    /// When set, the operator creates a cert-manager `Certificate` resource for this node
    /// instead of issuing a self-signed certificate internally. cert-manager handles
    /// renewal automatically; the operator watches the resulting Secret and triggers a
    /// rolling restart when the certificate is rotated.
    ///
    /// Requires cert-manager v1.x to be installed in the cluster.
    ///
    /// # Example
    /// ```yaml
    /// certManager:
    ///   issuerRef:
    ///     name: letsencrypt-prod
    ///     kind: ClusterIssuer
    ///   duration: "2160h"
    ///   renewBefore: "720h"
    /// ```
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_manager: Option<CertManagerConfig>,

    /// Forensic snapshot: set `metadata.annotations["stellar.org/request-forensic-snapshot"]="true"`
    /// to trigger a one-shot capture (PCAP, optional core dump) uploaded to S3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forensic_snapshot: Option<ForensicSnapshotConfig>,

    /// NAT Traversal sidecar configuration (STUN/TURN/ICE) for P2P networking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nat_traversal: Option<super::types::NatTraversalConfig>,
    /// Label propagation filter policy for child resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_propagation: Option<LabelPropagationConfig>,

    #[schemars(skip)]
    pub resource_meta: Option<ObjectMeta>,

    /// Optional sidecar containers to run alongside the main Stellar container.
    ///
    /// These containers share the pod's network namespace and can mount the same
    /// volumes (e.g. `data`, `config`) as the main container. Useful for log
    /// forwarders, monitoring agents, proxies, etc.
    ///
    /// # Example
    /// ```yaml
    /// sidecars:
    ///   - name: log-forwarder
    ///     image: fluent/fluent-bit:latest
    ///     volumeMounts:
    ///       - name: data
    ///         mountPath: /stellar-data
    ///         readOnly: true
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Vec<serde_json::Value>>")]
    pub sidecars: Option<Vec<k8s_openapi::api::core::v1::Container>>,

    /// Optional overrides for the liveness, readiness, and startup probes on the main container.
    ///
    /// When set, the specified fields replace the operator's built-in probe defaults.
    /// Unset fields continue to use the operator defaults.
    ///
    /// # Example
    /// ```yaml
    /// probes:
    ///   liveness:
    ///     initialDelaySeconds: 60
    ///     periodSeconds: 15
    ///     failureThreshold: 5
    ///   readiness:
    ///     initialDelaySeconds: 20
    ///     periodSeconds: 10
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probes: Option<ProbeConfig>,
    /// Cross-cloud failover configuration for Horizon clusters.
    /// Enables seamless traffic failover between cloud providers (AWS, GCP, Azure)
    /// during major provider outages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_cloud_failover: Option<crate::crd::CrossCloudFailoverConfig>,

    /// Hitless upgrade configuration for zero-interruption Stellar Core upgrades.
    ///
    /// When enabled, the operator injects a handoff sidecar that transfers open
    /// peer TCP socket file descriptors to the new container via `SCM_RIGHTS`,
    /// avoiding peer re-discovery during version upgrades.
    ///
    /// Only applicable to `Validator` nodes.
    ///
    /// See `docs/hitless-upgrade.md` for the full design and feasibility study.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hitless_upgrade: Option<super::types::HitlessUpgradeConfig>,

    /// Proactive Failure Detection using eBPF.
    ///
    /// When enabled, an `ebpf-exporter` sidecar is injected into the Validator Pod
    /// to monitor silent failures like slow disk IO and network jitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ebpf_config: Option<super::types::EbpfConfig>,

    /// Automated secret rotation for database credentials.
    ///
    /// When enabled, the operator automatically rotates PostgreSQL passwords
    /// on a configurable schedule, updating both the database and Kubernetes
    /// secrets without downtime.
    ///
    /// Requires database configuration (either `database` or `managed_database`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_rotation: Option<crate::backup::SecretRotationConfig>,

    /// Automated backup verification via temporary clusters.
    ///
    /// When enabled, the operator periodically spins up temporary clusters,
    /// restores backups, and verifies data integrity to ensure backups are
    /// recoverable.
    ///
    /// Requires backup configuration and sufficient cluster resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_verification: Option<crate::backup::BackupVerificationConfig>,

    /// History archive pruning policy for automated retention management.
    /// Enables safe deletion of old checkpoints based on retention policies.
    /// Only applicable to `Validator` nodes with history archives enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruning_policy: Option<super::types::PruningPolicy>,

    /// RBAC configuration for operator-level permissions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rbac: Option<RbacConfig>,

    /// Audit logging configuration for operator actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<AuditConfig>,

    /// Policy-based authorization configuration (OPA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyConfig>,
}

fn default_replicas() -> i32 {
    1
}

impl Default for StellarNodeSpec {
    fn default() -> Self {
        Self {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            history_mode: Default::default(),
            resources: Default::default(),
            storage: Default::default(),
            validator_config: None,
            read_pool_endpoint: None,
            horizon_config: None,
            soroban_config: None,
            replicas: default_replicas(),
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
            strategy: Default::default(),
            maintenance_mode: false,
            proximity_aware: false,
            network_policy: None,
            dr_config: None,
            replication_config: None,
            pod_anti_affinity: Default::default(),
            placement: Default::default(),
            topology_spread_constraints: None,
            cve_handling: None,
            snapshot_schedule: None,
            restore_from_snapshot: None,
            read_replica_config: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            forensic_snapshot: None,
            nat_traversal: None,
            label_propagation: None,
            resource_meta: None,
            custom_network_passphrase: None,
            sidecars: None,
            cert_manager: None,
            probes: None,
            cross_cloud_failover: None,
            hitless_upgrade: None,
            ebpf_config: None,
            secret_rotation: None,
            backup_verification: None,
            log_shipper: None,
            sync_state_scaling: None,
            pruning_policy: None,
            rbac: None,
            audit: None,
            policy: None,
        }
    }
}

impl StellarNodeSpec {
    /// Get the network passphrase based on network type and custom string
    pub fn network_passphrase(&self) -> &str {
        self.network.passphrase(&self.custom_network_passphrase)
    }

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
    /// let spec = StellarNodeSpec::default();
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

        // 0. Custom network name validation
        if let Err(msg) = self.network.validate_custom_name() {
            errors.push(SpecValidationError::new(
                "spec.network.customName",
                msg,
                "Use only lowercase alphanumeric characters and hyphens, 1–63 characters, no leading/trailing hyphens (DNS-1123). Example: \"my-private-net\".",
            ));
        }

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

        // 2a. Storage Mode Validation
        if self.storage.mode == crate::crd::types::StorageMode::Local {
            // Usually local storage requires node alignment or specific classes
            if self.storage.node_affinity.is_none() && self.storage.storage_class.is_empty() {
                errors.push(SpecValidationError::new(
                     "spec.storage",
                     "LocalStorage mode requires either a specific storage_class or node_affinity to be set",
                     "Provide a node_affinity definition to pin the volume, or provide a Local StorageClass name.",
                 ));
            }
        }

        // 2b. snapshotRef validation (applies to all node types)
        if let Some(ref snap_ref) = self.storage.snapshot_ref {
            let has_csi = snap_ref.volume_snapshot_name.is_some();
            let has_backup = snap_ref.backup_url.is_some();

            if has_csi && has_backup {
                errors.push(SpecValidationError::new(
                    "spec.storage.snapshotRef",
                    "snapshotRef cannot specify both volumeSnapshotName and backupUrl simultaneously",
                    "Use either volumeSnapshotName (CSI VolumeSnapshot) or backupUrl (compressed archive), not both.",
                ));
            }
            if !has_csi && !has_backup {
                errors.push(SpecValidationError::new(
                    "spec.storage.snapshotRef",
                    "snapshotRef must specify either volumeSnapshotName or backupUrl",
                    "Set spec.storage.snapshotRef.volumeSnapshotName for CSI snapshot restore, or spec.storage.snapshotRef.backupUrl for compressed backup restore.",
                ));
            }
            if has_backup
                && snap_ref
                    .backup_url
                    .as_deref()
                    .map(|u| u.is_empty())
                    .unwrap_or(false)
            {
                errors.push(SpecValidationError::new(
                    "spec.storage.snapshotRef.backupUrl",
                    "backupUrl must not be empty when set",
                    "Provide a valid S3 or HTTPS URL for the compressed backup archive.",
                ));
            }
        }

        // 2c. Replication validation
        if let Some(ref repl_cfg) = self.replication_config {
            if repl_cfg.enabled {
                if self.managed_database.is_none() {
                    errors.push(SpecValidationError::new(
                        "spec.replicationConfig",
                        "Replication requires a managed database (spec.managedDatabase)",
                        "Configure a managed database using spec.managedDatabase to enable multi-region replication via CloudNativePG.",
                    ));
                }
                if repl_cfg.remote_cluster_id.is_empty() {
                    errors.push(SpecValidationError::new(
                        "spec.replicationConfig.remoteClusterId",
                        "remoteClusterId must not be empty when replication is enabled",
                        "Provide the identifier of the secondary/remote cluster in spec.replicationConfig.remoteClusterId.",
                    ));
                }
            }
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
                if self.strategy.canary().is_some() {
                    errors.push(SpecValidationError::new(
                        "spec.strategy",
                        "canary rollout strategy is not supported for Validator nodes",
                        "Use RollingUpdate strategy for Validator nodes; canary is only supported for Horizon and SorobanRpc.",
                    ));
                }

                // High-security seed handling for HSM-backed validators:
                // disallow seed sources that materialize the validator seed into Kubernetes Secrets (stored in etcd).
                if let Some(vc) = &self.validator_config {
                    if vc.hsm_config.is_some() {
                        match &vc.seed_secret_source {
                            Some(src) => {
                                if let Err(e) = src.validate() {
                                    errors.push(SpecValidationError::new(
                                        "spec.validatorConfig.seedSecretSource",
                                        format!("Invalid seedSecretSource: {e}"),
                                        "Configure exactly one of localRef, externalRef, csiRef, or vaultRef.",
                                    ));
                                } else {
                                    let uses_k8s_secret =
                                        src.local_ref.is_some() || src.external_ref.is_some();
                                    if uses_k8s_secret {
                                        errors.push(SpecValidationError::new(
                                            "spec.validatorConfig.seedSecretSource",
                                            "HSM config requires a seed source that does not materialize seeds into Kubernetes Secrets (etcd).",
                                            "Use seedSecretSource.csiRef (Secrets Store CSI) or seedSecretSource.vaultRef (Vault Agent Injector). Avoid seedSecretSource.localRef/externalRef.",
                                        ));
                                    }
                                }
                            }
                            None => {
                                // Legacy seedSecretRef is a plain Kubernetes Secret reference.
                                if !vc.seed_secret_ref.is_empty() {
                                    errors.push(SpecValidationError::new(
                                        "spec.validatorConfig.seedSecretRef",
                                        "HSM config forbids the legacy seedSecretRef (materializes into Kubernetes Secret / etcd).",
                                        "Switch to spec.validatorConfig.seedSecretSource.csiRef or spec.validatorConfig.seedSecretSource.vaultRef.",
                                    ));
                                } else {
                                    errors.push(SpecValidationError::new(
                                        "spec.validatorConfig.seedSecretSource",
                                        "HSM config requires seedSecretSource.csiRef or seedSecretSource.vaultRef.",
                                        "Configure a non-Kubernetes-Secret seed backend for high-security operation.",
                                    ));
                                }
                            }
                        }
                    }
                }
                // Snapshot schedule and restore only apply to Validators (ledger data)
                if (self.snapshot_schedule.is_some() || self.restore_from_snapshot.is_some())
                    && self
                        .restore_from_snapshot
                        .as_ref()
                        .map(|r| r.volume_snapshot_name.is_empty())
                        .unwrap_or(false)
                {
                    errors.push(SpecValidationError::new(
                        "spec.restoreFromSnapshot.volumeSnapshotName",
                        "volumeSnapshotName must not be empty when restoreFromSnapshot is set",
                        "Set spec.restoreFromSnapshot.volumeSnapshotName to an existing VolumeSnapshot name.",
                    ));
                }
            }
            NodeType::Horizon => {
                if self.snapshot_schedule.is_some() || self.restore_from_snapshot.is_some() {
                    errors.push(SpecValidationError::new(
                        "spec.snapshotSchedule / spec.restoreFromSnapshot",
                        "snapshot and restore are only supported for Validator nodes",
                        "Remove spec.snapshotSchedule and spec.restoreFromSnapshot for Horizon nodes.",
                    ));
                }
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
                    if let Some(ref gas) = autoscaling.gas_autoscaling {
                        if gas.enabled {
                            errors.push(SpecValidationError::new(
                                "spec.autoscaling.gasAutoscaling",
                                "gasAutoscaling is only supported for SorobanRPC nodes",
                                "Remove spec.autoscaling.gasAutoscaling or set enabled: false for Horizon nodes; gas-based autoscaling is only supported for SorobanRpc.",
                            ));
                        }
                    }
                }
                if let Some(ingress) = &self.ingress {
                    validate_ingress(ingress, &mut errors);
                }
            }
            NodeType::SorobanRpc => {
                if self.snapshot_schedule.is_some() || self.restore_from_snapshot.is_some() {
                    errors.push(SpecValidationError::new(
                        "spec.snapshotSchedule / spec.restoreFromSnapshot",
                        "snapshot and restore are only supported for Validator nodes",
                        "Remove spec.snapshotSchedule and spec.restoreFromSnapshot for SorobanRpc nodes.",
                    ));
                }
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
                    if let Some(ref gas) = autoscaling.gas_autoscaling {
                        validate_gas_autoscaling(gas, &mut errors);
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
        if let Some(ref mesh) = self.service_mesh {
            validate_service_mesh(mesh, &mut errors);
        }

        // 4. NAT Traversal Validation
        if let Some(nat) = &self.nat_traversal {
            if nat.enabled {
                if nat.stun_server.is_none() && nat.turn_server.is_none() {
                    errors.push(SpecValidationError::new(
                        "spec.natTraversal",
                        "Neither stunServer nor turnServer is provided",
                        "Must provide at least one STUN or TURN server when natTraversal is enabled.",
                    ));
                }
                if nat.turn_server.is_some() && nat.turn_credentials_secret_ref.is_none() {
                    errors.push(SpecValidationError::new(
                        "spec.natTraversal.turnCredentialsSecretRef",
                        "turnCredentialsSecretRef is required when turnServer is provided",
                        "Provide a Secret reference containing 'username' and 'password' keys for the TURN server.",
                    ));
                }
            }
        }

        // 5. Probe override validation
        if let Some(ref probe_config) = self.probes {
            for msg in probe_config.validate() {
                errors.push(SpecValidationError::new(
                    "spec.probes",
                    msg,
                    "Ensure all probe fields are positive integers (initialDelaySeconds >= 0, others >= 1).",
                ));
            }
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
        let separator = if self.version.starts_with("sha256:") {
            "@"
        } else {
            ":"
        };
        format!("stellar/{}{}{}", name, separator, self.version)
    }

    pub fn should_delete_pvc(&self) -> bool {
        self.storage.retention_policy == RetentionPolicy::Delete
    }
}

fn validate_gas_autoscaling(gas: &GasAutoscalingConfig, errors: &mut Vec<SpecValidationError>) {
    if !gas.enabled {
        return;
    }
    if gas.min_replicas > gas.max_replicas {
        errors.push(SpecValidationError::new(
            "spec.autoscaling.gasAutoscaling.minReplicas",
            "gasAutoscaling.minReplicas must be <= maxReplicas",
            "Set spec.autoscaling.gasAutoscaling.minReplicas to a value less than or equal to maxReplicas.",
        ));
    }
    if gas.ewma_alpha <= 0.0 || gas.ewma_alpha >= 1.0 {
        errors.push(SpecValidationError::new(
            "spec.autoscaling.gasAutoscaling.ewmaAlpha",
            "gasAutoscaling.ewmaAlpha must be in range (0.0, 1.0) exclusive",
            "Set spec.autoscaling.gasAutoscaling.ewmaAlpha to a value strictly between 0.0 and 1.0 (e.g. 0.3).",
        ));
    }
    if gas.scale_up_threshold <= gas.scale_down_threshold {
        errors.push(SpecValidationError::new(
            "spec.autoscaling.gasAutoscaling.scaleUpThreshold",
            "gasAutoscaling.scaleUpThreshold must be greater than scaleDownThreshold",
            "Set spec.autoscaling.gasAutoscaling.scaleUpThreshold to a value strictly greater than scaleDownThreshold.",
        ));
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

        if let Some(fed) = &cc.federation {
            if fed.enabled && peer.kubeconfig_secret_ref.as_deref().unwrap_or("").is_empty() {
                errors.push(SpecValidationError::new(
                    format!("spec.crossCluster.peerClusters[{i}].kubeconfigSecretRef"),
                    "crossCluster.peerClusters[].kubeconfigSecretRef is required for federation",
                    "Set spec.crossCluster.peerClusters[].kubeconfigSecretRef when crossCluster.federation.enabled is true.",
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

fn validate_service_mesh(
    mesh: &super::service_mesh::ServiceMeshConfig,
    errors: &mut Vec<SpecValidationError>,
) {
    // Validate that only one of Istio or Linkerd is configured
    if mesh.istio.is_some() && mesh.linkerd.is_some() {
        errors.push(SpecValidationError::new(
            "spec.serviceMesh",
            "Cannot specify both Istio and Linkerd configurations",
            "Choose either spec.serviceMesh.istio or spec.serviceMesh.linkerd, but not both.",
        ));
    }

    // Validate Istio configuration if present
    if let Some(ref istio) = mesh.istio {
        if let Some(ref cb) = istio.circuit_breaker {
            if cb.consecutive_errors == 0 {
                errors.push(SpecValidationError::new(
                    "spec.serviceMesh.istio.circuitBreaker.consecutiveErrors",
                    "consecutiveErrors must be greater than 0",
                    "Set spec.serviceMesh.istio.circuitBreaker.consecutiveErrors to a value greater than 0.",
                ));
            }
            if cb.time_window_secs == 0 {
                errors.push(SpecValidationError::new(
                    "spec.serviceMesh.istio.circuitBreaker.timeWindowSecs",
                    "timeWindowSecs must be greater than 0",
                    "Set spec.serviceMesh.istio.circuitBreaker.timeWindowSecs to a value greater than 0.",
                ));
            }
        }

        if let Some(ref retry) = istio.retries {
            if retry.max_retries == 0 {
                errors.push(SpecValidationError::new(
                    "spec.serviceMesh.istio.retries.maxRetries",
                    "maxRetries must be greater than 0",
                    "Set spec.serviceMesh.istio.retries.maxRetries to a value greater than 0.",
                ));
            }
        }

        if istio.timeout_secs == 0 {
            errors.push(SpecValidationError::new(
                "spec.serviceMesh.istio.timeoutSecs",
                "timeoutSecs must be greater than 0",
                "Set spec.serviceMesh.istio.timeoutSecs to a value greater than 0.",
            ));
        }
    }

    // Validate Linkerd configuration if present
    if let Some(ref linkerd) = mesh.linkerd {
        if !["allow", "deny", "audit"].contains(&linkerd.policy_mode.as_str()) {
            errors.push(SpecValidationError::new(
                "spec.serviceMesh.linkerd.policyMode",
                format!(
                    "policyMode must be one of: allow, deny, audit (got: {})",
                    linkerd.policy_mode
                ),
                "Set spec.serviceMesh.linkerd.policyMode to one of: allow, deny, or audit.",
            ));
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

    /// Timestamp of the last ledger update (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_updated_at: Option<String>,

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

    /// Timestamp when the canary was created (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canary_start_time: Option<String>,

    /// Current traffic weight routed to the canary (0–100)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_weight: Option<i32>,

    /// Observed 4xx/5xx error rate on the canary in the last analysis window (0.0–1.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_error_rate: Option<f64>,

    /// Number of consecutive successful health checks for the current canary
    #[serde(default)]
    pub canary_consecutive_healthy: i32,

    /// Version of the database schema after last successful migration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_migrated_version: Option<String>,

    /// Quorum fragility score (0.0 = resilient, 1.0 = fragile)
    /// Only populated for validator nodes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quorum_fragility: Option<f64>,

    /// Timestamp of last quorum analysis (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quorum_analysis_timestamp: Option<String>,

    /// Last observed Vault secret version annotation (for rotation-driven rollouts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_observed_secret_version: Option<String>,

    /// Phase of the last forensic snapshot request (`Pending`, `Capturing`, `Complete`, `Failed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forensic_snapshot_phase: Option<String>,

    /// Result of the last label propagation pass.
    /// One of: "Synced", "Partial", "Failed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_propagation_status: Option<String>,

    /// Bootstrap status when the node was started from a snapshot or compressed backup.
    /// Tracks the restore phase and time-to-sync for observability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_bootstrap: Option<SnapshotBootstrapStatus>,
    /// Cross-cloud failover status (Horizon/SorobanRpc nodes only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_cloud_failover_status: Option<crate::crd::CrossCloudFailoverStatus>,

    /// Current observed sync state of the Stellar Core node.
    /// One of: `CatchingUp`, `Synced`, `Unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_state: Option<CoreSyncState>,

    /// Whether sync-state-driven resource scaling is currently active and which
    /// profile is applied (`CatchingUp` or `Synced`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_scaling_active_profile: Option<String>,

    /// Status of the history archive pruning process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pruning_status: Option<super::types::PruningStatus>,
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

/// Status of a snapshot-based bootstrap operation.
///
/// Populated when `spec.storage.snapshotRef` or `spec.restoreFromSnapshot` is set.
/// Tracks the restore phase and time-to-sync so operators can verify the
/// "synced within 10 minutes" acceptance criterion.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotBootstrapStatus {
    /// Current phase of the bootstrap operation.
    /// One of: `Pending`, `Restoring`, `Restored`, `Syncing`, `Synced`, `Failed`
    pub phase: String,

    /// Source used for bootstrap (VolumeSnapshot name or backup URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// RFC3339 timestamp when the restore init container started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_started_at: Option<String>,

    /// RFC3339 timestamp when the restore init container completed successfully.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_completed_at: Option<String>,

    /// RFC3339 timestamp when the node first reached `Synced` state after bootstrap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<String>,

    /// Elapsed seconds from restore completion to first `Synced` state.
    /// A value ≤ 600 satisfies the "synced within 10 minutes" acceptance criterion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds_to_sync: Option<u64>,

    /// Human-readable message about the current bootstrap state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
            validator_config: Some(ValidatorConfig {
                seed_secret_ref: "test".to_string(),
                ..Default::default()
            }),
            strategy: RolloutStrategy {
                strategy_type: crate::crd::types::RolloutStrategyType::Canary,
                canary: Some(CanaryConfig {
                    weight: 10,
                    check_interval_seconds: 300,
                    ..Default::default()
                }),
            },
            ..Default::default()
        };

        assert!(spec.validate().is_err());
    }

    #[test] // Ensure this attribute is there
    fn test_horizon_with_canary_should_pass() {
        let spec = StellarNodeSpec {
            node_type: NodeType::Horizon,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            horizon_config: Some(HorizonConfig {
                database_secret_ref: "test".to_string(),
                enable_ingest: true,
                stellar_core_url: "http://core".to_string(),
                ..Default::default()
            }),
            replicas: 3,
            strategy: RolloutStrategy {
                strategy_type: crate::crd::types::RolloutStrategyType::Canary,
                canary: Some(CanaryConfig {
                    weight: 20,
                    check_interval_seconds: 300,
                    ..Default::default()
                }),
            },
            ..Default::default()
        };

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_container_image_formats() {
        // 1. Standard tag
        let mut spec = StellarNodeSpec {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            replicas: 1,
            ..Default::default()
        };
        assert_eq!(spec.container_image(), "stellar/stellar-core:v21.0.0");

        // 2. Pure digest
        spec.version =
            "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678".to_string();
        assert_eq!(
            spec.container_image(),
            "stellar/stellar-core@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678"
        );

        // 3. Tag with digest
        spec.version =
            "v21.0.0@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678"
                .to_string();
        assert_eq!(
            spec.container_image(),
            "stellar/stellar-core:v21.0.0@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678"
        );

        // 4. Horizon node
        spec.node_type = NodeType::Horizon;
        spec.version = "v2.10.0".to_string();
        assert_eq!(spec.container_image(), "stellar/horizon:v2.10.0");
    }
}
