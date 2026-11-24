//! Prometheus metrics exporter for Soroban smart contract gas profiling.
//!
//! Exposes a `/metrics` HTTP endpoint that serves CPU instruction and memory
//! consumption histograms aggregated by contract ID. The exporter is designed
//! for real-time operation with minimal overhead on the hot path.
//!
//! # Exported metrics
//!
//! | Metric | Type | Labels | Description |
//! |---|---|---|---|
//! | `soroban_contract_cpu_instructions` | histogram | `contract_id` | CPU instructions per invocation |
//! | `soroban_contract_memory_bytes` | histogram | `contract_id` | Peak memory bytes per invocation |
//! | `soroban_contract_invocations_total` | counter | `contract_id`, `host_function`, `success` | Total invocations |
//! | `soroban_contract_wasm_duration_us` | histogram | `contract_id` | Wasm execution duration in µs |
//! | `soroban_contract_storage_fee_stroops` | histogram | `contract_id` | Storage fees in stroops |

use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use prometheus_client::encoding::text::encode;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::parser::InvocationRecord;

// ---------------------------------------------------------------------------
// Label types
// ---------------------------------------------------------------------------

/// Labels for per-contract histograms.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ContractLabels {
    pub contract_id: String,
}

/// Labels for invocation counters (includes success/failure and function name).
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct InvocationLabels {
    pub contract_id: String,
    pub host_function: String,
    pub success: String,
}

// ---------------------------------------------------------------------------
// Metrics registry
// ---------------------------------------------------------------------------

/// Thread-safe Prometheus metrics exporter.
pub struct MetricsExporter {
    registry: Registry,
    cpu_instructions: Family<ContractLabels, Histogram>,
    memory_bytes: Family<ContractLabels, Histogram>,
    invocations_total: Family<InvocationLabels, Counter<u64, AtomicU64>>,
    wasm_duration_us: Family<ContractLabels, Histogram>,
    storage_fee_stroops: Family<ContractLabels, Histogram>,
}

impl MetricsExporter {
    /// Create a new exporter with all metrics registered.
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let cpu_instructions: Family<ContractLabels, Histogram> =
            Family::new_with_constructor(|| {
                // 100 instructions .. ~100M across 20 buckets (log2 scale).
                Histogram::new(exponential_buckets(100.0, 2.0, 20))
            });

        let memory_bytes: Family<ContractLabels, Histogram> = Family::new_with_constructor(|| {
            // 64 bytes .. ~1 GB across 24 buckets.
            Histogram::new(exponential_buckets(64.0, 2.0, 24))
        });

        let invocations_total: Family<InvocationLabels, Counter<u64, AtomicU64>> =
            Family::default();

        let wasm_duration_us: Family<ContractLabels, Histogram> =
            Family::new_with_constructor(|| {
                // 1 µs .. ~65 ms across 16 buckets.
                Histogram::new(exponential_buckets(1.0, 2.0, 16))
            });

        let storage_fee_stroops: Family<ContractLabels, Histogram> =
            Family::new_with_constructor(|| {
                // 1 stroop .. ~65k stroops across 16 buckets.
                Histogram::new(exponential_buckets(1.0, 2.0, 16))
            });

        registry.register(
            "soroban_contract_cpu_instructions",
            "CPU instructions consumed per contract invocation",
            cpu_instructions.clone(),
        );
        registry.register(
            "soroban_contract_memory_bytes",
            "Peak memory bytes consumed per contract invocation",
            memory_bytes.clone(),
        );
        registry.register(
            "soroban_contract_invocations_total",
            "Total number of contract invocations",
            invocations_total.clone(),
        );
        registry.register(
            "soroban_contract_wasm_duration_us",
            "Wasm execution duration in microseconds",
            wasm_duration_us.clone(),
        );
        registry.register(
            "soroban_contract_storage_fee_stroops",
            "Contract storage fees in stroops",
            storage_fee_stroops.clone(),
        );

        Self {
            registry,
            cpu_instructions,
            memory_bytes,
            invocations_total,
            wasm_duration_us,
            storage_fee_stroops,
        }
    }

    /// Record a parsed invocation into Prometheus metrics.
    pub fn record_invocation(&self, record: &InvocationRecord<'_>) {
        let contract_labels = ContractLabels {
            contract_id: record.contract_id.to_string(),
        };

        self.cpu_instructions
            .get_or_create(&contract_labels)
            .observe(record.cpu_instructions as f64);

        self.memory_bytes
            .get_or_create(&contract_labels)
            .observe(record.memory_bytes as f64);

        self.wasm_duration_us
            .get_or_create(&contract_labels)
            .observe(record.wasm_execution_duration_us as f64);

        self.storage_fee_stroops
            .get_or_create(&contract_labels)
            .observe(record.storage_fee_stroops as f64);

        let invocation_labels = InvocationLabels {
            contract_id: record.contract_id.to_string(),
            host_function: record.host_function.to_string(),
            success: record.success.to_string(),
        };
        self.invocations_total
            .get_or_create(&invocation_labels)
            .inc();
    }

    /// Encode the current metrics into the Prometheus text exposition format.
    pub fn encode_metrics(&self) -> String {
        let mut buf = String::new();
        if let Err(e) = encode(&mut buf, &self.registry) {
            error!("Failed to encode metrics: {}", e);
            return format!("# encoding error: {e}\n");
        }
        buf
    }

    /// Returns a reference to the internal registry (for advanced use).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Shared state for HTTP server
// ---------------------------------------------------------------------------

struct ExporterState {
    exporter: MetricsExporter,
    invocations_total: std::sync::atomic::AtomicU64,
}

type SharedState = Arc<RwLock<ExporterState>>;

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

async fn metrics_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    let body = st.exporter.encode_metrics();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        body,
    )
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Start the metrics exporter HTTP server.
///
/// Returns immediately after spawning the server task. The server runs until
/// the provided shutdown signal is triggered or the process exits.
pub async fn serve_metrics(
    exporter: MetricsExporter,
    bind_addr: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state: SharedState = Arc::new(RwLock::new(ExporterState {
        exporter,
        invocations_total: std::sync::atomic::AtomicU64::new(0),
    }));

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(health_handler))
        .with_state(state);

    let listener = TcpListener::bind(bind_addr).await?;
    info!("Metrics exporter listening on http://{bind_addr}");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Convenience: build and start the exporter from a bind address string.
pub async fn run_exporter(bind_addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: SocketAddr = bind_addr.parse()?;
    let exporter = MetricsExporter::new();
    serve_metrics(exporter, addr).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_invocation_line;

    const SAMPLE_LOG: &str = r#"{"timestamp":"2025-01-15T10:30:00Z","level":"info","msg":"contract_invocation","contract_id":"CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC","cpu_instructions":142000,"memory_bytes":524288,"wasm_execution_duration_us":1500,"storage_fee_stroops":100,"host_function":"invoke","success":true}"#;

    #[test]
    fn test_exporter_records_invocation() {
        let exporter = MetricsExporter::new();
        let record = parse_invocation_line(SAMPLE_LOG).unwrap();
        exporter.record_invocation(&record);

        let text = exporter.encode_metrics();
        assert!(text.contains("soroban_contract_cpu_instructions"));
        assert!(text.contains("soroban_contract_memory_bytes"));
        assert!(text.contains("soroban_contract_invocations_total"));
        assert!(text.contains("soroban_contract_wasm_duration_us"));
        assert!(text.contains("soroban_contract_storage_fee_stroops"));
    }

    #[test]
    fn test_exporter_text_format_valid() {
        let exporter = MetricsExporter::new();
        let record = parse_invocation_line(SAMPLE_LOG).unwrap();
        exporter.record_invocation(&record);

        let text = exporter.encode_metrics();
        // Prometheus text format lines start with metric name or '#'.
        for line in text.lines() {
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with("HELP")
                || line.starts_with("TYPE")
            {
                continue;
            }
            assert!(line.starts_with("soroban_"), "unexpected line: {line}");
        }
    }

    #[test]
    fn test_exporter_multiple_contracts() {
        let exporter = MetricsExporter::new();
        let line1 = r#"{"contract_id":"C1","cpu_instructions":1000,"memory_bytes":256,"host_function":"invoke","success":true}"#;
        let line2 = r#"{"contract_id":"C2","cpu_instructions":2000,"memory_bytes":512,"host_function":"invoke","success":true}"#;
        exporter.record_invocation(&parse_invocation_line(line1).unwrap());
        exporter.record_invocation(&parse_invocation_line(line2).unwrap());

        let text = exporter.encode_metrics();
        assert!(text.contains("contract_id=\"C1\""));
        assert!(text.contains("contract_id=\"C2\""));
    }

    #[test]
    fn test_exporter_failed_invocation_label() {
        let exporter = MetricsExporter::new();
        let line = r#"{"contract_id":"C1","cpu_instructions":0,"memory_bytes":0,"host_function":"invoke","success":false}"#;
        exporter.record_invocation(&parse_invocation_line(line).unwrap());

        let text = exporter.encode_metrics();
        assert!(text.contains("success=\"false\""));
    }

    #[test]
    fn test_exporter_default() {
        let exporter = MetricsExporter::default();
        let text = exporter.encode_metrics();
        assert!(text.contains("soroban_contract_cpu_instructions"));
    }

    #[tokio::test]
    async fn test_metrics_endpoint_returns_200() {
        let state: SharedState = Arc::new(RwLock::new(ExporterState {
            exporter: MetricsExporter::new(),
            invocations_total: AtomicU64::new(0),
        }));

        let app = Router::new()
            .route("/metrics", get(metrics_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resp = reqwest::get(format!("http://{addr}/metrics"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.text().await.unwrap();
        assert!(body.contains("soroban_contract_cpu_instructions"));
    }
}
