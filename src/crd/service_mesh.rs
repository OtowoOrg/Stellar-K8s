//! Service Mesh Configuration Types
//!
//! Provides types for configuring service mesh integration with Istio and Linkerd
//! for advanced traffic control, mTLS enforcement, circuit breaking, and retries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Service Mesh configuration for Istio/Linkerd integration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMeshConfig {
    /// Enable sidecar injection for this node
    #[serde(default = "default_true")]
    pub sidecar_injection: bool,

    /// Istio-specific configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub istio: Option<IstioMeshConfig>,

    /// Linkerd-specific configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkerd: Option<LinkerdMeshConfig>,
}

fn default_true() -> bool {
    true
}

/// Istio-specific mesh configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IstioMeshConfig {
    /// mTLS mode (STRICT or PERMISSIVE)
    #[serde(default)]
    pub mtls_mode: MtlsMode,

    /// Circuit breaker configuration for outlier detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_breaker: Option<CircuitBreakerConfig>,

    /// Retry policy for failed requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retries: Option<RetryConfig>,

    /// VirtualService timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
}

fn default_timeout() -> u32 {
    30
}

/// Linkerd-specific mesh configuration
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkerdMeshConfig {
    /// Enable automatic mTLS
    #[serde(default = "default_true")]
    pub auto_mtls: bool,

    /// Policy mode (deny, audit, allow)
    #[serde(default = "default_linkerd_policy_mode")]
    pub policy_mode: String,
}

fn default_linkerd_policy_mode() -> String {
    "allow".to_string()
}

/// mTLS mode enumeration for Istio
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum MtlsMode {
    /// All traffic must be encrypted with mTLS
    #[default]
    Strict,
    /// Both encrypted and unencrypted traffic accepted (migration mode)
    Permissive,
}

impl std::fmt::Display for MtlsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MtlsMode::Strict => write!(f, "STRICT"),
            MtlsMode::Permissive => write!(f, "PERMISSIVE"),
        }
    }
}

/// Circuit breaker configuration for Istio DestinationRule
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CircuitBreakerConfig {
    /// Number of consecutive errors before opening circuit
    #[serde(default = "default_consecutive_errors")]
    pub consecutive_errors: u32,

    /// Time window in seconds for counting errors
    #[serde(default = "default_time_window")]
    pub time_window_secs: u32,

    /// Minimum request volume before applying circuit breaking
    #[serde(default = "default_min_request_volume")]
    pub min_request_volume: u32,
}

fn default_consecutive_errors() -> u32 {
    5
}

fn default_time_window() -> u32 {
    30
}

fn default_min_request_volume() -> u32 {
    10
}

/// Retry configuration for Istio VirtualService
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetryConfig {
    /// Maximum number of retries
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Backoff duration in milliseconds
    #[serde(default = "default_backoff")]
    pub backoff_ms: u32,

    /// Retryable status codes (e.g., 503, 504)
    #[serde(default)]
    pub retryable_status_codes: Vec<u32>,
}

fn default_max_retries() -> u32 {
    3
}

fn default_backoff() -> u32 {
    25
}
