//! Benchmark result collector
//!
//! After all load-generator pods have completed, this module:
//!
//! 1. Reads the structured JSON result from each pod's logs.
//! 2. Queries the target Horizon `/metrics` endpoint for ledger close time.
//! 3. Aggregates the per-pod results into a [`BenchmarkReportSpec`].
//! 4. Writes the report to a `BenchmarkReport` CR or a `ConfigMap`.
//!
//! # Pod Log Format
//!
//! The load-generator container is expected to print a single JSON object on
//! the last line of stdout that matches [`PodMetricsOutput`].  Lines that do
//! not parse as JSON are treated as informational logs and ignored.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::api::{Api, Patch, PatchParams};
use kube::{Client, Resource, ResourceExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use crate::crd::stellar_benchmark::{
    BenchmarkReport, BenchmarkReportSpec, BenchmarkReportStatus, PodResult, ResultStorage,
    StellarBenchmark,
};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Pod metrics output schema
// ---------------------------------------------------------------------------

/// JSON structure that the load-generator container prints to stdout on exit.
///
/// The operator parses the **last** line of the pod logs that is valid JSON
/// and matches this schema.  All other lines are treated as informational.
#[derive(Debug, Deserialize, Serialize)]
pub struct PodMetricsOutput {
    /// Peak TPS achieved by this pod.
    pub peak_tps: f64,
    /// Total transactions submitted.
    pub total_transactions: u64,
    /// Transactions accepted by the network.
    pub successful_transactions: u64,
    /// Transactions rejected or timed out.
    pub failed_transactions: u64,
    /// P99 API latency in milliseconds.
    pub p99_latency_ms: f64,
    /// P95 API latency in milliseconds (optional).
    #[serde(default)]
    pub p95_latency_ms: f64,
    /// P50 (median) API latency in milliseconds (optional).
    #[serde(default)]
    pub p50_latency_ms: f64,
    /// Throughput in bytes per second (optional).
    #[serde(default)]
    pub throughput_bytes_per_sec: Option<f64>,
}

// ---------------------------------------------------------------------------
// Horizon /metrics scrape
// ---------------------------------------------------------------------------

/// Scrape the Horizon `/metrics` endpoint and extract the average ledger close
/// time in milliseconds.
///
/// Returns `None` if the endpoint is unreachable or the metric is absent.
pub async fn scrape_ledger_close_time_ms(target_endpoint: &str) -> Option<f64> {
    let metrics_url = format!("{}/metrics", target_endpoint.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    let body = client
        .get(&metrics_url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    // Parse Prometheus text format — look for `stellar_core_ledger_close_duration_seconds`
    // or `horizon_ledger_close_duration_seconds`.
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line.contains("ledger_close_duration_seconds") || line.contains("ledger_close_time_ms") {
            // Extract the numeric value after the last space.
            if let Some(value_str) = line.split_whitespace().last() {
                if let Ok(seconds) = value_str.parse::<f64>() {
                    // Convert seconds → milliseconds if the metric is in seconds.
                    let ms = if line.contains("_seconds") {
                        seconds * 1000.0
                    } else {
                        seconds
                    };
                    debug!(
                        metric_line = %line,
                        value_ms = ms,
                        "Scraped ledger close time"
                    );
                    return Some(ms);
                }
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Log parsing
// ---------------------------------------------------------------------------

/// Parse the structured JSON result from a pod's log output.
///
/// Scans lines in reverse order and returns the first line that deserialises
/// as [`PodMetricsOutput`].  Returns `None` if no valid JSON result is found.
pub fn parse_pod_metrics(logs: &str) -> Option<PodMetricsOutput> {
    for line in logs.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }
        if let Ok(metrics) = serde_json::from_str::<PodMetricsOutput>(trimmed) {
            return Some(metrics);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Result collection
// ---------------------------------------------------------------------------

/// Collect results from all completed load-generator pods.
///
/// For each pod:
/// 1. Fetch the pod logs via the Kubernetes API.
/// 2. Parse the structured JSON result from the last JSON line.
/// 3. Build a [`PodResult`] (with a fallback for pods that produced no JSON).
#[instrument(skip(client), fields(benchmark = %benchmark.name_any()))]
pub async fn collect_pod_results(
    client: &Client,
    benchmark: &StellarBenchmark,
    pod_names: &[String],
) -> Result<Vec<PodResult>> {
    let namespace = benchmark
        .namespace()
        .unwrap_or_else(|| "default".to_string());
    let pods_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    let mut results = Vec::with_capacity(pod_names.len());

    for pod_name in pod_names {
        let logs = match pods_api
            .logs(
                pod_name,
                &kube::api::LogParams {
                    container: Some("load-generator".to_string()),
                    tail_lines: Some(200),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(l) => l,
            Err(e) => {
                warn!(pod = %pod_name, error = %e, "Failed to fetch pod logs; using zero metrics");
                String::new()
            }
        };

        // Determine exit code from pod status.
        let exit_code = pods_api
            .get(pod_name)
            .await
            .ok()
            .and_then(|p| p.status)
            .and_then(|s| s.container_statuses)
            .and_then(|cs| cs.into_iter().find(|c| c.name == "load-generator"))
            .and_then(|c| c.state)
            .and_then(|s| s.terminated)
            .map(|t| t.exit_code)
            .unwrap_or(-1);

        // Truncate logs to 4 KiB for storage in the CRD.
        let truncated_logs = if logs.len() > 4096 {
            format!("...(truncated)...\n{}", &logs[logs.len() - 4096..])
        } else {
            logs.clone()
        };

        let metrics = parse_pod_metrics(&logs);

        let result = match metrics {
            Some(m) => {
                info!(
                    pod = %pod_name,
                    peak_tps = m.peak_tps,
                    total_tx = m.total_transactions,
                    p99_ms = m.p99_latency_ms,
                    "Collected pod metrics"
                );
                PodResult {
                    pod_name: pod_name.clone(),
                    peak_tps: m.peak_tps,
                    total_transactions: m.total_transactions,
                    successful_transactions: m.successful_transactions,
                    failed_transactions: m.failed_transactions,
                    p99_latency_ms: m.p99_latency_ms,
                    exit_code,
                    logs: Some(truncated_logs),
                }
            }
            None => {
                warn!(
                    pod = %pod_name,
                    "No structured JSON metrics found in pod logs; recording zeros"
                );
                PodResult {
                    pod_name: pod_name.clone(),
                    peak_tps: 0.0,
                    total_transactions: 0,
                    successful_transactions: 0,
                    failed_transactions: 0,
                    p99_latency_ms: 0.0,
                    exit_code,
                    logs: Some(truncated_logs),
                }
            }
        };

        results.push(result);
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Report storage
// ---------------------------------------------------------------------------

/// Write the benchmark results to a `BenchmarkReport` CR.
///
/// If a report with the same name already exists it is replaced via a
/// server-side apply patch.
#[instrument(skip(client, report_spec), fields(benchmark = %benchmark_name))]
pub async fn write_benchmark_report(
    client: &Client,
    namespace: &str,
    benchmark_name: &str,
    benchmark: &StellarBenchmark,
    report_spec: BenchmarkReportSpec,
) -> Result<String> {
    let report_name = format!("{}-report", benchmark_name);
    let api: Api<BenchmarkReport> = Api::namespaced(client.clone(), namespace);

    let owner_ref = OwnerReference {
        api_version: StellarBenchmark::api_version(&()).to_string(),
        kind: StellarBenchmark::kind(&()).to_string(),
        name: benchmark.name_any(),
        uid: benchmark.metadata.uid.clone().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    };

    let report = BenchmarkReport {
        metadata: ObjectMeta {
            name: Some(report_name.clone()),
            namespace: Some(namespace.to_string()),
            owner_references: Some(vec![owner_ref]),
            labels: Some(BTreeMap::from([(
                "stellar.org/benchmark".to_string(),
                benchmark_name.to_string(),
            )])),
            ..Default::default()
        },
        spec: report_spec,
        status: Some(BenchmarkReportStatus { ready: true }),
    };

    let patch_params = PatchParams::apply("stellar-operator").force();
    let patch_data = serde_json::to_value(&report).map_err(Error::SerializationError)?;

    api.patch(&report_name, &patch_params, &Patch::Apply(&patch_data))
        .await
        .map_err(Error::KubeError)?;

    info!(report = %report_name, "BenchmarkReport written");
    Ok(report_name)
}

/// Write the benchmark results to a `ConfigMap` (fallback storage).
#[instrument(skip(client, report_spec), fields(benchmark = %benchmark_name))]
pub async fn write_benchmark_configmap(
    client: &Client,
    namespace: &str,
    benchmark_name: &str,
    benchmark: &StellarBenchmark,
    report_spec: &BenchmarkReportSpec,
) -> Result<String> {
    let cm_name = format!("{}-report", benchmark_name);
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);

    let owner_ref = OwnerReference {
        api_version: StellarBenchmark::api_version(&()).to_string(),
        kind: StellarBenchmark::kind(&()).to_string(),
        name: benchmark.name_any(),
        uid: benchmark.metadata.uid.clone().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    };

    let report_json =
        serde_json::to_string_pretty(report_spec).map_err(Error::SerializationError)?;

    let metrics = &report_spec.metrics;
    let summary = format!(
        "peak_tps={:.2} avg_ledger_close_ms={:.2} p99_api_latency_ms={:.2} total_tx={} success={} failed={} error_rate={:.2}%",
        metrics.peak_tps,
        metrics.avg_ledger_close_ms,
        metrics.p99_api_latency_ms,
        metrics.total_transactions,
        metrics.successful_transactions,
        metrics.failed_transactions,
        metrics.error_rate_pct,
    );

    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(cm_name.clone()),
            namespace: Some(namespace.to_string()),
            owner_references: Some(vec![owner_ref]),
            labels: Some(BTreeMap::from([(
                "stellar.org/benchmark".to_string(),
                benchmark_name.to_string(),
            )])),
            ..Default::default()
        },
        data: Some(BTreeMap::from([
            ("report.json".to_string(), report_json),
            ("summary".to_string(), summary),
            ("completed_at".to_string(), report_spec.completed_at.clone()),
        ])),
        ..Default::default()
    };

    let patch_params = PatchParams::apply("stellar-operator").force();
    let patch_data = serde_json::to_value(&cm).map_err(Error::SerializationError)?;

    api.patch(&cm_name, &patch_params, &Patch::Apply(&patch_data))
        .await
        .map_err(Error::KubeError)?;

    info!(configmap = %cm_name, "Benchmark ConfigMap written");
    Ok(cm_name)
}

/// Dispatch to the correct storage backend based on the benchmark spec.
pub async fn store_results(
    client: &Client,
    namespace: &str,
    benchmark_name: &str,
    benchmark: &StellarBenchmark,
    report_spec: BenchmarkReportSpec,
) -> Result<String> {
    match benchmark.spec.result_storage {
        ResultStorage::BenchmarkReport => {
            write_benchmark_report(client, namespace, benchmark_name, benchmark, report_spec).await
        }
        ResultStorage::ConfigMap => {
            write_benchmark_configmap(client, namespace, benchmark_name, benchmark, &report_spec)
                .await
        }
    }
}
