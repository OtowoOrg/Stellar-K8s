//! `StellarBenchmark` reconciler
//!
//! This is the main controller loop for `StellarBenchmark` resources.
//!
//! # State machine
//!
//! ```text
//! Pending
//!   → create load-generator pods
//!   → patch status to Running
//! Running
//!   → poll pod phases
//!   → when all pods Succeeded/Failed → patch status to Collecting
//! Collecting
//!   → collect pod logs & metrics
//!   → scrape Horizon /metrics for ledger close time
//!   → write BenchmarkReport or ConfigMap
//!   → patch status to Completed (or Failed on error)
//! Completed / Failed
//!   → no-op (terminal states)
//! ```
//!
//! The controller requeueing strategy:
//! - `Pending` → requeue after 2 s (pods need to be created first)
//! - `Running` → requeue after 10 s (poll pod completion)
//! - `Collecting` → requeue after 5 s (collection is fast)
//! - `Completed` / `Failed` → requeue after 5 min (nothing to do)

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config;
use kube::{Client, ResourceExt};
use tracing::{debug, error, info, instrument, warn};

use crate::controller::benchmark::collector::{
    collect_pod_results, scrape_ledger_close_time_ms, store_results,
};
use crate::controller::benchmark::pod_builder::{build_load_generator_pod_with_secret, pod_name};
use crate::controller::conditions::{
    set_condition, CONDITION_STATUS_FALSE, CONDITION_STATUS_TRUE, CONDITION_TYPE_READY,
};
use crate::crd::stellar_benchmark::{
    BenchmarkPhase, BenchmarkReportSpec, BenchmarkSummary, StellarBenchmark, StellarBenchmarkStatus,
};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Shared controller state
// ---------------------------------------------------------------------------

/// Shared state passed to every reconcile invocation.
pub struct BenchmarkControllerState {
    pub client: Client,
    pub is_leader: Arc<std::sync::atomic::AtomicBool>,
}

// ---------------------------------------------------------------------------
// Controller entry point
// ---------------------------------------------------------------------------

/// Start the `StellarBenchmark` controller loop.
///
/// This function blocks until the controller is shut down (e.g. via SIGTERM).
pub async fn run_benchmark_controller(
    client: Client,
    is_leader: Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let state = Arc::new(BenchmarkControllerState {
        client: client.clone(),
        is_leader,
    });

    let benchmarks: Api<StellarBenchmark> = Api::all(client.clone());

    info!("Starting StellarBenchmark controller");

    Controller::new(benchmarks, Config::default())
        // Watch load-generator pods so we get notified when they complete.
        .owns::<Pod>(Api::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, state)
        .for_each(|_res| async {})
        .await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Error policy
// ---------------------------------------------------------------------------

fn error_policy(
    _obj: Arc<StellarBenchmark>,
    err: &Error,
    _ctx: Arc<BenchmarkControllerState>,
) -> Action {
    warn!(error = %err, "StellarBenchmark reconcile error; retrying in 30s");
    Action::requeue(Duration::from_secs(30))
}

// ---------------------------------------------------------------------------
// Main reconcile function
// ---------------------------------------------------------------------------

#[instrument(
    skip(benchmark, ctx),
    fields(
        benchmark = %benchmark.name_any(),
        namespace = ?benchmark.namespace(),
        phase = ?benchmark.status.as_ref().map(|s| s.phase.to_string())
    )
)]
async fn reconcile(
    benchmark: Arc<StellarBenchmark>,
    ctx: Arc<BenchmarkControllerState>,
) -> Result<Action> {
    if !ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let namespace = benchmark
        .namespace()
        .unwrap_or_else(|| "default".to_string());
    let name = benchmark.name_any();
    let client = &ctx.client;

    let phase = benchmark
        .status
        .as_ref()
        .map(|s| s.phase.clone())
        .unwrap_or(BenchmarkPhase::Pending);

    info!(benchmark = %name, phase = %phase, "Reconciling StellarBenchmark");

    match phase {
        BenchmarkPhase::Pending => handle_pending(client, &benchmark, &namespace, &name).await,
        BenchmarkPhase::Running => handle_running(client, &benchmark, &namespace, &name).await,
        BenchmarkPhase::Collecting => {
            handle_collecting(client, &benchmark, &namespace, &name).await
        }
        BenchmarkPhase::Completed | BenchmarkPhase::Failed => {
            // Terminal states — nothing to do.
            Ok(Action::requeue(Duration::from_secs(300)))
        }
    }
}

// ---------------------------------------------------------------------------
// Phase handlers
// ---------------------------------------------------------------------------

/// Pending → create load-generator pods → Running
async fn handle_pending(
    client: &Client,
    benchmark: &StellarBenchmark,
    namespace: &str,
    name: &str,
) -> Result<Action> {
    info!(benchmark = %name, "Creating load-generator pods");

    let pods_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let concurrency = benchmark.spec.concurrency.max(1);
    let mut pod_names = Vec::with_capacity(concurrency as usize);

    for i in 0..concurrency {
        let pod = build_load_generator_pod_with_secret(benchmark, i);
        let pname = pod_name(name, i);

        match pods_api
            .create(&kube::api::PostParams::default(), &pod)
            .await
        {
            Ok(_) => {
                info!(pod = %pname, "Created load-generator pod");
                pod_names.push(pname);
            }
            Err(kube::Error::Api(e)) if e.code == 409 => {
                // Pod already exists (idempotent re-create after a crash).
                info!(pod = %pname, "Load-generator pod already exists");
                pod_names.push(pname);
            }
            Err(e) => {
                error!(pod = %pname, error = %e, "Failed to create load-generator pod");
                patch_status(
                    client,
                    namespace,
                    name,
                    StellarBenchmarkStatus {
                        phase: BenchmarkPhase::Failed,
                        message: Some(format!("Failed to create pod {pname}: {e}")),
                        ..Default::default()
                    },
                )
                .await?;
                return Ok(Action::requeue(Duration::from_secs(60)));
            }
        }
    }

    let started_at = Utc::now().to_rfc3339();
    let mut conditions = vec![];
    set_condition(
        &mut conditions,
        "PodsCreated",
        CONDITION_STATUS_TRUE,
        "PodsCreated",
        &format!("Created {} load-generator pod(s)", pod_names.len()),
    );

    patch_status(
        client,
        namespace,
        name,
        StellarBenchmarkStatus {
            phase: BenchmarkPhase::Running,
            message: Some(format!("Running {} load-generator pod(s)", pod_names.len())),
            started_at: Some(started_at),
            pod_names,
            conditions,
            ..Default::default()
        },
    )
    .await?;

    // Requeue quickly to start polling pod completion.
    Ok(Action::requeue(Duration::from_secs(10)))
}

/// Running → poll pod phases → Collecting (when all done)
async fn handle_running(
    client: &Client,
    benchmark: &StellarBenchmark,
    namespace: &str,
    name: &str,
) -> Result<Action> {
    let status = match &benchmark.status {
        Some(s) => s,
        None => {
            // Status not yet set — treat as Pending.
            return handle_pending(client, benchmark, namespace, name).await;
        }
    };

    let pod_names = &status.pod_names;
    if pod_names.is_empty() {
        warn!(benchmark = %name, "Running phase but no pod names recorded; re-creating pods");
        return handle_pending(client, benchmark, namespace, name).await;
    }

    let pods_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let mut all_done = true;
    let mut any_failed = false;
    let mut running_count = 0u32;

    for pname in pod_names {
        match pods_api.get(pname).await {
            Ok(pod) => {
                let phase = pod
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.as_deref())
                    .unwrap_or("Unknown");

                match phase {
                    "Succeeded" => {
                        debug!(pod = %pname, "Pod succeeded");
                    }
                    "Failed" => {
                        warn!(pod = %pname, "Load-generator pod failed");
                        any_failed = true;
                    }
                    _ => {
                        // Pending / Running / Unknown
                        all_done = false;
                        running_count += 1;
                    }
                }
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                warn!(pod = %pname, "Pod not found; treating as failed");
                any_failed = true;
            }
            Err(e) => {
                warn!(pod = %pname, error = %e, "Error fetching pod status");
                all_done = false;
                running_count += 1;
            }
        }
    }

    if !all_done {
        info!(
            benchmark = %name,
            running = running_count,
            "Pods still running; requeueing"
        );
        return Ok(Action::requeue(Duration::from_secs(10)));
    }

    // All pods have terminated.
    let mut conditions = status.conditions.clone();
    if any_failed {
        set_condition(
            &mut conditions,
            "PodsSucceeded",
            CONDITION_STATUS_FALSE,
            "SomePodsFailed",
            "One or more load-generator pods exited with a non-zero status",
        );
    } else {
        set_condition(
            &mut conditions,
            "PodsSucceeded",
            CONDITION_STATUS_TRUE,
            "AllPodsSucceeded",
            "All load-generator pods completed successfully",
        );
    }

    patch_status(
        client,
        namespace,
        name,
        StellarBenchmarkStatus {
            phase: BenchmarkPhase::Collecting,
            message: Some("Collecting results from load-generator pods".to_string()),
            started_at: status.started_at.clone(),
            pod_names: status.pod_names.clone(),
            conditions,
            ..Default::default()
        },
    )
    .await?;

    Ok(Action::requeue(Duration::from_secs(2)))
}

/// Collecting → gather metrics → write report → Completed
async fn handle_collecting(
    client: &Client,
    benchmark: &StellarBenchmark,
    namespace: &str,
    name: &str,
) -> Result<Action> {
    let status = match &benchmark.status {
        Some(s) => s,
        None => {
            return Err(Error::ConfigError(
                "Collecting phase but no status found".to_string(),
            ));
        }
    };

    let started_at = status
        .started_at
        .clone()
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    // 1. Collect per-pod results.
    let pod_results = collect_pod_results(client, benchmark, &status.pod_names).await?;

    // 2. Scrape ledger close time from the target endpoint.
    let avg_ledger_close_ms = scrape_ledger_close_time_ms(&benchmark.spec.target_endpoint)
        .await
        .unwrap_or(0.0);

    // 3. Build the report spec.
    let mut report_spec =
        BenchmarkReportSpec::from_pod_results(name, &benchmark.spec, &started_at, pod_results);

    // Patch in the scraped ledger close time.
    report_spec.metrics.avg_ledger_close_ms = avg_ledger_close_ms;

    // Compute aggregate P50/P95 from pod results (take max across pods as a
    // conservative estimate; a proper percentile merge would require raw samples).
    let p95 = report_spec
        .pod_results
        .iter()
        .map(|p| p.p99_latency_ms * 0.95) // approximation
        .fold(0.0_f64, f64::max);
    let p50 = report_spec
        .pod_results
        .iter()
        .map(|p| p.p99_latency_ms * 0.5) // approximation
        .fold(0.0_f64, f64::max);
    report_spec.metrics.p95_api_latency_ms = p95;
    report_spec.metrics.p50_api_latency_ms = p50;

    // 4. Store the report.
    let report_ref =
        match store_results(client, namespace, name, benchmark, report_spec.clone()).await {
            Ok(r) => r,
            Err(e) => {
                error!(benchmark = %name, error = %e, "Failed to store benchmark report");
                let mut conditions = status.conditions.clone();
                set_condition(
                    &mut conditions,
                    CONDITION_TYPE_READY,
                    CONDITION_STATUS_FALSE,
                    "ReportStoreFailed",
                    &format!("Failed to store report: {e}"),
                );
                patch_status(
                    client,
                    namespace,
                    name,
                    StellarBenchmarkStatus {
                        phase: BenchmarkPhase::Failed,
                        message: Some(format!("Failed to store report: {e}")),
                        started_at: status.started_at.clone(),
                        pod_names: status.pod_names.clone(),
                        conditions,
                        ..Default::default()
                    },
                )
                .await?;
                return Ok(Action::requeue(Duration::from_secs(60)));
            }
        };

    // 5. Build inline summary for the StellarBenchmark status.
    let m = &report_spec.metrics;
    let summary = BenchmarkSummary {
        peak_tps: m.peak_tps,
        avg_ledger_close_ms: m.avg_ledger_close_ms,
        p99_api_latency_ms: m.p99_api_latency_ms,
        total_transactions: m.total_transactions,
        successful_transactions: m.successful_transactions,
        failed_transactions: m.failed_transactions,
        error_rate_pct: m.error_rate_pct,
    };

    info!(
        benchmark = %name,
        peak_tps = summary.peak_tps,
        avg_ledger_close_ms = summary.avg_ledger_close_ms,
        p99_api_latency_ms = summary.p99_api_latency_ms,
        total_tx = summary.total_transactions,
        error_rate_pct = summary.error_rate_pct,
        report = %report_ref,
        "Benchmark completed"
    );

    let mut conditions = status.conditions.clone();
    set_condition(
        &mut conditions,
        CONDITION_TYPE_READY,
        CONDITION_STATUS_TRUE,
        "BenchmarkComplete",
        &format!("Report stored at {report_ref}"),
    );

    patch_status(
        client,
        namespace,
        name,
        StellarBenchmarkStatus {
            phase: BenchmarkPhase::Completed,
            message: Some(format!(
                "Benchmark complete. Peak TPS: {:.2}, P99 Latency: {:.2}ms, Avg Ledger Close: {:.2}ms",
                summary.peak_tps, summary.p99_api_latency_ms, summary.avg_ledger_close_ms
            )),
            started_at: status.started_at.clone(),
            completed_at: Some(Utc::now().to_rfc3339()),
            report_ref: Some(report_ref),
            pod_names: status.pod_names.clone(),
            summary: Some(summary),
            conditions,
        },
    )
    .await?;

    Ok(Action::requeue(Duration::from_secs(300)))
}

// ---------------------------------------------------------------------------
// Status patch helper
// ---------------------------------------------------------------------------

/// Patch the `status` subresource of a `StellarBenchmark`.
async fn patch_status(
    client: &Client,
    namespace: &str,
    name: &str,
    new_status: StellarBenchmarkStatus,
) -> Result<()> {
    let api: Api<StellarBenchmark> = Api::namespaced(client.clone(), namespace);

    let patch = serde_json::json!({
        "apiVersion": "stellar.org/v1alpha1",
        "kind": "StellarBenchmark",
        "metadata": { "name": name },
        "status": new_status,
    });

    api.patch_status(
        name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}
