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
    Custom,
}

impl StellarNetwork {
    pub fn passphrase<'a>(&'a self, custom: &'a Option<String>) -> &'a str {
        match self {
            StellarNetwork::Mainnet => "Public Global Stellar Network ; September 2015",
            StellarNetwork::Testnet => "Test SDF Network ; September 2015",
            StellarNetwork::Futurenet => "Test SDF Future Network ; October 2022",
            StellarNetwork::Custom => custom.as_deref().unwrap_or(""),
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
    pub fn scheduling_label_value(&self, custom: &Option<String>) -> String {
        match self {
            StellarNetwork::Mainnet => "mainnet".to_string(),
            StellarNetwork::Testnet => "testnet".to_string(),
            StellarNetwork::Futurenet => "futurenet".to_string(),
            StellarNetwork::Custom => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                let passphrase = custom.as_deref().unwrap_or("");
                passphrase.hash(&mut h);
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
        }
    }
}

/// PVC retention policy on node deletion
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum RetentionPolicy {
    #[default]
    Delete,
    Retain,
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
    /// Cloud HSM configuration for secure key loading (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hsm_config: Option<HsmConfig>,
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
}

/// External database configuration for managed Postgres databases
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDatabaseConfig {
    pub secret_key_ref: SecretKeyRef,
}

/// Reference to a key within a Kubernetes Secret
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub name: String,
    pub key: String,
}

/// Ingress configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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
}

/// Ingress host entry
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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
}

/// Scaling behavior configuration for HPA
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScalingBehavior {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_up: Option<ScalingPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_down: Option<ScalingPolicy>,
}

/// Scaling policy
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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
}

/// Rollout strategy for updates
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
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
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanaryConfig {
    #[serde(default = "default_canary_weight")]
    pub weight: i32,
    #[serde(default = "default_canary_interval")]
    pub check_interval_seconds: i32,
}

fn default_canary_weight() -> i32 {
    10
}

fn default_canary_interval() -> i32 {
    300
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

/// Status of the Disaster Recovery setup
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryStatus {
    pub current_role: Option<DRRole>,
    pub peer_health: Option<String>,
    pub last_peer_contact: Option<String>,
    pub sync_lag: Option<u64>,
    pub failover_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_drill_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_drill_result: Option<DRDrillResult>,
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlacementConfig {
    /// Enable SCP-aware anti-affinity.
    /// When true, the operator will inject podAntiAffinity rules to discourage
    /// placing nodes from the same quorum slice on the same physical host.
    #[serde(default)]
    pub scp_aware_anti_affinity: bool,
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
            peer_clusters: Vec::new(),
            latency_threshold_ms: default_latency_threshold(),
            auto_discovery: false,
            health_check: None,
        }
    }
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
