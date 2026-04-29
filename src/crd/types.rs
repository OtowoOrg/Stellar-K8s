//! Shared types for Stellar node specifications
//!
//! These types are used across the CRD definitions and controller logic.
//! They define the configuration for different Stellar node types, resource requirements,
//! storage policies, and advanced features like autoscaling, ingress, and network policies.
//!
//! # Type Hierarchy
//!
//! - [`NodeType`] - Specifies the type of Stellar infrastructure (Validator, Horizon, SorobanRpc)
//! - [`StellarNetwork`] - Target Stellar network (Mainnet, Testnet, Futurenet, or Custom)
//! - [`ResourceRequirements`] - CPU and memory requests/limits following Kubernetes conventions
//! - [`StorageConfig`] - Persistent storage configuration with retention policies
//! - Node-specific configs: [`ValidatorConfig`], [`HorizonConfig`], [`SorobanConfig`]
//! - Advanced features: [`AutoscalingConfig`], [`IngressConfig`], [`NetworkPolicyConfig`]

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Supported Stellar node types
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum NodeType {
    /// Full validator node running Stellar Core
    /// Participates in consensus and validates transactions
    #[default]
    Validator,

    /// Horizon API server for REST access to the Stellar network
    /// Provides a RESTful API for querying the Stellar ledger
    Horizon,

    /// Soroban RPC node for smart contract interactions
    /// Handles Soroban smart contract simulation and submission
    SorobanRpc,
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeType::Validator => write!(f, "Validator"),
            NodeType::Horizon => write!(f, "Horizon"),
            NodeType::SorobanRpc => write!(f, "SorobanRpc"),
        }
    }
}

/// History mode for the node
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum HistoryMode {
    /// Full history node (VSL compatible, archive)
    Full,
    /// Recent history only (lighter, faster sync)
    #[default]
    Recent,
}

impl std::fmt::Display for HistoryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryMode::Full => write!(f, "Full"),
            HistoryMode::Recent => write!(f, "Recent"),
        }
    }
}

/// Target Stellar network
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StellarNetwork {
    Mainnet,
    #[default]
    Testnet,
    Futurenet,
    Custom(String),
}

impl StellarNetwork {
    pub fn passphrase<'a>(&'a self, custom: &'a Option<String>) -> &'a str {
        match self {
            StellarNetwork::Mainnet => "Public Global Stellar Network ; September 2015",
            StellarNetwork::Testnet => "Test SDF Network ; September 2015",
            StellarNetwork::Futurenet => "Test SDF Future Network ; October 2022",
            StellarNetwork::Custom(_) => custom.as_deref().unwrap_or(""),
        }
    }

    /// Validate the custom network name against DNS-1123 label rules.
    ///
    /// Rules (applied only to `Custom` variants):
    /// - Must not be empty (minLength: 1)
    /// - Must not exceed 63 characters (maxLength: 63)
    /// - Must match `^[a-z0-9]([-a-z0-9]*[a-z0-9])?$` (lowercase alphanumeric and hyphens,
    ///   no leading/trailing hyphens)
    pub fn validate_custom_name(&self) -> Result<(), String> {
        let name = match self {
            StellarNetwork::Custom(n) => n,
            _ => return Ok(()),
        };
        if name.is_empty() {
            return Err("customName must not be empty (minLength: 1)".to_string());
        }
        if name.len() > 63 {
            return Err(format!(
                "customName '{name}' exceeds 63 characters (maxLength: 63)"
            ));
        }
        let valid = name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && name.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !valid {
            return Err(format!(
                "customName '{name}' is invalid: must match ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ (lowercase alphanumeric and hyphens only, no leading/trailing hyphens)"
            ));
        }
        Ok(())
    }

    /// Stable, DNS-1123-friendly label value for topology spread and anti-affinity.
    pub fn scheduling_label_value(&self, _custom: &Option<String>) -> String {
        match self {
            StellarNetwork::Mainnet => "mainnet".to_string(),
            StellarNetwork::Testnet => "testnet".to_string(),
            StellarNetwork::Futurenet => "futurenet".to_string(),
            StellarNetwork::Custom(name) => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                name.hash(&mut h);
                format!("custom-{:x}", h.finish())
            }
        }
    }
}

/// Controls default pod anti-affinity for spreading pods that share the same
/// [`StellarNetwork`] across nodes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum PodAntiAffinityStrength {
    /// `requiredDuringScheduling` — do not place on a node that already runs a matching pod.
    #[default]
    Hard,
    /// `preferredDuringScheduling` — best-effort separation with weight 100.
    Soft,
    /// Do not inject pod anti-affinity (topology spread defaults still apply unless overridden).
    Disabled,
}

/// Kubernetes-style resource requirements
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequirements {
    pub requests: ResourceSpec,
    pub limits: ResourceSpec,
}

impl Default for ResourceRequirements {
    fn default() -> Self {
        Self {
            requests: ResourceSpec {
                cpu: "500m".to_string(),
                memory: "1Gi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "2".to_string(),
                memory: "4Gi".to_string(),
            },
        }
    }
}

/// Resource specification for CPU and memory
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct ResourceSpec {
    pub cpu: String,
    pub memory: String,
}

impl Default for ResourceSpec {
    fn default() -> Self {
        Self {
            cpu: "500m".to_string(),
            memory: "1Gi".to_string(),
        }
    }
}

/// Storage mode for persistent data
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum StorageMode {
    #[default]
    PersistentVolume,
    Local,
}

/// Reference to a pre-computed snapshot used to bootstrap a new node.
///
/// Supports two bootstrap mechanisms:
/// - **CSI VolumeSnapshot**: A Kubernetes `VolumeSnapshot` object (snapshot.storage.k8s.io/v1)
///   that the operator uses as the PVC `dataSource` for near-instant volume cloning.
/// - **Compressed backup**: A `.tar.gz` (or `.tar.zst`) archive stored in an S3-compatible
///   bucket or a Kubernetes PVC. The operator injects an init container that downloads and
///   extracts the archive into the data volume before Stellar Core starts.
///
/// Only one of `volume_snapshot_name` or `backup_url` should be set.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRef {
    /// Name of an existing `VolumeSnapshot` (snapshot.storage.k8s.io/v1) in the same namespace.
    /// When set, the PVC is provisioned from this snapshot — no init container is needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_snapshot_name: Option<String>,

    /// Optional namespace of the VolumeSnapshot when it lives in a different namespace.
    /// Requires `CrossNamespaceVolumeDataSource` feature gate on the cluster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_snapshot_namespace: Option<String>,

    /// URL of a compressed DB backup archive (`.tar.gz` or `.tar.zst`).
    ///
    /// Supported schemes:
    /// - `s3://bucket/path/to/backup.tar.gz`
    /// - `https://host/path/to/backup.tar.gz`
    ///
    /// The operator injects an init container (`snapshot-restore`) that downloads and
    /// extracts the archive into `/data` before Stellar Core starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_url: Option<String>,

    /// Name of a Kubernetes Secret containing credentials for the backup URL.
    ///
    /// For S3 URLs the secret must have keys `AWS_ACCESS_KEY_ID` and
    /// `AWS_SECRET_ACCESS_KEY` (and optionally `AWS_DEFAULT_REGION`).
    /// For HTTPS URLs the secret may have a `BEARER_TOKEN` key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret_ref: Option<String>,

    /// Container image used for the restore init container.
    /// Must have `aws` CLI (for S3) or `curl`/`wget` plus `tar` available.
    /// Defaults to `amazon/aws-cli:latest` for S3 URLs and `alpine:3` for HTTPS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_image: Option<String>,
}

/// Storage configuration for persistent data
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    #[serde(default)]
    pub mode: StorageMode,
    pub storage_class: String,
    pub size: String,
    #[serde(default)]
    pub retention_policy: RetentionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
    /// Node affinity for local storage mode (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "super::schema_utils::object_schema")]
    pub node_affinity: Option<k8s_openapi::api::core::v1::NodeAffinity>,

    /// Bootstrap this node from a pre-computed snapshot or compressed DB backup.
    ///
    /// When set, the operator either:
    /// - Provisions the PVC from a CSI `VolumeSnapshot` (zero-copy, near-instant), or
    /// - Injects a `snapshot-restore` init container that downloads and extracts a
    ///   compressed archive before Stellar Core starts.
    ///
    /// This reduces catch-up time from days to minutes for new validator nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<SnapshotRef>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            mode: StorageMode::default(),
            storage_class: "standard".to_string(),
            size: "100Gi".to_string(),
            retention_policy: RetentionPolicy::default(),
            annotations: None,
            node_affinity: None,
            snapshot_ref: None,
        }
    }
}

/// Probe configuration for liveness, readiness, and startup probes.
///
/// All fields are optional; unset fields fall back to the operator's built-in defaults.
/// Values must be positive integers where applicable.
///
/// # Example
/// ```yaml
/// probes:
///   liveness:
///     initialDelaySeconds: 30
///     periodSeconds: 10
///     failureThreshold: 3
///   readiness:
///     initialDelaySeconds: 10
///     periodSeconds: 5
///   startup:
///     failureThreshold: 30
///     periodSeconds: 10
/// ```
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOverride {
    /// Number of seconds after the container starts before the probe is initiated. Min: 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_delay_seconds: Option<i32>,
    /// How often (in seconds) to perform the probe. Min: 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_seconds: Option<i32>,
    /// Number of seconds after which the probe times out. Min: 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<i32>,
    /// Minimum consecutive successes for the probe to be considered successful. Min: 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_threshold: Option<i32>,
    /// Minimum consecutive failures for the probe to be considered failed. Min: 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_threshold: Option<i32>,
}

impl ProbeOverride {
    /// Validate that all set fields are within acceptable ranges.
    pub fn validate(&self, field_prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(v) = self.initial_delay_seconds {
            if v < 0 {
                errors.push(format!(
                    "{field_prefix}.initialDelaySeconds must be >= 0, got {v}"
                ));
            }
        }
        for (name, val) in [
            ("periodSeconds", self.period_seconds),
            ("timeoutSeconds", self.timeout_seconds),
            ("successThreshold", self.success_threshold),
            ("failureThreshold", self.failure_threshold),
        ] {
            if let Some(v) = val {
                if v < 1 {
                    errors.push(format!("{field_prefix}.{name} must be >= 1, got {v}"));
                }
            }
        }
        errors
    }
}

/// Per-container probe overrides for liveness, readiness, and startup probes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeConfig {
    /// Override for the liveness probe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liveness: Option<ProbeOverride>,
    /// Override for the readiness probe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readiness: Option<ProbeOverride>,
    /// Override for the startup probe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup: Option<ProbeOverride>,
}

impl ProbeConfig {
    /// Validate all probe overrides. Returns a list of error strings.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(p) = &self.liveness {
            errors.extend(p.validate("spec.probes.liveness"));
        }
        if let Some(p) = &self.readiness {
            errors.extend(p.validate("spec.probes.readiness"));
        }
        if let Some(p) = &self.startup {
            errors.extend(p.validate("spec.probes.startup"));
        }
        errors
    }
}

/// PVC retention policy on node deletion
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum RetentionPolicy {
    #[default]
    Delete,
    Retain,
}

// ============================================================================
// cert-manager integration
// ============================================================================

/// Reference to a cert-manager Issuer or ClusterIssuer.
///
/// When set on a `StellarNode`, the operator will create a cert-manager
/// `Certificate` resource for the node instead of issuing a self-signed
/// certificate with `rcgen`. cert-manager then manages rotation automatically.
///
/// # Example
/// ```yaml
/// certManager:
///   issuerRef:
///     name: letsencrypt-prod
///     kind: ClusterIssuer
///   duration: "2160h"   # 90 days
///   renewBefore: "720h" # 30 days
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CertManagerConfig {
    /// Reference to the Issuer or ClusterIssuer that will sign the certificate.
    pub issuer_ref: CertManagerIssuerRef,

    /// Requested certificate duration (e.g. `"2160h"` for 90 days).
    /// Defaults to cert-manager's default (90 days) when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,

    /// How long before expiry cert-manager should renew the certificate
    /// (e.g. `"720h"` for 30 days). Defaults to cert-manager's default when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renew_before: Option<String>,
}

/// Reference to a cert-manager issuer resource.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CertManagerIssuerRef {
    /// Name of the Issuer or ClusterIssuer resource.
    pub name: String,

    /// Kind of the issuer: `"Issuer"` (namespace-scoped) or `"ClusterIssuer"` (cluster-scoped).
    /// Defaults to `"Issuer"` when omitted.
    #[serde(default = "default_issuer_kind")]
    pub kind: String,

    /// API group of the issuer. Defaults to `"cert-manager.io"`.
    #[serde(default = "default_issuer_group")]
    pub group: String,
}

fn default_issuer_kind() -> String {
    "Issuer".to_string()
}

fn default_issuer_group() -> String {
    "cert-manager.io".to_string()
}

/// Configuration for zero-downtime CSI VolumeSnapshot scheduling
///
/// When set, the operator will create Kubernetes VolumeSnapshot resources targeting
/// the node's data PVC on the given schedule (or on-demand via annotation).
/// For database consistency, the operator can optionally trigger a brief flush/lock
/// before taking the snapshot when the storage driver does not guarantee crash consistency.
///
/// Only applies to Validator nodes (Stellar Core ledger data).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotScheduleConfig {
    /// Cron expression for scheduled snapshots (e.g. "0 2 * * *" for daily at 2 AM).
    /// If unset, snapshots are only taken when triggered via annotation `stellar.org/request-snapshot: "true"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// VolumeSnapshotClass name. If unset, the default class for the PVC's driver is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_snapshot_class_name: Option<String>,
    /// If true, the operator will attempt to flush/lock the Stellar database briefly before creating the snapshot (e.g. via stellar-core HTTP or exec). Requires the node to be healthy.
    #[serde(default)]
    pub flush_before_snapshot: bool,
    /// Maximum number of snapshots to retain per node. Oldest snapshots are deleted when exceeded. 0 means no limit.
    #[serde(default)]
    pub retention_count: u32,
    /// Reference to a Cloud KMS key for encrypting the snapshot (e.g. AWS KMS ARN, GCP KMS Key Name).
    /// If provided, the operator will ensure the snapshot is encrypted using this key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_key_ref: Option<String>,
}

/// Configuration to bootstrap a new node from an existing CSI VolumeSnapshot
///
/// When set, the node's PVC is created from the specified VolumeSnapshot instead of
/// starting empty, enabling near-instant bootstrap without syncing from a history archive.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreFromSnapshotConfig {
    /// Name of the VolumeSnapshot to restore from (must exist in the same namespace as the StellarNode).
    pub volume_snapshot_name: String,
    /// Optional: namespace of the VolumeSnapshot if different from the StellarNode. Requires CrossNamespaceVolumeDataSource where supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// VPA update mode
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum VpaUpdateMode {
    #[default]
    Initial,
    Auto,
}

/// Per-container resource policy for the VPA
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaContainerPolicy {
    pub container_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_allowed: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_allowed: Option<std::collections::BTreeMap<String, String>>,
}

/// VPA configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaConfig {
    #[serde(default)]
    pub update_mode: VpaUpdateMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub container_policies: Vec<VpaContainerPolicy>,
}

/// Configuration for the durable log-to-S3 sidecar.
///
/// When set, the operator injects a `stellar-log-shipper` sidecar into every
/// managed pod.  The sidecar tails `/var/log/stellar/`, batches lines into
/// gzip-compressed chunks, and uploads them to S3 on a rolling schedule.
///
/// # Example
/// ```yaml
/// logShipper:
///   enabled: true
///   s3Bucket: "my-stellar-logs"
///   s3Prefix: "validators/mainnet"
///   credentialsSecretRef: "aws-log-shipper-creds"
///   batchSizeLines: 5000
///   flushIntervalSecs: 60
///   retentionDays: 90
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogShipperConfig {
    /// Enable the log-shipper sidecar.
    #[serde(default)]
    pub enabled: bool,

    /// S3 bucket name for log archives.
    pub s3_bucket: String,

    /// Optional key prefix inside the bucket (e.g. `"validators/mainnet"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_prefix: Option<String>,

    /// AWS region for the S3 bucket (e.g. `"us-east-1"`).
    /// Falls back to `AWS_DEFAULT_REGION` env var if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_region: Option<String>,

    /// Name of a Kubernetes Secret in the same namespace containing
    /// `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.
    /// Omit when using IRSA / instance profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_secret_ref: Option<String>,

    /// Flush a new gzip batch after this many log lines (default: 5000).
    #[serde(default = "default_batch_size_lines")]
    pub batch_size_lines: u32,

    /// Flush a new gzip batch after this many seconds even if
    /// `batch_size_lines` has not been reached (default: 60).
    #[serde(default = "default_flush_interval_secs")]
    pub flush_interval_secs: u64,

    /// Delete S3 objects older than this many days (0 = keep forever).
    /// Implemented via S3 lifecycle rules applied at startup.
    #[serde(default)]
    pub retention_days: u32,

    /// Container image for the log-shipper sidecar.
    /// Defaults to the same image as the operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

fn default_batch_size_lines() -> u32 {
    5000
}
fn default_flush_interval_secs() -> u64 {
    60
}
/// Observed sync state of a Stellar Core node, derived from the `/info` HTTP endpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CoreSyncState {
    /// Node is actively catching up on historical ledgers (compute-intensive).
    CatchingUp,
    /// Node is fully synced with the network (steady-state, lower resource needs).
    #[default]
    Synced,
    /// State could not be determined (pod not ready, endpoint unreachable, etc.).
    Unknown,
}

impl std::fmt::Display for CoreSyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreSyncState::CatchingUp => write!(f, "CatchingUp"),
            CoreSyncState::Synced => write!(f, "Synced"),
            CoreSyncState::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Resource profile applied during a specific sync phase.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncPhaseResources {
    /// CPU request (e.g. "2", "4000m")
    pub cpu_request: String,
    /// Memory request (e.g. "4Gi", "8Gi")
    pub memory_request: String,
    /// CPU limit (e.g. "4", "8000m")
    pub cpu_limit: String,
    /// Memory limit (e.g. "8Gi", "16Gi")
    pub memory_limit: String,
}

/// Dynamic resource scaling based on Stellar Core sync state.
///
/// When enabled, the operator monitors the `/info` endpoint on port 11626 and
/// applies `catching_up` resources while the node is catching up, then switches
/// to `synced` resources once the node reports `"Synced!"`.  The update is done
/// via an in-place `PATCH` on the pod's container resources (requires the
/// `InPlacePodVerticalScaling` feature gate, available since Kubernetes 1.27).
///
/// # Example
/// ```yaml
/// syncStateScaling:
///   enabled: true
///   catchingUp:
///     cpuRequest: "4"
///     memoryRequest: "8Gi"
///     cpuLimit: "8"
///     memoryLimit: "16Gi"
///   synced:
///     cpuRequest: "500m"
///     memoryRequest: "2Gi"
///     cpuLimit: "2"
///     memoryLimit: "4Gi"
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStateScalingConfig {
    /// Enable or disable sync-state-driven resource scaling.
    #[serde(default)]
    pub enabled: bool,

    /// Resources to apply while the node is in `CatchingUp` state.
    pub catching_up: SyncPhaseResources,

    /// Resources to apply once the node reaches `Synced` state.
    pub synced: SyncPhaseResources,

    /// How often (in seconds) to poll the stellar-core `/info` endpoint.
    /// Defaults to 30 seconds.
    #[serde(default = "default_sync_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_sync_poll_interval_secs() -> u64 {
    30
}

/// Forensic snapshot bundle upload (S3-compatible via AWS CLI in ephemeral capture).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForensicSnapshotConfig {
    /// Target S3 bucket for the encrypted forensic tarball.
    pub s3_bucket: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_prefix: Option<String>,

    /// Optional KMS key id for SSE-KMS (`aws s3 cp --sse aws:kms`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,

    /// Secret in the same namespace with `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`
    /// when not using IRSA/instance roles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_secret_ref: Option<String>,

    /// Set `shareProcessNamespace: true` on validator pods so the capture container
    /// can see `stellar-core` for core dumps (recommended for forensic workflows).
    #[serde(default)]
    pub enable_share_process_namespace: bool,
}

/// Validator-specific configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorConfig {
    /// Secret name containing the validator seed (key: STELLAR_CORE_SEED)
    /// DEPRECATED: Use seed_secret_source for KMS/ESO/CSI-backed secrets in production
    #[serde(default)]
    pub seed_secret_ref: String,

    // -------------------------------------------------------------------------
    // NEW FIELD: KMS / External Secrets Operator / CSI secret source
    // When set, this takes precedence over seed_secret_ref.
    // -------------------------------------------------------------------------
    /// Production seed source: ESO (AWS SM / GCP SM / Vault) or CSI Secret Store Driver.
    /// Takes precedence over seed_secret_ref when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_secret_source: Option<crate::crd::seed_secret::SeedSecretSource>,

    /// Quorum set configuration as TOML string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quorum_set: Option<String>,
    /// Known peers configuration as TOML string (KNOWN_PEERS)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_peers: Option<String>,
    /// Enable history archive for this validator
    #[serde(default)]
    pub enable_history_archive: bool,
    /// History archive URLs to fetch from
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_archive_urls: Vec<String>,
    /// Node is in catchup mode (syncing historical data)
    #[serde(default)]
    pub catchup_complete: bool,
    /// Source of the validator seed (Secret or KMS)
    #[serde(default)]
    pub key_source: KeySource,
    /// KMS configuration for fetching the validator seed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_config: Option<KmsConfig>,
    /// Trusted source for Validator Selection List (VSL)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vl_source: Option<String>,
    /// Quorum set optimization configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quorum_optimization: Option<QuorumOptimizationConfig>,
    /// Cloud HSM configuration for secure key loading (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hsm_config: Option<HsmConfig>,
    /// ExternalDNS configuration for automated peer discovery
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_dns: Option<ExternalDNSConfig>,
}

/// Quorum set optimization configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuorumOptimizationConfig {
    /// Enable automated quorum set optimization
    #[serde(default)]
    pub enabled: bool,
    /// Optimization mode (Manual or Auto)
    #[serde(default)]
    pub mode: QuorumOptimizationMode,
    /// Interval in seconds for optimization analysis (default: 3600s / 1h)
    #[serde(default = "default_optimization_interval")]
    pub interval_secs: u32,
    /// Maximum RTT threshold in ms for considering a peer "slow" (default: 500ms)
    #[serde(default = "default_rtt_threshold")]
    pub rtt_threshold_ms: u32,
}

fn default_optimization_interval() -> u32 {
    3600
}

fn default_rtt_threshold() -> u32 {
    500
}

/// Quorum optimization mode
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum QuorumOptimizationMode {
    /// Suggest updates only (emits events and updates status)
    #[default]
    Manual,
    /// Automatically apply recommended updates to the CRD
    Auto,
}

// =============================================================================
// NEW: impl block for ValidatorConfig
// =============================================================================
impl ValidatorConfig {
    /// Return the effective seed source.
    ///
    /// Precedence: `seed_secret_source` (new, KMS/ESO/CSI) → `seed_secret_ref` (legacy).
    /// Returns `None` only when neither field is set.
    pub fn resolve_seed_source(&self) -> Option<crate::crd::seed_secret::SeedSecretSource> {
        // Prefer the new typed field
        if let Some(ref src) = self.seed_secret_source {
            return Some(src.clone());
        }
        // Fall back to the legacy plain-Secret ref
        if !self.seed_secret_ref.is_empty() {
            return Some(crate::crd::seed_secret::SeedSecretSource {
                local_ref: Some(crate::crd::seed_secret::LocalSecretRef {
                    name: self.seed_secret_ref.clone(),
                    key: None,
                }),
                external_ref: None,
                csi_ref: None,
                vault_ref: None,
            });
        }
        None
    }
}

/// Configuration for Hardware Security Module (HSM) integration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HsmConfig {
    pub provider: HsmProvider,
    pub pkcs11_lib_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hsm_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hsm_credentials_secret_ref: Option<String>,
}

/// Supported HSM Providers
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum HsmProvider {
    #[default]
    AWS,
    Azure,
}

/// Source of security keys
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    #[default]
    Secret,
    KMS,
}

/// Configuration for cloud-native KMS or Vault
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KmsConfig {
    pub key_id: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetcher_image: Option<String>,
}

/// Horizon API server configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HorizonConfig {
    pub database_secret_ref: String,
    #[serde(default = "default_true")]
    pub enable_ingest: bool,
    pub stellar_core_url: String,
    #[serde(default = "default_ingest_workers")]
    pub ingest_workers: u32,
    #[serde(default)]
    pub enable_experimental_ingestion: bool,
    #[serde(default = "default_true")]
    pub auto_migration: bool,
}

fn default_true() -> bool {
    true
}

fn default_ingest_workers() -> u32 {
    1
}

/// Captive Core configuration for Soroban RPC
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptiveCoreConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_passphrase: Option<String>,
    #[serde(default)]
    pub history_archive_urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_config: Option<String>,
}

/// Soroban RPC server configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SorobanConfig {
    pub stellar_core_url: String,
    #[deprecated(
        since = "0.2.0",
        note = "Use captive_core_structured_config for type-safe configuration"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captive_core_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captive_core_structured_config: Option<CaptiveCoreConfig>,
    #[serde(default = "default_true")]
    pub enable_preflight: bool,
    #[serde(default = "default_max_events")]
    pub max_events_per_request: u32,
    /// Multi-layered cache configuration (L1 in-memory LRU + L2 local-SSD).
    /// When set, the operator provisions an emptyDir volume and injects cache
    /// path / size env vars into the Soroban RPC container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_config: Option<crate::controller::soroban_cache::SorobanCacheConfig>,
}

/// External database configuration for managed Postgres databases
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDatabaseConfig {
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub user: String,
    pub password_secret: String,
    pub secret_key_ref: Option<SecretKeyRef>,
}

/// Reference to a key within a Kubernetes Secret
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub name: String,
    pub key: String,
}

/// Ingress configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    pub hosts: Vec<IngressHost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_secret_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_manager_issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_manager_cluster_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
    /// ExternalDNS configuration for automated record management
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_dns: Option<ExternalDNSConfig>,
}

/// Ingress host entry
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressHost {
    pub host: String,
    #[serde(
        default = "default_ingress_paths",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub paths: Vec<IngressPath>,
}

/// Ingress path mapping
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressPath {
    pub path: String,
    #[serde(default = "default_path_type")]
    pub path_type: Option<String>,
}

fn default_ingress_paths() -> Vec<IngressPath> {
    vec![IngressPath {
        path: "/".to_string(),
        path_type: default_path_type(),
    }]
}

fn default_path_type() -> Option<String> {
    Some("Prefix".to_string())
}

fn default_max_events() -> u32 {
    10000
}

/// Horizontal Pod Autoscaling configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingConfig {
    pub min_replicas: i32,
    pub max_replicas: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_cpu_utilization_percentage: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_metrics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<ScalingBehavior>,
    /// Predictive scaling configuration.
    ///
    /// When enabled, the operator uses a Holt-Winters forecasting model to
    /// predict the next hour's ledger volume and pre-emptively adjusts
    /// `minReplicas` before traffic spikes occur.
    ///
    /// Only applicable to `Horizon` nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predictive_scaling: Option<crate::controller::predictive_scaling::PredictiveScalingConfig>,
    /// Gas-consumption-driven autoscaling configuration. Only valid for SorobanRPC nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_autoscaling: Option<GasAutoscalingConfig>,
}

/// eBPF-based proactive failure detection configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EbpfConfig {
    /// Enable the eBPF exporter sidecar
    #[serde(default)]
    pub enabled: bool,
    /// Monitor write() latency to the ledger DB
    #[serde(default = "default_true")]
    pub monitor_write_latency: bool,
    /// Track TCP retransmits and handshake times for peer connections
    #[serde(default = "default_true")]
    pub monitor_tcp_retransmits: bool,
}

/// Scaling behavior configuration for HPA
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScalingBehavior {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_up: Option<ScalingPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_down: Option<ScalingPolicy>,
}

/// Scaling policy
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScalingPolicy {
    pub stabilization_window_seconds: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<HPAPolicy>,
}

/// Individual HPA policy
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HPAPolicy {
    pub policy_type: String,
    pub value: i32,
    pub period_seconds: i32,
}

/// Condition for status reporting
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    pub type_: String,
    pub status: String,
    pub last_transition_time: String,
    pub reason: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}

impl Condition {
    pub fn ready(status: bool, reason: &str, message: &str) -> Self {
        Self {
            type_: "Ready".to_string(),
            status: if status { "True" } else { "False" }.to_string(),
            last_transition_time: chrono::Utc::now().to_rfc3339(),
            reason: reason.to_string(),
            message: message.to_string(),
            observed_generation: None,
        }
    }

    pub fn progressing(reason: &str, message: &str) -> Self {
        Self {
            type_: "Progressing".to_string(),
            status: "True".to_string(),
            last_transition_time: chrono::Utc::now().to_rfc3339(),
            reason: reason.to_string(),
            message: message.to_string(),
            observed_generation: None,
        }
    }

    pub fn degraded(reason: &str, message: &str) -> Self {
        Self {
            type_: "Degraded".to_string(),
            status: "True".to_string(),
            last_transition_time: chrono::Utc::now().to_rfc3339(),
            reason: reason.to_string(),
            message: message.to_string(),
            observed_generation: None,
        }
    }

    pub fn with_observed_generation(mut self, generation: i64) -> Self {
        self.observed_generation = Some(generation);
        self
    }
}

/// Network Policy configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_namespaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_pod_selector: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_cidrs: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_metrics_scrape: bool,
    #[serde(default = "default_monitoring_namespace")]
    pub metrics_namespace: String,
}

fn default_monitoring_namespace() -> String {
    "monitoring".to_string()
}

impl Default for NetworkPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_namespaces: Vec::new(),
            allow_pod_selector: None,
            allow_cidrs: Vec::new(),
            allow_metrics_scrape: true,
            metrics_namespace: default_monitoring_namespace(),
        }
    }
}

/// Rollout strategy type
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RolloutStrategyType {
    #[default]
    RollingUpdate,
    Canary,
    BlueGreen,
}

/// Rollout strategy for updates
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RolloutStrategy {
    #[serde(rename = "type")]
    pub strategy_type: RolloutStrategyType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canary: Option<CanaryConfig>,
}

impl RolloutStrategy {
    pub fn canary(&self) -> Option<&CanaryConfig> {
        if let RolloutStrategyType::Canary = self.strategy_type {
            self.canary.as_ref()
        } else {
            None
        }
    }
}

/// Configuration for Canary rollout
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanaryConfig {
    /// Initial traffic weight sent to the canary (0–100). Default: 10
    #[serde(default = "default_canary_weight")]
    pub weight: i32,

    /// How often (seconds) to evaluate canary health before promoting or rolling back.
    /// Default: 300 (5 minutes)
    #[serde(default = "default_canary_interval")]
    pub check_interval_seconds: i32,

    /// Maximum 4xx/5xx error rate (0.0–1.0) allowed before triggering automatic rollback.
    /// E.g. 0.05 = 5%. Default: 0.05
    #[serde(default = "default_max_error_rate")]
    pub max_error_rate: f64,

    /// When set, the operator progressively increases canary weight by this amount
    /// each check interval until `max_weight` is reached. Default: 0 (no stepping)
    #[serde(default)]
    pub step_weight: i32,

    /// Maximum weight the canary may reach during progressive rollout. Default: 50
    #[serde(default = "default_max_weight")]
    pub max_weight: i32,

    /// Number of consecutive healthy checks required before promoting. Default: 1
    #[serde(default = "default_canary_success_threshold")]
    pub success_threshold: i32,
}

fn default_canary_weight() -> i32 {
    10
}

fn default_canary_interval() -> i32 {
    300
}

fn default_max_error_rate() -> f64 {
    0.05
}

fn default_max_weight() -> i32 {
    50
}

fn default_canary_success_threshold() -> i32 {
    1
}

/// Load Balancer configuration for external access via MetalLB
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: LoadBalancerMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_pool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_balancer_ip: Option<String>,
    #[serde(default)]
    pub external_traffic_policy: ExternalTrafficPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgp: Option<BGPConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
    #[serde(default = "default_true")]
    pub health_check_enabled: bool,
    #[serde(default = "default_health_check_port")]
    pub health_check_port: i32,
    /// ExternalDNS configuration for the LoadBalancer service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_dns: Option<ExternalDNSConfig>,
}

fn default_health_check_port() -> i32 {
    9100
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: LoadBalancerMode::default(),
            address_pool: None,
            load_balancer_ip: None,
            external_traffic_policy: ExternalTrafficPolicy::default(),
            bgp: None,
            annotations: None,
            health_check_enabled: true,
            health_check_port: default_health_check_port(),
            external_dns: None,
        }
    }
}

/// Load balancer mode selection
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum LoadBalancerMode {
    #[default]
    L2,
    BGP,
}

impl std::fmt::Display for LoadBalancerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadBalancerMode::L2 => write!(f, "L2"),
            LoadBalancerMode::BGP => write!(f, "BGP"),
        }
    }
}

/// External traffic policy for LoadBalancer services
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum ExternalTrafficPolicy {
    #[default]
    Cluster,
    Local,
}

impl std::fmt::Display for ExternalTrafficPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalTrafficPolicy::Cluster => write!(f, "Cluster"),
            ExternalTrafficPolicy::Local => write!(f, "Local"),
        }
    }
}

/// BGP configuration for MetalLB anycast routing
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPConfig {
    pub local_asn: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<BGPPeer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub communities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub large_communities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advertisement: Option<BGPAdvertisementConfig>,
    #[serde(default)]
    pub bfd_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bfd_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_selectors: Option<BTreeMap<String, String>>,
}

/// BGP peer router configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPPeer {
    pub address: String,
    pub asn: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_secret_ref: Option<SecretKeyRef>,
    #[serde(default = "default_bgp_port")]
    pub port: u16,
    #[serde(default = "default_hold_time")]
    pub hold_time: u32,
    #[serde(default = "default_keepalive_time")]
    pub keepalive_time: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_address: Option<String>,
    #[serde(default)]
    pub ebgp_multi_hop: bool,
    #[serde(default = "default_true")]
    pub graceful_restart: bool,
}

fn default_bgp_port() -> u16 {
    179
}

fn default_hold_time() -> u32 {
    90
}

fn default_keepalive_time() -> u32 {
    30
}

/// BGP advertisement configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPAdvertisementConfig {
    #[serde(default = "default_aggregation_length")]
    pub aggregation_length: u8,
    #[serde(default = "default_aggregation_length_v6")]
    pub aggregation_length_v6: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_pref: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_selectors: Option<BTreeMap<String, String>>,
}

fn default_aggregation_length() -> u8 {
    32
}

fn default_aggregation_length_v6() -> u8 {
    128
}

/// Global node discovery configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GlobalDiscoveryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default)]
    pub topology_aware_hints: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_mesh: Option<ServiceMeshConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_dns: Option<ExternalDNSConfig>,
}

fn default_priority() -> u32 {
    100
}

impl Default for GlobalDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            region: None,
            zone: None,
            priority: default_priority(),
            topology_aware_hints: false,
            service_mesh: None,
            external_dns: None,
        }
    }
}

/// Service mesh integration configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMeshConfig {
    pub mesh_type: ServiceMeshType,
    #[serde(default = "default_true")]
    pub sidecar_injection: bool,
    #[serde(default)]
    pub mtls_mode: MTLSMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_service_host: Option<String>,
}

/// Supported service mesh implementations
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceMeshType {
    Istio,
    Linkerd,
    Consul,
}

/// mTLS enforcement mode
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum MTLSMode {
    Disable,
    #[default]
    Permissive,
    Strict,
}

/// ExternalDNS configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDNSConfig {
    pub hostname: String,
    #[serde(default = "default_dns_ttl")]
    pub ttl: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

fn default_dns_ttl() -> u32 {
    300
}

/// Configuration for multi-cluster disaster recovery
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryConfig {
    #[serde(default)]
    pub enabled: bool,
    pub role: DRRole,
    pub peer_cluster_id: String,
    #[serde(default)]
    pub sync_strategy: DRSyncStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failover_dns: Option<ExternalDNSConfig>,
    #[serde(default = "default_dr_check_interval")]
    pub health_check_interval: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drill_schedule: Option<DRDrillScheduleConfig>,

    /// Configuration for history archive integrity checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_integrity_config: Option<ArchiveIntegrityConfig>,
}

/// Configuration for periodic history archive integrity checks
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveIntegrityConfig {
    /// Enable periodic integrity checks
    pub enabled: bool,
    /// Interval between integrity checks (e.g. "1h", "6h", "24h")
    #[serde(default = "default_archive_check_interval")]
    pub interval: String,
    /// Percentage of checkpoints to verify in each run (1-100)
    #[serde(default = "default_archive_check_percentage")]
    pub check_percentage: u32,
    /// Maximum number of historical checkpoints to verify in each run
    #[serde(default = "default_archive_check_max_checkpoints")]
    pub max_checkpoints: u32,
}

fn default_archive_check_interval() -> String {
    "6h".to_string()
}

fn default_archive_check_percentage() -> u32 {
    5
}

fn default_archive_check_max_checkpoints() -> u32 {
    10
}

fn default_dr_check_interval() -> u32 {
    30
}

/// Role of a node in a DR configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DRRole {
    Primary,
    Standby,
}

/// Synchronization strategy for hot standby nodes
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DRSyncStrategy {
    #[default]
    Consensus,
    PeerTracking,
    ArchiveSync,
}

/// Configuration for multi-region ledger replication
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Replication mode (currently only asynchronous supported)
    pub mode: ReplicationMode,
    /// Role of this cluster in the replication setup (Active/Passive)
    pub role: ReplicationRole,
    /// Identifier of the remote cluster
    pub remote_cluster_id: String,
    /// Networking configuration for cross-cluster connectivity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub networking: Option<ReplicationNetworkingConfig>,
}

/// Replication mode for ledger data
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReplicationMode {
    Asynchronous,
}

/// Role of a cluster in a replication configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReplicationRole {
    Active,
    Passive,
}

/// Cross-cluster networking configuration for replication
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationNetworkingConfig {
    /// VPN-based cross-cluster connectivity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpn: Option<VpnConfig>,
    /// VPC Peering or Cloud Interconnect/DirectConnect based connectivity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peering: Option<PeeringConfig>,
}

/// VPN configuration for cross-cluster replication
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VpnConfig {
    /// Public IP or DNS of the remote VPN gateway
    pub remote_gateway: String,
    /// Pre-shared key secret reference
    pub psk_secret_ref: String,
    /// Local CIDR range to advertise
    pub local_cidr: String,
    /// Remote CIDR range to expect
    pub remote_cidr: String,
}

/// VPC Peering configuration for cross-cluster replication
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeeringConfig {
    /// ID of the remote VPC/VNet
    pub peer_vpc_id: String,
    /// ID of the peer cloud account/project
    pub peer_project_id: String,
    /// Region of the peer VPC
    pub peer_region: String,
}

/// Status of the Disaster Recovery setup
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryStatus {
    pub current_role: Option<DRRole>,
    /// Active peer cluster (primary or standby target)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_peer_cluster_id: Option<String>,
    pub peer_health: Option<String>,
    /// Per-peer health status for multi-region DR
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_health_map: Option<Vec<DRPeerHealth>>,
    pub last_peer_contact: Option<String>,
    pub sync_lag: Option<u64>,
    pub failover_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failover_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failover_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_check_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_drill_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_drill_result: Option<DRDrillResult>,
}

/// Health status for a DR peer cluster.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DRPeerHealth {
    pub cluster_id: String,
    pub health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_contact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
}

/// Configuration for automated DR drill scheduling
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DRDrillScheduleConfig {
    /// Cron expression for drill scheduling (e.g., "0 2 * * 0" for weekly Sunday 2 AM)
    pub schedule: String,
    /// Whether to actually perform failover or just simulate it (dry-run)
    #[serde(default)]
    pub dry_run: bool,
    /// Maximum time to wait for failover to complete (seconds)
    #[serde(default = "default_drill_timeout")]
    pub timeout_seconds: u32,
    /// Whether to automatically rollback after drill completion
    #[serde(default = "default_drill_auto_rollback")]
    pub auto_rollback: bool,
    /// Rollback delay after drill completion (seconds)
    #[serde(default = "default_drill_rollback_delay")]
    pub rollback_delay_seconds: u32,
}

fn default_drill_timeout() -> u32 {
    300 // 5 minutes
}

fn default_drill_auto_rollback() -> bool {
    true
}

fn default_drill_rollback_delay() -> u32 {
    60 // 1 minute
}

/// Result of a DR drill execution
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DRDrillResult {
    /// Drill execution status
    pub status: DRDrillStatus,
    /// Time to recovery in milliseconds
    pub time_to_recovery_ms: Option<u64>,
    /// Whether standby successfully took over
    pub standby_takeover_success: bool,
    /// Whether application remained available during drill
    pub application_availability: bool,
    /// Human-readable message about drill result
    pub message: String,
    /// Timestamp when drill started
    pub started_at: String,
    /// Timestamp when drill completed
    pub completed_at: Option<String>,
}

/// Placement configuration for intelligent pod scheduling.
/// Enables SCP-aware anti-affinity to ensure validator resilience.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlacementConfig {
    /// Enable SCP-aware anti-affinity.
    /// When true, the operator will inject podAntiAffinity rules to discourage
    /// placing nodes from the same quorum slice on the same physical host.
    #[serde(default)]
    pub scp_aware_anti_affinity: bool,

    /// Jurisdictional compliance configuration.
    ///
    /// When set, the operator enforces that this node is physically placed in
    /// the specified geographical jurisdiction by injecting `nodeAffinity` and
    /// `tolerations` that match the corresponding Kubernetes node labels.
    ///
    /// # Example
    /// ```yaml
    /// placement:
    ///   jurisdiction:
    ///     code: "EU"
    ///     regions:
    ///       - "eu-west-1"
    ///       - "eu-central-1"
    ///     tolerations:
    ///       - key: "jurisdiction"
    ///         operator: "Equal"
    ///         value: "EU"
    ///         effect: "NoSchedule"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<JurisdictionConfig>,
}

/// Jurisdictional compliance configuration for node placement.
///
/// Maps a jurisdiction code (e.g. `"EU"`, `"US"`, `"SG"`) to Kubernetes
/// node labels so that the operator can enforce physical placement via
/// `nodeAffinity` and `tolerations`.
///
/// The operator uses `topology.kubernetes.io/region` by default, but any
/// label key can be specified via `label_key`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JurisdictionConfig {
    /// ISO 3166-1 alpha-2 country code or a custom jurisdiction identifier
    /// (e.g. `"EU"`, `"US"`, `"SG"`, `"DE"`).
    pub code: String,

    /// List of Kubernetes region values that satisfy this jurisdiction.
    /// Mapped to the `label_key` node label (default: `topology.kubernetes.io/region`).
    ///
    /// Example: `["eu-west-1", "eu-central-1"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<String>,

    /// Node label key used for region matching.
    /// Defaults to `topology.kubernetes.io/region`.
    #[serde(default = "default_jurisdiction_label_key")]
    pub label_key: String,

    /// Additional tolerations to apply when scheduling in this jurisdiction.
    /// Useful when jurisdiction-specific nodes carry taints (e.g. `jurisdiction=EU:NoSchedule`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<serde_json::Value>")]
    pub tolerations: Vec<k8s_openapi::api::core::v1::Toleration>,
}

fn default_jurisdiction_label_key() -> String {
    "topology.kubernetes.io/region".to_string()
}

/// Status of a DR drill execution
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DRDrillStatus {
    Pending,
    Running,
    Success,
    Failed,
    RolledBack,
}

/// Configuration for cross-cluster communication
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrossClusterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: CrossClusterMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_mesh: Option<CrossClusterServiceMeshConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_name: Option<ExternalNameConfig>,
    /// Federation configuration for CRD synchronization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federation: Option<FederationConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_clusters: Vec<PeerClusterConfig>,
    #[serde(default = "default_latency_threshold")]
    pub latency_threshold_ms: u32,
    #[serde(default)]
    pub auto_discovery: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_check: Option<CrossClusterHealthCheck>,
}

fn default_latency_threshold() -> u32 {
    200
}

impl Default for CrossClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: CrossClusterMode::default(),
            service_mesh: None,
            external_name: None,
            federation: None,
            peer_clusters: Vec::new(),
            latency_threshold_ms: default_latency_threshold(),
            auto_discovery: false,
            health_check: None,
        }
    }
}

/// Federation configuration for cross-cluster CRD synchronization
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub sync_crds: bool,
    #[serde(default = "default_true")]
    pub sync_stellarnodes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_namespace: Option<String>,
}

/// Cross-cluster networking mode
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CrossClusterMode {
    #[default]
    ServiceMesh,
    ExternalName,
    DirectIP,
}

/// Service mesh configuration for cross-cluster networking
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrossClusterServiceMeshConfig {
    pub mesh_type: CrossClusterMeshType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_set_id: Option<String>,
    #[serde(default = "default_true")]
    pub mtls_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_export: Option<ServiceExportConfig>,
    #[serde(default)]
    pub traffic_policy: CrossClusterTrafficPolicy,
}

/// Supported service mesh types for cross-cluster networking
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CrossClusterMeshType {
    Submariner,
    Istio,
    Linkerd,
    Cilium,
}

/// Service export configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceExportConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_clusters: Vec<String>,
}

/// Traffic policy for cross-cluster routing
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CrossClusterTrafficPolicy {
    #[default]
    LocalPreferred,
    Global,
    LocalOnly,
    LatencyBased,
}

/// ExternalName service configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalNameConfig {
    pub external_dns_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_provider: Option<String>,
    #[serde(default = "default_dns_ttl")]
    pub ttl: u32,
    #[serde(default = "default_true")]
    pub create_external_name_services: bool,
}

/// Peer cluster configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeerClusterConfig {
    pub cluster_id: String,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_threshold_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default = "default_peer_priority")]
    pub priority: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Secret containing kubeconfig for federation sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kubeconfig_secret_ref: Option<String>,
    /// Key in the kubeconfig secret data (default: "kubeconfig")
    #[serde(default = "default_kubeconfig_key")]
    pub kubeconfig_secret_key: String,
    /// Optional namespace override for federated resources
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_namespace: Option<String>,
}

fn default_kubeconfig_key() -> String {
    "kubeconfig".to_string()
}

fn default_peer_priority() -> u32 {
    100
}

/// Health check configuration for cross-cluster peers
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrossClusterHealthCheck {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_health_check_interval")]
    pub interval_seconds: u32,
    #[serde(default = "default_health_check_timeout")]
    pub timeout_seconds: u32,
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,
    #[serde(default = "default_success_threshold")]
    pub success_threshold: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_measurement: Option<LatencyMeasurementConfig>,
}

fn default_health_check_interval() -> u32 {
    30
}

fn default_health_check_timeout() -> u32 {
    5
}

fn default_failure_threshold() -> u32 {
    3
}

fn default_success_threshold() -> u32 {
    1
}

/// Latency measurement configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LatencyMeasurementConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub method: LatencyMeasurementMethod,
    #[serde(default = "default_latency_samples")]
    pub sample_count: u32,
    #[serde(default = "default_latency_percentile")]
    pub percentile: u8,
}

fn default_latency_samples() -> u32 {
    10
}

fn default_latency_percentile() -> u8 {
    95
}

/// Method for measuring cross-cluster latency
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LatencyMeasurementMethod {
    #[default]
    Ping,
    TCP,
    HTTP,
    GRPC,
}

// ============================================================================
// CVE Handling Configuration
// ============================================================================
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CVEHandlingConfig {
    #[serde(default = "default_cve_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cve_scan_interval")]
    pub scan_interval_secs: u64,
    #[serde(default)]
    pub critical_only: bool,
    #[serde(default = "default_canary_timeout")]
    pub canary_test_timeout_secs: u64,
    #[serde(default = "default_canary_pass_rate")]
    pub canary_pass_rate_threshold: f64,
    #[serde(default = "default_enable_rollback")]
    pub enable_auto_rollback: bool,
    #[serde(default = "default_health_threshold")]
    pub consensus_health_threshold: f64,
}

fn default_cve_enabled() -> bool {
    true
}

fn default_cve_scan_interval() -> u64 {
    3600
}

fn default_canary_timeout() -> u64 {
    300
}

fn default_canary_pass_rate() -> f64 {
    100.0
}

fn default_enable_rollback() -> bool {
    true
}

fn default_health_threshold() -> f64 {
    0.95
}

impl Default for CVEHandlingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scan_interval_secs: 3600,
            critical_only: false,
            canary_test_timeout_secs: 300,
            canary_pass_rate_threshold: 100.0,
            enable_auto_rollback: true,
            consensus_health_threshold: 0.95,
        }
    }
}

// ============================================================================
// CloudNativePG Managed Database Configuration
// ============================================================================

/// Configuration for managed High-Availability Postgres clusters via CloudNativePG
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedDatabaseConfig {
    #[serde(default = "default_db_instances")]
    pub instances: i32,
    pub storage: StorageConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<ManagedDatabaseBackupConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pooling: Option<PgBouncerConfig>,
    #[serde(default = "default_postgres_version")]
    pub postgres_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

fn default_db_instances() -> i32 {
    3
}

fn default_postgres_version() -> String {
    "16".to_string()
}

/// Backup configuration for managed databases using Barman
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedDatabaseBackupConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub destination_path: String,
    pub credentials_secret_ref: String,
    #[serde(default = "default_retention")]
    pub retention_policy: String,
}

fn default_retention() -> String {
    "30d".to_string()
}

/// pgBouncer connection pooling configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PgBouncerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_pooler_replicas")]
    pub replicas: i32,
    #[serde(default)]
    pub pool_mode: PgBouncerPoolMode,
    #[serde(default = "default_max_client_conn")]
    pub max_client_conn: i32,
    #[serde(default = "default_pool_size")]
    pub default_pool_size: i32,
}

// ============================================================================
// Database Maintenance Configuration
// ============================================================================

/// Configuration for automated database maintenance (VACUUM, Reindexing)
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DbMaintenanceConfig {
    /// Enable automated database maintenance
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Maintenance window start time (24h format, e.g., "02:00")
    /// Maintenance will only trigger during this window
    pub window_start: String,

    /// Maintenance window duration (e.g., "2h")
    pub window_duration: String,

    /// Bloat threshold percentage to trigger VACUUM FULL (default: 30)
    #[serde(default = "default_bloat_threshold")]
    pub bloat_threshold_percent: u32,

    /// Automatically reindex bloated tables
    #[serde(default = "default_true")]
    pub auto_reindex: bool,

    /// Coordination with read-pool for zero-downtime
    #[serde(default = "default_true")]
    pub read_pool_coordination: bool,
}

fn default_bloat_threshold() -> u32 {
    30
}

fn default_pooler_replicas() -> i32 {
    2
}

fn default_max_client_conn() -> i32 {
    1000
}

fn default_pool_size() -> i32 {
    20
}

/// pgBouncer pooling modes
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PgBouncerPoolMode {
    Session,
    #[default]
    Transaction,
    Statement,
}

// ============================================================================
// ============================================================================
// Hitless Upgrade (#503)
// ============================================================================

/// Configuration for zero-interruption (hitless) upgrades of Stellar Core.
///
/// When enabled, the operator injects a `stellar-handoff` sidecar that
/// transfers open peer TCP socket file descriptors to the new container
/// via `SCM_RIGHTS` over a Unix domain socket, avoiding peer re-discovery.
///
/// See `docs/hitless-upgrade.md` for the full design and feasibility study.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HitlessUpgradeConfig {
    /// Enable hitless upgrade support.
    /// When `true`, the `stellar-handoff` sidecar is injected into the pod.
    #[serde(default)]
    pub enabled: bool,

    /// Maximum seconds to wait for the FD handoff to complete before
    /// falling back to a standard rolling restart.
    /// Default: 10
    #[serde(default = "default_handoff_timeout")]
    pub handoff_timeout_seconds: u32,

    /// Fall back to a standard rolling restart if the handoff times out.
    /// Default: true
    #[serde(default = "default_true")]
    pub fallback_to_rolling_restart: bool,

    /// Container image for the handoff sidecar.
    /// Defaults to the same image as the operator with the `handoff-sidecar` binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_image: Option<String>,
}

fn default_handoff_timeout() -> u32 {
    10
}

impl Default for HitlessUpgradeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            handoff_timeout_seconds: default_handoff_timeout(),
            fallback_to_rolling_restart: true,
            sidecar_image: None,
        }
    }
}

// NAT Traversal Configuration
// ============================================================================

/// Configuration for NAT traversal (STUN/TURN/ICE) for P2P networking.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NatTraversalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stun_server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_credentials_secret_ref: Option<String>,
    #[serde(default = "default_true")]
    pub enable_ice: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_image: Option<String>,
}

// ============================================================================
// OCI Snapshot Sync (#231)
// ============================================================================

/// Strategy for generating the OCI image tag for a ledger snapshot
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TagStrategy {
    /// Tag the image with the current ledger sequence number, e.g. `snapshot-12345678`
    #[default]
    LatestLedger,
    /// Always use the same fixed tag, e.g. `latest` or `stable`
    Fixed,
}

/// Configuration for packaging and syncing ledger snapshots via an OCI registry.
///
/// When `push` is enabled the operator will create a Kubernetes Job after the node
/// reaches Ready state that tars the contents of the node's data PVC and pushes it
/// as an OCI image layer to the configured registry.
///
/// When `pull` is enabled the operator will create a Job that pulls the most recent
/// snapshot image and extracts it onto a freshly provisioned PVC before the node pod
/// starts, enabling fast bootstrapping of new validator/RPC nodes across regions.
///
/// Registry credentials are read from a K8s Secret (`.dockerconfigjson` format) whose
/// name is specified in `credential_secret_name`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OciSnapshotConfig {
    /// Whether the OCI snapshot feature is enabled (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// OCI registry host, e.g. `ghcr.io` or `registry-1.docker.io`
    pub registry: String,

    /// Image name within the registry, e.g. `myorg/stellar-snapshot`
    pub image: String,

    /// Tag used when pushing/pulling the snapshot image.
    /// With `LatestLedger` the tag is `snapshot-<ledger_seq>`; with `Fixed` the
    /// literal `fixed_tag` value is used.
    #[serde(default)]
    pub tag_strategy: TagStrategy,

    /// Fixed tag to use when `tag_strategy` is `Fixed` (e.g. `latest`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_tag: Option<String>,

    /// Name of a K8s Secret in the same namespace containing Docker registry
    /// credentials as `config.json` (standard `~/.docker/config.json` format).
    pub credential_secret_name: String,

    /// Enable pushing snapshots to the registry (default: false)
    #[serde(default)]
    pub push: bool,

    /// Enable pulling a snapshot to bootstrap a new node's PVC (default: false)
    #[serde(default)]
    pub pull: bool,

    /// Image reference to pull from (full `registry/image:tag` string).
    /// Required when `pull = true`; if omitted the operator constructs the reference
    /// from `registry`, `image`, and `tag_strategy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_image_ref: Option<String>,
}

// ============================================================================
// Label Propagation Configuration
// ============================================================================

/// Filter policy controlling which StellarNode labels are propagated to child resources.
///
/// When both lists are empty, all user labels are propagated (subject to the implicit
/// denylist for `kubernetes.io/` and `k8s.io/` prefixes).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelPropagationConfig {
    /// Glob patterns for label keys that are allowed to propagate.
    /// When empty, all user labels are eligible (subject to denyList).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_list: Vec<String>,

    /// Glob patterns for label keys that are always blocked from propagation.
    /// Takes precedence over allowList.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_list: Vec<String>,
}

// ── Cross-Cloud Failover ─────────────────────────────────────────────────────

/// Cross-cloud failover configuration for Horizon clusters.
///
/// Enables seamless traffic failover between cloud providers (AWS, GCP, Azure)
/// during major provider outages, targeting 99.99% availability.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrossCloudFailoverConfig {
    /// Enable cross-cloud failover
    #[serde(default)]
    pub enabled: bool,

    /// Role of this cluster in the cross-cloud setup
    #[serde(default)]
    pub role: CrossCloudRole,

    /// Cloud provider identifier for this cluster (e.g. "aws", "gcp", "azure")
    pub primary_cloud_provider: String,

    /// All cloud endpoints participating in the failover pool
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clouds: Vec<CloudEndpointConfig>,

    /// Global Load Balancer configuration (Cloudflare, F5, AWS Global Accelerator)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_load_balancer: Option<GlobalLoadBalancerConfig>,

    /// Database synchronization configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_sync: Option<DatabaseSyncConfig>,

    /// Number of consecutive health check failures before triggering failover
    #[serde(default = "default_failure_threshold_cc")]
    pub failure_threshold: Option<u32>,

    /// Health check timeout in seconds
    #[serde(default = "default_health_check_timeout_cc")]
    pub health_check_timeout_seconds: Option<u32>,

    /// Automatically fail back to primary cloud when it recovers
    #[serde(default)]
    pub auto_failback: Option<bool>,
}

fn default_failure_threshold_cc() -> Option<u32> {
    Some(3)
}

fn default_health_check_timeout_cc() -> Option<u32> {
    Some(5)
}

/// Role of this cluster in the cross-cloud failover setup
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CrossCloudRole {
    /// This cluster is the primary traffic destination
    #[default]
    Primary,
    /// This cluster is a warm standby
    Secondary,
}

/// Configuration for a single cloud endpoint in the failover pool
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloudEndpointConfig {
    /// Cloud provider identifier (e.g. "aws", "gcp", "azure")
    pub cloud_provider: String,

    /// Cloud region (e.g. "us-east-1", "us-central1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    /// Public hostname or IP of the Horizon cluster in this cloud
    pub endpoint: String,

    /// Priority for failover selection (higher = preferred). Default: 100
    #[serde(default = "default_cloud_priority")]
    pub priority: u32,

    /// Whether this cloud endpoint is active
    #[serde(default = "default_true_cc")]
    pub enabled: bool,
}

fn default_cloud_priority() -> u32 {
    100
}

fn default_true_cc() -> bool {
    true
}

/// Global Load Balancer provider and configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalLoadBalancerConfig {
    /// GLB provider
    pub provider: GLBProvider,

    /// Public hostname managed by the GLB (e.g. "horizon.stellar.example.com")
    pub hostname: String,

    /// Path used for health checks (default: "/health")
    #[serde(default = "default_health_path")]
    pub health_check_path: Option<String>,

    /// DNS TTL in seconds (lower = faster failover, higher = less DNS traffic)
    #[serde(default = "default_glb_ttl")]
    pub ttl_seconds: u32,

    /// Kubernetes Secret containing GLB API credentials
    /// For Cloudflare: keys `CF_API_TOKEN` and `CF_ZONE_ID`
    /// For F5: keys `F5_USERNAME` and `F5_PASSWORD`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret_ref: Option<String>,
}

fn default_health_path() -> Option<String> {
    Some("/health".to_string())
}

fn default_glb_ttl() -> u32 {
    60
}

/// Supported Global Load Balancer providers
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GLBProvider {
    /// Cloudflare Load Balancing / DNS
    #[default]
    Cloudflare,
    /// F5 BIG-IP Global Traffic Manager
    F5,
    /// AWS Global Accelerator
    AWSGlobalAccelerator,
    /// Generic external-dns (works with any DNS provider)
    ExternalDNS,
}

/// Database synchronization configuration for cross-cloud failover
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSyncConfig {
    /// Synchronization method
    pub method: DatabaseSyncMethod,

    /// PostgreSQL logical replication slot name (for LogicalReplication method)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replication_slot: Option<String>,

    /// Maximum acceptable replication lag in seconds before blocking failover
    #[serde(default = "default_max_lag")]
    pub max_lag_seconds: Option<u32>,

    /// Secret containing database credentials for the standby cluster
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standby_credentials_secret_ref: Option<String>,
}

fn default_max_lag() -> Option<u32> {
    Some(30)
}

/// Database synchronization method
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseSyncMethod {
    /// PostgreSQL logical replication (lowest RPO, requires pg_logical)
    #[default]
    LogicalReplication,
    /// CloudNativePG cross-cluster replica (uses CNPG Cluster with externalClusters)
    CNPGCrossCluster,
    /// Periodic snapshot restore (higher RPO, simpler setup)
    SnapshotRestore,
}

/// Status of the cross-cloud failover
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrossCloudFailoverStatus {
    /// Current role of this cluster
    pub current_role: Option<CrossCloudRole>,

    /// Whether a failover is currently active
    #[serde(default)]
    pub failover_active: bool,

    /// Cloud provider currently receiving traffic
    pub active_cloud: Option<String>,

    /// Health status of each cloud endpoint
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_health: Option<Vec<crate::controller::cross_cloud_failover::CloudHealthStatus>>,

    /// Timestamp of the last health check
    pub last_check_time: Option<String>,

    /// Timestamp of the last failover
    pub last_failover_time: Option<String>,

    /// Reason for the last failover
    pub last_failover_reason: Option<String>,

    /// Timestamp of the last failback
    pub last_failback_time: Option<String>,

    /// Timestamp of the last failover attempt (may have been blocked)
    pub last_failover_attempt: Option<String>,
}

// ============================================================================
// History Archive Pruning Policy
// ============================================================================

/// Retention policy for history archive pruning
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PruningPolicy {
    /// Enable automatic pruning of history archives
    #[serde(default)]
    pub enabled: bool,

    /// Retention period in days. Checkpoints older than this will be deleted.
    /// Mutually exclusive with retention_ledgers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,

    /// Retention period in ledgers. Checkpoints older than this will be deleted.
    /// Mutually exclusive with retention_days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_ledgers: Option<u32>,

    /// Minimum number of checkpoints to always retain, regardless of age.
    /// Provides a safety buffer to ensure recent history is always available.
    /// Must be at least 10 (hardcoded minimum for safety).
    #[serde(default = "default_min_checkpoints")]
    pub min_checkpoints: u32,

    /// Maximum age of checkpoints to consider for deletion (in days).
    /// Checkpoints newer than this will never be deleted, even if they exceed retention.
    /// Provides additional safety against accidentally deleting recent checkpoints.
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u32,

    /// Number of concurrent deletion operations.
    /// Higher values speed up pruning but may hit API rate limits.
    #[serde(default = "default_pruning_concurrency")]
    pub concurrency: usize,

    /// Cron expression for scheduled pruning (e.g., "0 2 * * *" for daily at 2 AM).
    /// If unset, pruning is only triggered manually via annotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,

    /// Whether to automatically execute deletions or only report what would be deleted.
    /// When false (default), only dry-run analysis is performed.
    #[serde(default)]
    pub auto_delete: bool,

    /// Skip confirmation prompt when auto_delete is true.
    /// Only applicable when auto_delete is enabled.
    #[serde(default)]
    pub skip_confirmation: bool,
}

fn default_min_checkpoints() -> u32 {
    50
}

fn default_max_age_days() -> u32 {
    7
}

fn default_pruning_concurrency() -> usize {
    10
}

impl Default for PruningPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: None,
            retention_ledgers: None,
            min_checkpoints: default_min_checkpoints(),
            max_age_days: default_max_age_days(),
            concurrency: default_pruning_concurrency(),
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        }
    }
}

impl PruningPolicy {
    /// Validate the pruning policy configuration
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        // Ensure exactly one retention policy is specified
        match (self.retention_days, self.retention_ledgers) {
            (None, None) => {
                return Err("Must specify either retention_days or retention_ledgers".to_string());
            }
            (Some(_), Some(_)) => {
                return Err("Cannot specify both retention_days and retention_ledgers".to_string());
            }
            _ => {}
        }

        // Validate min_checkpoints
        if self.min_checkpoints < 10 {
            return Err(format!(
                "min_checkpoints must be at least 10, got {}",
                self.min_checkpoints
            ));
        }

        // Validate max_age_days
        if self.max_age_days == 0 {
            return Err("max_age_days must be greater than 0".to_string());
        }

        // Validate concurrency
        if self.concurrency == 0 {
            return Err("concurrency must be greater than 0".to_string());
        }

        Ok(())
    }
}

/// Status of the last pruning operation
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PruningStatus {
    /// Timestamp of the last pruning operation
    pub last_run_time: Option<String>,

    /// Status of the last pruning operation (Pending, Running, Success, Failed)
    pub last_run_status: Option<String>,

    /// Total checkpoints found in the last scan
    pub total_checkpoints: Option<u32>,

    /// Checkpoints deleted in the last operation
    pub deleted_count: Option<u32>,

    /// Checkpoints retained in the last operation
    pub retained_count: Option<u32>,

    /// Total bytes freed in the last operation
    pub bytes_freed: Option<u64>,

    /// Human-readable message about the last operation
    pub message: Option<String>,

    /// Whether the last operation was a dry-run
    pub dry_run: Option<bool>,
}
// ── Gas Autoscaling default functions ────────────────────────────────────────

fn default_gas_min_replicas() -> u32 {
    1
}

fn default_gas_max_replicas() -> u32 {
    5
}

fn default_scale_up_threshold() -> f64 {
    2_000_000.0
}

fn default_scale_down_threshold() -> f64 {
    500_000.0
}

fn default_target_gas_trend_score() -> f64 {
    1_000_000.0
}

fn default_scale_step() -> u32 {
    1
}

fn default_scale_up_cooldown() -> String {
    "60s".to_string()
}

fn default_scale_down_cooldown() -> String {
    "300s".to_string()
}

fn default_ledger_window() -> u32 {
    10
}

fn default_ewma_alpha() -> f64 {
    0.3
}

fn default_poll_interval_seconds() -> u32 {
    6
}

/// Gas-consumption-driven autoscaling for Soroban RPC nodes.
/// Only valid when spec.nodeType == SorobanRPC.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GasAutoscalingConfig {
    /// Enable gas-based autoscaling.
    #[serde(default)]
    pub enabled: bool,

    /// Minimum replica count. Must be >= 1.
    #[serde(default = "default_gas_min_replicas")]
    pub min_replicas: u32,

    /// Maximum replica count. Must be >= min_replicas.
    #[serde(default = "default_gas_max_replicas")]
    pub max_replicas: u32,

    /// Gas_Trend_Score above which a scale-up is triggered.
    #[serde(default = "default_scale_up_threshold")]
    pub scale_up_threshold: f64,

    /// Gas_Trend_Score below which a scale-down is triggered.
    #[serde(default = "default_scale_down_threshold")]
    pub scale_down_threshold: f64,

    /// HPA external metric target value (used by K8s HPA).
    #[serde(default = "default_target_gas_trend_score")]
    pub target_gas_trend_score: f64,

    /// Number of replicas to add per scale-up event.
    #[serde(default = "default_scale_step")]
    pub scale_up_step: u32,

    /// Number of replicas to remove per scale-down event.
    #[serde(default = "default_scale_step")]
    pub scale_down_step: u32,

    /// Cooldown after a scale-up event (e.g. "60s", "2m").
    #[serde(default = "default_scale_up_cooldown")]
    pub scale_up_cooldown: String,

    /// Cooldown after a scale-down event (e.g. "300s", "5m").
    #[serde(default = "default_scale_down_cooldown")]
    pub scale_down_cooldown: String,

    /// Number of recent ledgers to include in the EWMA window.
    #[serde(default = "default_ledger_window")]
    pub ledger_window: u32,

    /// EWMA decay factor alpha. Must be in (0.0, 1.0) exclusive.
    #[serde(default = "default_ewma_alpha")]
    pub ewma_alpha: f64,

    /// Polling interval in seconds (default: 6, matching average ledger close time).
    #[serde(default = "default_poll_interval_seconds")]
    pub poll_interval_seconds: u32,
}

// =============================================================================
// RBAC, Audit Logging, and Policy Configuration
// =============================================================================

/// Fine-grained operator roles for RBAC
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum OperatorRole {
    /// Full access to all resources and actions
    SuperAdmin,
    /// Can perform operational tasks but not modify security settings
    Operator,
    /// Read-only access to audit logs and system status
    Auditor,
    /// Read-only access to system status
    Viewer,
}

/// RBAC configuration for operator-level permissions
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RbacConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Map of Kubernetes subjects (users/groups/SAs) to Operator roles
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_bindings: Vec<RoleBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleBinding {
    /// Kind of the subject: "User", "Group", or "ServiceAccount"
    pub subject_kind: String,
    /// Name of the subject
    pub subject_name: String,
    /// Role assigned to this subject
    pub role: OperatorRole,
}

/// Audit logging configuration
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Where to send audit logs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sinks: Vec<AuditSinkConfig>,
    /// Retention policy for audit logs
    #[serde(default)]
    pub retention: AuditRetentionPolicy,
    /// Field-level encryption for sensitive data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<FieldLevelEncryptionConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditSinkConfig {
    #[serde(rename = "type")]
    pub type_: AuditSinkType,
    /// Optional endpoint for the sink (e.g. S3 URL, Database DSN)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Optional credentials for the sink
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum AuditSinkType {
    Stdout,
    File,
    Database,
    ExternalAggregator,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditRetentionPolicy {
    /// Maximum age of audit logs in days
    pub max_age_days: u32,
    /// Maximum size of audit logs in MB before rotation
    pub max_size_mb: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldLevelEncryptionConfig {
    pub enabled: bool,
    /// Reference to a KMS key for encryption
    pub kms_key_ref: String,
}

/// Policy-based authorization (OPA/Gatekeeper)
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Open Policy Agent (OPA) configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opa: Option<OPAConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OPAConfig {
    /// OPA server endpoint
    pub endpoint: String,
    /// If true, allows the action if OPA is unreachable
    #[serde(default)]
    pub fail_open: bool,
}
