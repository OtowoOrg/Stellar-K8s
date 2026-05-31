use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Cluster-level traffic shaping policy used by the operator.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "TrafficPolicy",
    namespaced,
    status = "TrafficPolicyStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct TrafficPolicySpec {
    /// Optional StellarNode name this policy targets. If omitted, policy is global.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_node: Option<String>,

    #[serde(default)]
    pub adaptive_rate_limit: AdaptiveRateLimitPolicy,

    #[serde(default)]
    pub token_bucket: TokenBucketPolicy,

    #[serde(default)]
    pub leaky_bucket: LeakyBucketPolicy,

    #[serde(default)]
    pub qos_classes: Vec<QosClassPolicy>,

    #[serde(default)]
    pub priority_rules: Vec<PriorityRule>,

    #[serde(default)]
    pub circuit_breaker: CircuitBreakerPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveRateLimitPolicy {
    #[serde(default = "default_base_rps")]
    pub base_rps: u32,

    #[serde(default = "default_min_rps")]
    pub min_rps: u32,

    #[serde(default = "default_max_rps")]
    pub max_rps: u32,

    /// Target load where scaling shifts from boost to shedding.
    #[serde(default = "default_target_load")]
    pub target_load: f64,

    /// Additional throughput scaling when system load is below target.
    #[serde(default = "default_boost_factor")]
    pub boost_factor: f64,

    /// Throughput shedding when system load is above target.
    #[serde(default = "default_shed_factor")]
    pub shed_factor: f64,
}

impl Default for AdaptiveRateLimitPolicy {
    fn default() -> Self {
        Self {
            base_rps: default_base_rps(),
            min_rps: default_min_rps(),
            max_rps: default_max_rps(),
            target_load: default_target_load(),
            boost_factor: default_boost_factor(),
            shed_factor: default_shed_factor(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenBucketPolicy {
    #[serde(default = "default_token_capacity")]
    pub capacity: u32,

    #[serde(default = "default_token_refill_rps")]
    pub refill_rps: u32,
}

impl Default for TokenBucketPolicy {
    fn default() -> Self {
        Self {
            capacity: default_token_capacity(),
            refill_rps: default_token_refill_rps(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeakyBucketPolicy {
    #[serde(default = "default_leaky_capacity")]
    pub capacity: u32,

    #[serde(default = "default_leaky_rate_rps")]
    pub leak_rate_rps: u32,
}

impl Default for LeakyBucketPolicy {
    fn default() -> Self {
        Self {
            capacity: default_leaky_capacity(),
            leak_rate_rps: default_leaky_rate_rps(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QosClassPolicy {
    pub class_name: String,
    pub priority: TrafficPriorityClass,

    /// Minimum guaranteed percentage of throughput for this class.
    #[serde(default = "default_min_share_percent")]
    pub min_share_percent: u8,

    /// Relative class weight for best-effort scheduling.
    #[serde(default = "default_weight")]
    pub weight: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrafficPriorityClass {
    Critical,
    High,
    Normal,
    Low,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PriorityRule {
    pub name: String,
    pub class: TrafficPriorityClass,

    /// Optional path prefix for classifying requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,

    /// Optional method filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CircuitBreakerPolicy {
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,

    #[serde(default = "default_success_threshold")]
    pub success_threshold: u32,

    #[serde(default = "default_open_window_secs")]
    pub open_window_secs: u64,
}

impl Default for CircuitBreakerPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            success_threshold: default_success_threshold(),
            open_window_secs: default_open_window_secs(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficPolicyStatus {
    pub observed_generation: Option<i64>,
    pub effective_rps: Option<u32>,
    pub current_load: Option<f64>,
    pub dropped_requests: u64,
    pub circuit_breaker_state: Option<String>,
    pub last_transition_time: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_base_rps() -> u32 {
    500
}

fn default_min_rps() -> u32 {
    50
}

fn default_max_rps() -> u32 {
    5000
}

fn default_target_load() -> f64 {
    0.70
}

fn default_boost_factor() -> f64 {
    0.5
}

fn default_shed_factor() -> f64 {
    1.25
}

fn default_token_capacity() -> u32 {
    1000
}

fn default_token_refill_rps() -> u32 {
    500
}

fn default_leaky_capacity() -> u32 {
    2000
}

fn default_leaky_rate_rps() -> u32 {
    500
}

fn default_min_share_percent() -> u8 {
    10
}

fn default_weight() -> u32 {
    100
}

fn default_failure_threshold() -> u32 {
    5
}

fn default_success_threshold() -> u32 {
    2
}

fn default_open_window_secs() -> u64 {
    30
}
