//! StellarPerformance Custom Resource Definition
//!
//! `StellarPerformance` is the entry point of the Performance Optimization
//! Framework (epic #868). It lets operators declare **performance budgets**
//! (SLOs) for a Stellar workload and have the operator continuously evaluate
//! observed metrics against those budgets, while detecting **regressions**
//! relative to a rolling baseline.
//!
//! This CRD is the foundation the remaining epic capabilities build on
//! (continuous benchmarking, profiling, query optimization, multi-tier
//! caching, automated resource tuning, and load testing). The reconcile logic
//! for budget evaluation and regression detection lives in
//! [`crate::controller::performance`].
//!
//! # Lifecycle
//!
//! ```text
//! StellarPerformance (Pending)
//!   → operator samples target metrics every evaluationIntervalSeconds
//!   → evaluate against budgets + compare to rolling baseline
//!   → status.phase = WithinBudget | BudgetExceeded | Regressed
//! ```

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

/// Spec for a `StellarPerformance` resource.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarPerformance",
    namespaced,
    status = "StellarPerformanceStatus",
    shortname = "sperf",
    printcolumn = r#"{"name":"Target","type":"string","jsonPath":".spec.targetRef"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Regressed","type":"boolean","jsonPath":".status.regressionDetected"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarPerformanceSpec {
    /// Name of the StellarNode (or service) whose performance is governed.
    pub target_ref: String,

    /// Performance budgets / SLOs the target is expected to satisfy.
    pub budgets: PerformanceBudgets,

    /// Policy for detecting regressions against the rolling baseline.
    #[serde(default)]
    pub regression: RegressionPolicy,

    /// How often (seconds) the operator samples and evaluates the target.
    ///
    /// Default: 60
    #[serde(default = "default_eval_interval")]
    pub evaluation_interval_seconds: u32,
}

fn default_eval_interval() -> u32 {
    60
}

/// Performance budgets expressed as service-level objectives.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceBudgets {
    /// Maximum acceptable 95th-percentile API latency, in milliseconds.
    pub max_p95_latency_ms: f64,

    /// Minimum acceptable sustained throughput, in transactions per second.
    pub min_throughput_tps: f64,

    /// Maximum acceptable error rate, as a percentage (0–100).
    pub max_error_rate_pct: f64,
}

/// Policy controlling regression detection against the rolling baseline.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegressionPolicy {
    /// Flag a regression when p95 latency rises more than this percentage
    /// above the baseline. Default: 10.0 (%).
    #[serde(default = "default_latency_regression_pct")]
    pub max_latency_increase_pct: f64,

    /// Flag a regression when throughput falls more than this percentage
    /// below the baseline. Default: 10.0 (%).
    #[serde(default = "default_throughput_regression_pct")]
    pub max_throughput_decrease_pct: f64,
}

fn default_latency_regression_pct() -> f64 {
    10.0
}

fn default_throughput_regression_pct() -> f64 {
    10.0
}

impl Default for RegressionPolicy {
    fn default() -> Self {
        Self {
            max_latency_increase_pct: default_latency_regression_pct(),
            max_throughput_decrease_pct: default_throughput_regression_pct(),
        }
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Status for a `StellarPerformance` resource.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarPerformanceStatus {
    /// High-level phase derived from budget compliance and regression checks.
    #[serde(default)]
    pub phase: PerformancePhase,

    /// Human-readable message describing the current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// RFC 3339 timestamp of the most recent evaluation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluation: Option<String>,

    /// The most recently observed performance sample.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<PerformanceSample>,

    /// Rolling baseline the current sample is compared against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<PerformanceSample>,

    /// Per-SLO budget results from the latest evaluation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub budget_compliance: Vec<BudgetResult>,

    /// Whether a regression was detected in the latest evaluation.
    #[serde(default)]
    pub regression_detected: bool,

    /// Kubernetes-style conditions for detailed status tracking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<crate::crd::Condition>,
}

/// A single measured set of performance metrics.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSample {
    /// Observed 95th-percentile API latency, in milliseconds.
    pub p95_latency_ms: f64,
    /// Observed sustained throughput, in transactions per second.
    pub throughput_tps: f64,
    /// Observed error rate, as a percentage (0–100).
    pub error_rate_pct: f64,
}

/// Result of evaluating a single budget/SLO.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BudgetResult {
    /// Which SLO this result describes.
    pub metric: String,
    /// Whether the observed value is within budget.
    pub within_budget: bool,
    /// The observed value.
    pub observed: f64,
    /// The budgeted threshold.
    pub budget: f64,
}

/// High-level phase of a `StellarPerformance` evaluation.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum PerformancePhase {
    /// Resource created, not yet evaluated.
    #[default]
    Pending,
    /// All budgets met and no regression detected.
    WithinBudget,
    /// At least one budget violated.
    BudgetExceeded,
    /// A regression was detected relative to the baseline.
    Regressed,
}

impl std::fmt::Display for PerformancePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PerformancePhase::Pending => write!(f, "Pending"),
            PerformancePhase::WithinBudget => write!(f, "WithinBudget"),
            PerformancePhase::BudgetExceeded => write!(f, "BudgetExceeded"),
            PerformancePhase::Regressed => write!(f, "Regressed"),
        }
    }
}
