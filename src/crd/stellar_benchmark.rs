//! StellarBenchmark and BenchmarkReport Custom Resource Definitions
//!
//! `StellarBenchmark` is a namespaced CRD that operators create to trigger a
//! performance test run against their Stellar infrastructure.  The operator
//! reconciles it by spinning up ephemeral load-generator pods, collecting
//! metrics (Peak TPS, Average Ledger Close Time, P99 API Latency), and writing
//! the results into a companion `BenchmarkReport` resource (or a ConfigMap as
//! a fallback).
//!
//! # Lifecycle
//!
//! ```text
//! StellarBenchmark (Pending)
//!   → operator creates load-generator Job/Pods
//!   → StellarBenchmark (Running)
//!   → pods complete, operator collects metrics
//!   → BenchmarkReport created / updated
//!   → StellarBenchmark (Completed | Failed)
//! ```

use chrono::Utc;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// StellarBenchmark CRD
// ---------------------------------------------------------------------------

/// Spec for a `StellarBenchmark` resource.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarBenchmark",
    namespaced,
    status = "StellarBenchmarkStatus",
    shortname = "sbench",
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Target","type":"string","jsonPath":".spec.targetEndpoint"}"#,
    printcolumn = r#"{"name":"Duration","type":"integer","jsonPath":".spec.durationSeconds"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarBenchmarkSpec {
    /// HTTP(S) endpoint of the Horizon or Soroban RPC node under test.
    ///
    /// Example: `http://my-horizon.stellar-system.svc.cluster.local:8000`
    pub target_endpoint: String,

    /// How long (in seconds) the load-generator pods should run.
    ///
    /// Default: 60
    #[serde(default = "default_duration")]
    pub duration_seconds: u32,

    /// Target transactions per second to attempt during the test.
    ///
    /// The load generator will ramp up to this rate and sustain it.
    /// Default: 100
    #[serde(default = "default_target_tps")]
    pub target_tps: u32,

    /// Number of concurrent load-generator pods to spin up.
    ///
    /// Each pod contributes `target_tps / concurrency` TPS.
    /// Default: 1
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,

    /// Stellar network passphrase used when constructing test transactions.
    ///
    /// Defaults to the Testnet passphrase.
    #[serde(default = "default_network_passphrase")]
    pub network_passphrase: String,

    /// Container image for the load-generator pods.
    ///
    /// Must expose a `/metrics` endpoint (Prometheus format) and accept the
    /// environment variables `TARGET_ENDPOINT`, `DURATION_SECONDS`,
    /// `TARGET_TPS`, and `NETWORK_PASSPHRASE`.
    ///
    /// Default: `stellar/load-generator:latest`
    #[serde(default = "default_load_generator_image")]
    pub load_generator_image: String,

    /// Resource requirements for each load-generator pod.
    #[serde(default)]
    pub resources: BenchmarkResourceRequirements,

    /// Optional: name of a Kubernetes Secret whose keys are injected as
    /// environment variables into the load-generator pods (e.g. signing keys).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,

    /// Optional: additional environment variables for the load-generator pods.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_env: Vec<EnvVar>,

    /// Where to store the benchmark results.
    ///
    /// Default: `BenchmarkReport` (creates a `BenchmarkReport` CR).
    /// Use `ConfigMap` to store results in a plain ConfigMap instead.
    #[serde(default)]
    pub result_storage: ResultStorage,

    /// Optional: node selector for the load-generator pods.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_selector: BTreeMap<String, String>,

    /// Optional: service account name for the load-generator pods.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account_name: Option<String>,

    /// Optional: image pull policy for the load-generator image.
    #[serde(default = "default_pull_policy")]
    pub image_pull_policy: String,

    /// Optional: tolerations for the load-generator pods.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerations: Vec<Toleration>,
}

fn default_duration() -> u32 {
    60
}
fn default_target_tps() -> u32 {
    100
}
fn default_concurrency() -> u32 {
    1
}
fn default_network_passphrase() -> String {
    "Test SDF Network ; September 2015".to_string()
}
fn default_load_generator_image() -> String {
    "stellar/load-generator:latest".to_string()
}
fn default_pull_policy() -> String {
    "IfNotPresent".to_string()
}

/// Simplified resource requirements for benchmark pods.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkResourceRequirements {
    pub cpu_request: String,
    pub memory_request: String,
    pub cpu_limit: String,
    pub memory_limit: String,
}

impl Default for BenchmarkResourceRequirements {
    fn default() -> Self {
        Self {
            cpu_request: "250m".to_string(),
            memory_request: "256Mi".to_string(),
            cpu_limit: "1".to_string(),
            memory_limit: "512Mi".to_string(),
        }
    }
}

/// A simple key/value environment variable.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Where to persist benchmark results.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum ResultStorage {
    /// Create / update a `BenchmarkReport` custom resource (recommended).
    #[default]
    BenchmarkReport,
    /// Create / update a plain `ConfigMap` (no CRD required).
    ConfigMap,
}

/// A Kubernetes toleration for pod scheduling.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Toleration {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

// ---------------------------------------------------------------------------
// StellarBenchmarkStatus
// ---------------------------------------------------------------------------

/// Current status of a `StellarBenchmark` run.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarBenchmarkStatus {
    /// High-level phase of the benchmark run.
    #[serde(default)]
    pub phase: BenchmarkPhase,

    /// Human-readable message describing the current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// RFC 3339 timestamp when the benchmark run started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,

    /// RFC 3339 timestamp when the benchmark run completed (or failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,

    /// Name of the `BenchmarkReport` or `ConfigMap` that holds the results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_ref: Option<String>,

    /// Names of the load-generator pods that were created.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pod_names: Vec<String>,

    /// Inline summary of key metrics (populated once the run completes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<BenchmarkSummary>,

    /// Kubernetes-style conditions for detailed status tracking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<crate::crd::Condition>,
}

/// High-level phase of a benchmark run.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum BenchmarkPhase {
    /// Resource created, not yet acted on.
    #[default]
    Pending,
    /// Load-generator pods are running.
    Running,
    /// Pods finished, collecting / aggregating results.
    Collecting,
    /// Results stored, benchmark complete.
    Completed,
    /// An unrecoverable error occurred.
    Failed,
}

impl std::fmt::Display for BenchmarkPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkPhase::Pending => write!(f, "Pending"),
            BenchmarkPhase::Running => write!(f, "Running"),
            BenchmarkPhase::Collecting => write!(f, "Collecting"),
            BenchmarkPhase::Completed => write!(f, "Completed"),
            BenchmarkPhase::Failed => write!(f, "Failed"),
        }
    }
}

/// Inline summary of the most important benchmark metrics.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkSummary {
    /// Peak transactions per second observed during the run.
    pub peak_tps: f64,
    /// Average ledger close time in milliseconds.
    pub avg_ledger_close_ms: f64,
    /// 99th-percentile API latency in milliseconds.
    pub p99_api_latency_ms: f64,
    /// Total number of transactions submitted.
    pub total_transactions: u64,
    /// Total number of transactions that succeeded.
    pub successful_transactions: u64,
    /// Total number of transactions that failed.
    pub failed_transactions: u64,
    /// Error rate as a percentage (0–100).
    pub error_rate_pct: f64,
}

// ---------------------------------------------------------------------------
// BenchmarkReport CRD
// ---------------------------------------------------------------------------

/// Spec for a `BenchmarkReport` resource.
///
/// `BenchmarkReport` is a read-only resource written by the operator after a
/// `StellarBenchmark` run completes.  Operators should treat it as immutable;
/// the operator will overwrite it on the next run with the same name.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "BenchmarkReport",
    namespaced,
    status = "BenchmarkReportStatus",
    shortname = "br",
    printcolumn = r#"{"name":"Benchmark","type":"string","jsonPath":".spec.benchmarkRef"}"#,
    printcolumn = r#"{"name":"PeakTPS","type":"number","jsonPath":".spec.metrics.peakTps"}"#,
    printcolumn = r#"{"name":"AvgLedgerMs","type":"number","jsonPath":".spec.metrics.avgLedgerCloseMs"}"#,
    printcolumn = r#"{"name":"P99LatencyMs","type":"number","jsonPath":".spec.metrics.p99ApiLatencyMs"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReportSpec {
    /// Name of the `StellarBenchmark` that produced this report.
    pub benchmark_ref: String,

    /// RFC 3339 timestamp when the benchmark run started.
    pub started_at: String,

    /// RFC 3339 timestamp when the benchmark run completed.
    pub completed_at: String,

    /// Target endpoint that was tested.
    pub target_endpoint: String,

    /// Configuration snapshot from the `StellarBenchmark` spec.
    pub config: BenchmarkConfig,

    /// Aggregated performance metrics.
    pub metrics: BenchmarkMetrics,

    /// Per-pod raw results (one entry per load-generator pod).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pod_results: Vec<PodResult>,
}

/// Snapshot of the benchmark configuration used for this run.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkConfig {
    pub duration_seconds: u32,
    pub target_tps: u32,
    pub concurrency: u32,
    pub network_passphrase: String,
    pub load_generator_image: String,
}

/// Aggregated performance metrics for a benchmark run.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkMetrics {
    /// Peak transactions per second observed across all pods.
    pub peak_tps: f64,

    /// Average ledger close time in milliseconds (sampled from the target node).
    pub avg_ledger_close_ms: f64,

    /// 99th-percentile API response latency in milliseconds.
    pub p99_api_latency_ms: f64,

    /// 50th-percentile (median) API response latency in milliseconds.
    pub p50_api_latency_ms: f64,

    /// 95th-percentile API response latency in milliseconds.
    pub p95_api_latency_ms: f64,

    /// Total transactions submitted across all pods.
    pub total_transactions: u64,

    /// Transactions that were accepted by the network.
    pub successful_transactions: u64,

    /// Transactions that were rejected or timed out.
    pub failed_transactions: u64,

    /// Error rate as a percentage (0–100).
    pub error_rate_pct: f64,

    /// Throughput in bytes per second (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_bytes_per_sec: Option<f64>,
}

/// Raw results from a single load-generator pod.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodResult {
    /// Name of the pod.
    pub pod_name: String,

    /// Peak TPS reported by this pod.
    pub peak_tps: f64,

    /// Total transactions submitted by this pod.
    pub total_transactions: u64,

    /// Successful transactions from this pod.
    pub successful_transactions: u64,

    /// Failed transactions from this pod.
    pub failed_transactions: u64,

    /// P99 latency in milliseconds from this pod.
    pub p99_latency_ms: f64,

    /// Exit code of the load-generator container (0 = success).
    pub exit_code: i32,

    /// Truncated stdout/stderr from the pod (last 4 KiB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<String>,
}

/// Status subresource for `BenchmarkReport` (minimal — the spec is the source of truth).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReportStatus {
    /// Whether the report has been fully written.
    #[serde(default)]
    pub ready: bool,
}

// ---------------------------------------------------------------------------
// Helper constructors
// ---------------------------------------------------------------------------

impl BenchmarkReportSpec {
    /// Build a `BenchmarkReportSpec` from aggregated pod results.
    pub fn from_pod_results(
        benchmark_name: &str,
        spec: &StellarBenchmarkSpec,
        started_at: &str,
        pod_results: Vec<PodResult>,
    ) -> Self {
        let completed_at = Utc::now().to_rfc3339();

        // Aggregate metrics across pods
        let total_transactions: u64 = pod_results.iter().map(|p| p.total_transactions).sum();
        let successful_transactions: u64 =
            pod_results.iter().map(|p| p.successful_transactions).sum();
        let failed_transactions: u64 = pod_results.iter().map(|p| p.failed_transactions).sum();

        let peak_tps = pod_results
            .iter()
            .map(|p| p.peak_tps)
            .fold(0.0_f64, f64::max);

        // P99 latency: take the max across pods (worst-case for the operator)
        let p99_api_latency_ms = pod_results
            .iter()
            .map(|p| p.p99_latency_ms)
            .fold(0.0_f64, f64::max);

        // Placeholder for ledger close time — populated by the collector from
        // the Horizon /metrics endpoint.
        let avg_ledger_close_ms = 0.0;

        let error_rate_pct = if total_transactions > 0 {
            (failed_transactions as f64 / total_transactions as f64) * 100.0
        } else {
            0.0
        };

        Self {
            benchmark_ref: benchmark_name.to_string(),
            started_at: started_at.to_string(),
            completed_at,
            target_endpoint: spec.target_endpoint.clone(),
            config: BenchmarkConfig {
                duration_seconds: spec.duration_seconds,
                target_tps: spec.target_tps,
                concurrency: spec.concurrency,
                network_passphrase: spec.network_passphrase.clone(),
                load_generator_image: spec.load_generator_image.clone(),
            },
            metrics: BenchmarkMetrics {
                peak_tps,
                avg_ledger_close_ms,
                p99_api_latency_ms,
                p50_api_latency_ms: 0.0, // populated by collector
                p95_api_latency_ms: 0.0, // populated by collector
                total_transactions,
                successful_transactions,
                failed_transactions,
                error_rate_pct,
                throughput_bytes_per_sec: None,
            },
            pod_results,
        }
    }
}
