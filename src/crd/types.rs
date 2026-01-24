//! Shared types for Stellar node specifications
//!
//! These types are used across the CRD definitions and controller logic.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Supported Stellar node types
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum NodeType {
    /// Full validator node running Stellar Core
    /// Participates in consensus and validates transactions
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

/// Target Stellar network
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum StellarNetwork {
    /// Stellar public mainnet
    Mainnet,
    /// Stellar testnet for testing
    Testnet,
    /// Futurenet for bleeding-edge features
    Futurenet,
    /// Custom network with passphrase
    Custom(String),
}

impl StellarNetwork {
    /// Get the network passphrase for this network
    pub fn passphrase(&self) -> &str {
        match self {
            StellarNetwork::Mainnet => "Public Global Stellar Network ; September 2015",
            StellarNetwork::Testnet => "Test SDF Network ; September 2015",
            StellarNetwork::Futurenet => "Test SDF Future Network ; October 2022",
            StellarNetwork::Custom(passphrase) => passphrase,
        }
    }
}

/// Kubernetes-style resource requirements
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequirements {
    /// Minimum resources requested
    pub requests: ResourceSpec,
    /// Maximum resources allowed
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
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ResourceSpec {
    /// CPU cores (e.g., "500m", "2")
    pub cpu: String,
    /// Memory (e.g., "1Gi", "4Gi")
    pub memory: String,
}

/// Storage configuration for persistent data
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    /// Storage class name (e.g., "standard", "ssd", "premium-rwo")
    pub storage_class: String,
    /// Size of the PersistentVolumeClaim (e.g., "100Gi")
    pub size: String,
    /// Retention policy when the node is deleted
    #[serde(default)]
    pub retention_policy: RetentionPolicy,
    /// Optional annotations to apply to the PersistentVolumeClaim
    /// Useful for storage-class specific parameters (e.g., volumeBindingMode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_class: "standard".to_string(),
            size: "100Gi".to_string(),
            retention_policy: RetentionPolicy::default(),
            annotations: None,
        }
    }
}

/// PVC retention policy on node deletion
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum RetentionPolicy {
    /// Delete the PVC when the node is deleted
    #[default]
    Delete,
    /// Retain the PVC for manual cleanup or data recovery
    Retain,
}

/// Validator-specific configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorConfig {
    /// Secret name containing the validator seed (key: STELLAR_CORE_SEED)
    pub seed_secret_ref: String,
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
    /// Port number for peer-to-peer communication (default: 11625)
    /// Used for dynamic peer discovery to advertise reachability
    #[serde(default = "default_peer_port", skip_serializing_if = "Option::is_none")]
    pub peer_port: Option<u16>,
}

fn default_peer_port() -> Option<u16> {
    Some(11625)
}

/// Source of security keys
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    /// Use a standard Kubernetes Secret
    #[default]
    Secret,
    /// Fetch keys from a cloud KMS or Vault via init container
    KMS,
}

/// Configuration for cloud-native KMS or Vault
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KmsConfig {
    /// KMS Key ID, ARN, or Vault path (e.g., "alias/my-key" or "secret/stellar/validator-key")
    pub key_id: String,
    /// Provider name (e.g., "aws", "google", "vault")
    pub provider: String,
    /// Cloud region (e.g., "us-east-1", "europe-west1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Image to use for the KMS init container (default: stellar/kms-fetcher:latest)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetcher_image: Option<String>,
}

/// Horizon API server configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HorizonConfig {
    /// Secret reference for database credentials
    pub database_secret_ref: String,
    /// Enable real-time ingestion from Stellar Core
    #[serde(default = "default_true")]
    pub enable_ingest: bool,
    /// Stellar Core URL to ingest from
    pub stellar_core_url: String,
    /// Number of parallel ingestion workers
    #[serde(default = "default_ingest_workers")]
    pub ingest_workers: u32,
    /// Enable experimental features
    #[serde(default)]
    pub enable_experimental_ingestion: bool,
}

fn default_true() -> bool {
    true
}

fn default_ingest_workers() -> u32 {
    1
}

/// Soroban RPC server configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SorobanConfig {
    /// Stellar Core endpoint URL
    pub stellar_core_url: String,
    /// Captive Core configuration (TOML format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captive_core_config: Option<String>,
    /// Enable transaction simulation preflight
    #[serde(default = "default_true")]
    pub enable_preflight: bool,
    /// Maximum number of events to return per request
    #[serde(default = "default_max_events")]
    pub max_events_per_request: u32,
}

/// External database configuration for managed Postgres databases
/// Supports RDS, Cloud SQL, CockroachDB, and other managed database services
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDatabaseConfig {
    /// Reference to a Kubernetes Secret containing database credentials
    pub secret_key_ref: SecretKeyRef,
}

/// Reference to a key within a Kubernetes Secret
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    /// Name of the Secret resource
    pub name: String,
    /// Key within the Secret to use for the database connection string
    /// Common keys: "DATABASE_URL", "connection-string", "url"
    /// For individual components: "host", "port", "database", "user", "password"
    pub key: String,
}

/// Ingress configuration for exposing Horizon or Soroban RPC over HTTPS
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressConfig {
    /// Optional ingressClassName (e.g., "nginx", "traefik")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,

    /// Host rules with paths to route to the Service
    pub hosts: Vec<IngressHost>,

    /// TLS secret name used by the ingress controller for HTTPS termination
    /// If provided, all hosts are added to the TLS section
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_secret_name: Option<String>,

    /// cert-manager issuer name (namespaced)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_manager_issuer: Option<String>,

    /// cert-manager cluster issuer name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_manager_cluster_issuer: Option<String>,

    /// Additional annotations to attach to the Ingress
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

/// Ingress host entry
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressHost {
    /// DNS host name (e.g., "horizon.stellar.example.com")
    pub host: String,

    /// HTTP paths served for this host
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
    /// HTTP path prefix (e.g., "/")
    pub path: String,

    /// Path type ("Prefix" or "Exact")
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

/// Horizontal Pod Autoscaling configuration for Horizon and SorobanRpc nodes
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingConfig {
    /// Minimum number of replicas
    pub min_replicas: i32,

    /// Maximum number of replicas
    pub max_replicas: i32,

    /// Target CPU utilization percentage (0-100)
    /// When set, enables CPU-based scaling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_cpu_utilization_percentage: Option<i32>,

    /// List of custom metrics to scale on (e.g., ["http_requests_per_second"])
    /// Requires Prometheus Adapter to be installed in the cluster
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_metrics: Vec<String>,

    /// Behavior configuration for scale up/down
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<ScalingBehavior>,
}

/// Scaling behavior configuration for HPA
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScalingBehavior {
    /// Scale up configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_up: Option<ScalingPolicy>,

    /// Scale down configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_down: Option<ScalingPolicy>,
}

/// Scaling policy for scale up/down
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScalingPolicy {
    /// Stabilization window in seconds (how long to wait before scaling again)
    pub stabilization_window_seconds: Option<i32>,

    /// List of policies with different percentage/pod changes
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<HPAPolicy>,
}

/// Individual HPA policy
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HPAPolicy {
    /// Type of policy: "Percent" or "Pods"
    pub policy_type: String,

    /// Value for the policy (percentage or number of pods)
    pub value: i32,

    /// Period in seconds over which the policy is applied
    pub period_seconds: i32,
}

/// Condition for status reporting (Kubernetes convention)
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    /// Type of condition (e.g., "Ready", "Progressing", "Degraded")
    #[serde(rename = "type")]
    pub type_: String,
    /// Status of the condition: "True", "False", or "Unknown"
    pub status: String,
    /// Last time the condition transitioned
    pub last_transition_time: String,
    /// Machine-readable reason for the condition
    pub reason: String,
    /// Human-readable message
    pub message: String,
}

impl Condition {
    /// Create a new Ready condition
    pub fn ready(status: bool, reason: &str, message: &str) -> Self {
        Self {
            type_: "Ready".to_string(),
            status: if status { "True" } else { "False" }.to_string(),
            last_transition_time: chrono::Utc::now().to_rfc3339(),
            reason: reason.to_string(),
            message: message.to_string(),
        }
    }

    /// Create a new Progressing condition
    pub fn progressing(reason: &str, message: &str) -> Self {
        Self {
            type_: "Progressing".to_string(),
            status: "True".to_string(),
            last_transition_time: chrono::Utc::now().to_rfc3339(),
            reason: reason.to_string(),
            message: message.to_string(),
        }
    }
}
