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
///
/// Determines which Stellar service is deployed (Stellar Core, Horizon API, or Soroban RPC).
/// Each type has different resource requirements, network roles, and configuration options.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::NodeType;
///
/// let node_type = NodeType::Validator;
/// println!("Deploying {} node", node_type);
/// ```
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
///
/// Specifies which Stellar network the node connects to.
/// This determines the network passphrase, peer addresses, and historical data sources.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::StellarNetwork;
///
/// let network = StellarNetwork::Testnet;
/// let passphrase = network.passphrase();
/// assert_eq!(passphrase, "Test SDF Network ; September 2015");
/// ```
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
///
/// Specifies CPU and memory resource requests and limits for the node.
/// Follows Kubernetes conventions for resource quantities.
///
/// Resource quantities use the following formats:
/// - CPU: `"500m"` (millicores), `"2"` (cores), `"1.5"`
/// - Memory: `"512Mi"`, `"1Gi"`, `"2Gi"`
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::ResourceRequirements;
///
/// let resources = ResourceRequirements {
///     requests: Default::default(),
///     limits: Default::default(),
/// };
/// ```
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
///
/// Configures how node data is persisted to disk, including storage class selection,
/// size allocation, and cleanup behavior on node deletion.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::{StorageConfig, RetentionPolicy};
///
/// let storage = StorageConfig {
///     storage_class: "ssd".to_string(),
///     size: "500Gi".to_string(),
///     retention_policy: RetentionPolicy::Delete,
///     annotations: None,
/// };
/// ```
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
///
/// Determines whether the Persistent Volume Claim (PVC) is deleted or retained
/// when the StellarNode resource is deleted.
///
/// # Variants
///
/// - `Delete` (default) - PVC is deleted along with the node resource
/// - `Retain` - PVC persists for manual cleanup or data recovery
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum RetentionPolicy {
    /// Delete the PVC when the node is deleted
    #[default]
    Delete,
    /// Retain the PVC for manual cleanup or data recovery
    Retain,
}

/// Validator-specific configuration
///
/// Configuration for Stellar Core validator nodes, including seed management,
/// quorum set configuration, history archive setup, and key source preferences.
///
/// Validators authenticate network participants and validate transactions.
/// A validator must be configured with a seed key and optionally with a quorum set
/// to participate in consensus.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::{ValidatorConfig, KeySource};
///
/// let config = ValidatorConfig {
///     seed_secret_ref: "my-validator-seed".to_string(),
///     quorum_set: None,
///     enable_history_archive: true,
///     history_archive_urls: vec!["https://archive.example.com".to_string()],
///     catchup_complete: false,
///     key_source: KeySource::Secret,
///     kms_config: None,
/// };
/// ```
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
    /// Trusted source for Validator Selection List (VSL)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vl_source: Option<String>,
}

/// Source of security keys
///
/// Specifies where the validator seed key is stored and retrieved from.
///
/// # Variants
///
/// - `Secret` (default) - Use a standard Kubernetes Secret resource
/// - `KMS` - Fetch keys from a cloud KMS or Vault via an init container
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
///
/// Specifies cloud KMS (AWS KMS, GCP Cloud KMS, HashiCorp Vault) parameters
/// for securely fetching validator seeds.
///
/// When `KeySource::KMS` is selected, an init container runs to fetch the key
/// from the specified KMS before the main container starts.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::KmsConfig;
///
/// let kms = KmsConfig {
///     key_id: "arn:aws:kms:us-east-1:123456789012:key/12345678-1234-1234-1234-123456789012".to_string(),
///     provider: "aws".to_string(),
///     region: Some("us-east-1".to_string()),
///     fetcher_image: Some("stellar/kms-fetcher:latest".to_string()),
/// };
/// ```
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
///
/// Configuration for Horizon nodes that provide a REST API to query the Stellar ledger.
/// Horizon ingests data from Stellar Core and indexes it for fast queries.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::HorizonConfig;
///
/// let config = HorizonConfig {
///     database_secret_ref: "horizon-db-secret".to_string(),
///     enable_ingest: true,
///     stellar_core_url: "http://core.default:11626".to_string(),
///     ingest_workers: 4,
///     enable_experimental_ingestion: false,
/// };
/// ```
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
///
/// Configuration for Soroban RPC nodes that handle smart contract simulation
/// and transaction submission on Stellar's smart contract platform.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::SorobanConfig;
///
/// let config = SorobanConfig {
///     stellar_core_url: "http://core.default:11626".to_string(),
///     captive_core_config: None,
///     enable_preflight: true,
///     max_events_per_request: 10000,
/// };
/// ```
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
/// 
/// Specifies how to reference database credentials for external managed databases.
/// Supports AWS RDS, Google Cloud SQL, CockroachDB, and other managed services.
///
/// The operator injects database credentials as environment variables into the container.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::{ExternalDatabaseConfig, SecretKeyRef};
///
/// let config = ExternalDatabaseConfig {
///     secret_key_ref: SecretKeyRef {
///         name: "postgres-credentials".to_string(),
///         key: "DATABASE_URL".to_string(),
///     },
/// };
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDatabaseConfig {
    /// Reference to a Kubernetes Secret containing database credentials
    pub secret_key_ref: SecretKeyRef,
}

/// Reference to a key within a Kubernetes Secret
///
/// Used to reference database credentials, KMS keys, and other sensitive data
/// stored in Kubernetes Secrets.
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
///
/// Configures Kubernetes Ingress for external HTTP/HTTPS access to Horizon or Soroban RPC nodes.
/// Supports multiple hosts, path-based routing, TLS termination, and cert-manager integration.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::{IngressConfig, IngressHost, IngressPath};
///
/// let config = IngressConfig {
///     class_name: Some("nginx".to_string()),
///     hosts: vec![IngressHost {
///         host: "horizon.example.com".to_string(),
///         paths: vec![IngressPath {
///             path: "/".to_string(),
///             path_type: Some("Prefix".to_string()),
///         }],
///     }],
///     tls_secret_name: None,
///     cert_manager_issuer: Some("letsencrypt-prod".to_string()),
///     cert_manager_cluster_issuer: None,
///     annotations: None,
/// };
/// ```
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
///
/// Defines a single DNS host and the HTTP paths served for that host.
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
///
/// Defines a single HTTP path prefix or exact path for routing traffic to the service.
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
///
/// Configures Kubernetes Horizontal Pod Autoscaler (HPA) for automatic scaling
/// of Horizon and Soroban RPC nodes based on CPU or custom metrics.
/// Validators do not support autoscaling.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::AutoscalingConfig;
///
/// let config = AutoscalingConfig {
///     min_replicas: 2,
///     max_replicas: 10,
///     target_cpu_utilization_percentage: Some(70),
///     custom_metrics: vec![],
///     behavior: None,
/// };
/// ```
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
///
/// Defines scale-up and scale-down policies with stabilization windows
/// to control the rate and timing of replica changes.
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
///
/// Specifies a scaling policy with stabilization window and multiple policy options.
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
///
/// Defines a single scaling policy with a type (percentage or number of pods),
/// value, and time period.
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
///
/// Reports the status of a condition on the StellarNode resource.
/// Follows Kubernetes convention for condition reporting.
///
/// # Examples
///
/// ```rust,no_run
/// use stellar_k8s::crd::Condition;
///
/// let condition = Condition::ready(true, "Ready", "Node is ready");
/// ```
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
    /// ObservedGeneration represents the .metadata.generation that the condition was set based upon
    /// This field is optional and should be set by controllers to track which generation was observed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
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
            observed_generation: None,
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
            observed_generation: None,
        }
    }

    /// Create a new Degraded condition
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

    /// Set the observed generation for this condition
    pub fn with_observed_generation(mut self, generation: i64) -> Self {
        self.observed_generation = Some(generation);
        self
    }
}

/// Network Policy configuration for securing node traffic
///
/// When enabled, creates a default deny-all ingress policy with explicit allow rules
/// for peer-to-peer traffic (Validators), API access (Horizon/Soroban), and metrics.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyConfig {
    /// Enable NetworkPolicy creation (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Allow ingress from specific namespaces (by namespace name)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_namespaces: Vec<String>,

    /// Allow ingress from pods matching these labels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_pod_selector: Option<BTreeMap<String, String>>,

    /// Allow ingress from specific CIDR blocks (e.g., ["10.0.0.0/8"])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_cidrs: Vec<String>,

    /// Allow metrics scraping from monitoring namespace (default: true when enabled)
    #[serde(default = "default_true")]
    pub allow_metrics_scrape: bool,

    /// Namespace where Prometheus/monitoring stack runs (default: "monitoring")
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

// ============================================================================
// MetalLB / BGP Anycast Configuration
// ============================================================================

/// Load Balancer configuration for external access via MetalLB with BGP Anycast support
///
/// This enables global node discovery by advertising Stellar node endpoints
/// via BGP to upstream routers. Supports both L2 (ARP/NDP) and BGP modes.
///
/// # Example (BGP Anycast)
///
/// ```yaml
/// loadBalancer:
///   enabled: true
///   mode: BGP
///   addressPool: "stellar-anycast"
///   loadBalancerIP: "192.0.2.100"
///   bgp:
///     localASN: 64512
///     peers:
///       - address: "192.168.1.1"
///         asn: 64513
///         password: "bgp-secret"
///     communities:
///       - "64512:100"
///     advertisement:
///       aggregationLength: 32
///       aggregationLengthV6: 128
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancerConfig {
    /// Enable LoadBalancer service creation (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Load balancer mode: L2 or BGP (default: L2)
    #[serde(default)]
    pub mode: LoadBalancerMode,

    /// MetalLB IPAddressPool name to use for IP allocation
    /// Must match an existing IPAddressPool in the metallb-system namespace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_pool: Option<String>,

    /// Specific IP address to request from the pool
    /// If not specified, an IP will be automatically allocated from the pool
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_balancer_ip: Option<String>,

    /// External traffic policy: Cluster or Local
    /// - Cluster: distribute traffic across all nodes (default)
    /// - Local: preserve client source IP, only route to local pods
    #[serde(default)]
    pub external_traffic_policy: ExternalTrafficPolicy,

    /// BGP-specific configuration for anycast routing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgp: Option<BGPConfig>,

    /// Additional annotations to apply to the LoadBalancer Service
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,

    /// Enable health check endpoint for load balancer probes
    /// Creates an additional health check port on the service
    #[serde(default = "default_true")]
    pub health_check_enabled: bool,

    /// Port for health check probes (default: 9100)
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
    /// Layer 2 mode using ARP/NDP for local network advertisement
    /// Simpler setup, but limited to single network segment
    #[default]
    L2,
    /// BGP mode for anycast routing across multiple locations
    /// Enables global node discovery and automatic failover
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
    /// Distribute traffic across all cluster nodes (may cause extra hops)
    #[default]
    Cluster,
    /// Only route to pods on the local node (preserves source IP)
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
///
/// Enables advertising Stellar node IPs to upstream BGP routers,
/// allowing for geographic load distribution and automatic failover.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPConfig {
    /// Local Autonomous System Number (ASN) for this cluster
    /// Must be coordinated with network administrators
    pub local_asn: u32,

    /// BGP peer routers to advertise routes to
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<BGPPeer>,

    /// BGP communities to attach to advertised routes
    /// Format: "ASN:value" (e.g., "64512:100")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub communities: Vec<String>,

    /// Large BGP communities (RFC 8092) for extended tagging
    /// Format: "ASN:function:value" (e.g., "64512:1:100")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub large_communities: Vec<String>,

    /// BGP advertisement configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advertisement: Option<BGPAdvertisementConfig>,

    /// Enable BFD (Bidirectional Forwarding Detection) for fast failover
    #[serde(default)]
    pub bfd_enabled: bool,

    /// BFD profile name to use (if bfd_enabled is true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bfd_profile: Option<String>,

    /// Node selectors to limit which nodes can be BGP speakers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_selectors: Option<BTreeMap<String, String>>,
}

/// BGP peer router configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPPeer {
    /// IP address of the BGP peer router
    pub address: String,

    /// Autonomous System Number of the peer
    pub asn: u32,

    /// BGP session password (optional, stored in secret)
    /// Reference to a Kubernetes secret key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_secret_ref: Option<SecretKeyRef>,

    /// BGP port (default: 179)
    #[serde(default = "default_bgp_port")]
    pub port: u16,

    /// Hold time in seconds (default: 90)
    #[serde(default = "default_hold_time")]
    pub hold_time: u32,

    /// Keepalive time in seconds (default: 30)
    #[serde(default = "default_keepalive_time")]
    pub keepalive_time: u32,

    /// Router ID override (default: auto-detect from node IP)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_id: Option<String>,

    /// Source address for BGP session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_address: Option<String>,

    /// Enable EBGP multi-hop (required when peer is not directly connected)
    #[serde(default)]
    pub ebgp_multi_hop: bool,

    /// Enable graceful restart capability
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

/// BGP advertisement configuration for route announcement
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BGPAdvertisementConfig {
    /// IPv4 aggregation length (CIDR prefix length, 0-32)
    /// Used for route aggregation, e.g., 32 for host routes
    #[serde(default = "default_aggregation_length")]
    pub aggregation_length: u8,

    /// IPv6 aggregation length (CIDR prefix length, 0-128)
    #[serde(default = "default_aggregation_length_v6")]
    pub aggregation_length_v6: u8,

    /// Localpref value for this advertisement (affects route selection)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_pref: Option<u32>,

    /// Node selector to limit which nodes announce the route
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_selectors: Option<BTreeMap<String, String>>,
}

fn default_aggregation_length() -> u8 {
    32
}

fn default_aggregation_length_v6() -> u8 {
    128
}

/// Global node discovery configuration for Stellar network peering
///
/// Configures how this Stellar node advertises itself for peer discovery
/// across geographic regions using anycast and service mesh integration.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GlobalDiscoveryConfig {
    /// Enable global node discovery via BGP anycast
    #[serde(default)]
    pub enabled: bool,

    /// Geographic region identifier (e.g., "us-east", "eu-west", "ap-south")
    /// Used for topology-aware routing and failover
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    /// Availability zone within the region
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,

    /// Priority weight for this node (higher = more preferred)
    /// Used by BGP local preference and weighted routing
    #[serde(default = "default_priority")]
    pub priority: u32,

    /// Enable topology-aware hints for service routing
    /// Requires Kubernetes 1.23+ with topology-aware hints enabled
    #[serde(default)]
    pub topology_aware_hints: bool,

    /// Service mesh integration (Istio, Linkerd, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_mesh: Option<ServiceMeshConfig>,

    /// External DNS configuration for automatic DNS registration
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
    /// Service mesh type (istio, linkerd, consul)
    pub mesh_type: ServiceMeshType,

    /// Enable automatic sidecar injection
    #[serde(default = "default_true")]
    pub sidecar_injection: bool,

    /// mTLS mode for mesh communication
    #[serde(default)]
    pub mtls_mode: MTLSMode,

    /// Virtual service hostname for mesh routing
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
    /// No mTLS (plain text)
    Disable,
    /// Accept both mTLS and plain text
    #[default]
    Permissive,
    /// Require mTLS for all connections
    Strict,
}

/// ExternalDNS configuration for automatic DNS record management
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDNSConfig {
    /// DNS hostname to register (e.g., "stellar-node.example.com")
    pub hostname: String,

    /// TTL for DNS records in seconds (default: 300)
    #[serde(default = "default_dns_ttl")]
    pub ttl: u32,

    /// DNS provider (route53, cloudflare, google, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Additional DNS record annotations for external-dns
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

fn default_dns_ttl() -> u32 {
    300
}

// ============================================================================
// Cross-Region Disaster Recovery Configuration
// ============================================================================

/// Configuration for multi-cluster disaster recovery (DR)
///
/// Manages "hot standby" nodes in remote clusters and automated failover
/// using external DNS providers (Route53, Cloudflare).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryConfig {
    /// Whether DR is enabled for this node
    #[serde(default)]
    pub enabled: bool,

    /// Role of this cluster in the DR pairing
    pub role: DRRole,

    /// Identifier of the peer cluster/region
    pub peer_cluster_id: String,

    /// Strategy for state synchronization
    #[serde(default)]
    pub sync_strategy: DRSyncStrategy,

    /// DNS failover configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failover_dns: Option<ExternalDNSConfig>,

    /// Check interval for health of the other region (seconds)
    #[serde(default = "default_dr_check_interval")]
    pub health_check_interval: u32,
}

fn default_dr_check_interval() -> u32 {
    30
}

/// Role of a node in a DR configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DRRole {
    /// Primary node serving active traffic
    Primary,
    /// Standby node ready to take over if primary fails
    Standby,
}

/// Synchronization strategy for hot standby nodes
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DRSyncStrategy {
    /// Follow the network consensus normally
    #[default]
    Consensus,
    /// Actively track the peer node's ledger sequence
    PeerTracking,
    /// Continuous history archive sync
    ArchiveSync,
}

/// Status of the Disaster Recovery setup
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryStatus {
    /// Current effective role (may differ from spec during failover)
    pub current_role: Option<DRRole>,

    /// Health status of the peer cluster
    pub peer_health: Option<String>,

    /// Last time the peer was reachable
    pub last_peer_contact: Option<String>,

    /// Sync lag between primary and standby (in ledgers)
    pub sync_lag: Option<u64>,

    /// Whether failover is currently active
    pub failover_active: bool,
}
