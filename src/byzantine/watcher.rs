//! Watcher — polls a single Stellar Core node and exports Prometheus metrics.
//!
//! Each deployed `stellar-watcher` binary runs one `Watcher` instance. It:
//!
//! 1. Polls `GET /info` on the configured Stellar Core HTTP endpoint every
//!    `poll_interval_secs` seconds.
//! 2. Extracts the latest externalized ledger sequence and hash.
//! 3. Updates Prometheus gauges/counters.
//! 4. Serves `/metrics` on `metrics_bind_addr` for Prometheus scraping.
//!
//! The watcher is intentionally minimal — it has no Kubernetes dependency and
//! can run as a standalone binary in any cloud environment (ECS, GKE, AKS,
//! bare-metal, etc.).

use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use once_cell::sync::Lazy;
use prometheus_client::encoding::text::encode;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use reqwest::Client;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use super::types::{ConsensusObservation, StellarCoreInfoResponse, WatcherConfig};

// ---------------------------------------------------------------------------
// Metric label types
// ---------------------------------------------------------------------------

/// Labels attached to every watcher metric for multi-dimensional filtering.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct WatcherLabels {
    /// Unique watcher instance ID.
    pub watcher_id: String,
    /// Cloud provider.
    pub cloud: String,
    /// Geographic region.
    pub region: String,
    /// Stellar network.
    pub network: String,
    /// Stellar Core endpoint being polled.
    pub node_endpoint: String,
}

/// Labels for the ledger hash metric — carries the actual hash value as a label
/// so Prometheus can group-by hash across watchers.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct LedgerHashLabels {
    pub watcher_id: String,
    pub cloud: String,
    pub region: String,
    pub network: String,
    pub node_endpoint: String,
    /// Hex-encoded ledger close hash (64 chars).
    pub hash: String,
}

// ---------------------------------------------------------------------------
// Per-watcher Prometheus registry and metrics
// ---------------------------------------------------------------------------

/// Shared Prometheus registry for this watcher process.
pub struct WatcherMetrics {
    pub registry: Registry,

    /// Latest externalized ledger sequence.
    pub ledger_sequence: Family<WatcherLabels, Gauge<i64, AtomicI64>>,

    /// Always 1; the `hash` label carries the ledger close hash.
    /// Prometheus will see one time-series per unique hash value.
    /// We reset the old hash series by setting it to 0 before updating.
    pub ledger_hash: Family<LedgerHashLabels, Gauge<i64, AtomicI64>>,

    /// 1 = node is EXTERNALIZED (in consensus), 0 = not.
    pub consensus_view: Family<WatcherLabels, Gauge<i64, AtomicI64>>,

    /// Cumulative count of failed polls.
    pub poll_errors_total: Family<WatcherLabels, Counter<u64, AtomicU64>>,

    /// Unix timestamp (seconds) of the last successful poll.
    pub last_poll_timestamp_seconds: Family<WatcherLabels, Gauge<i64, AtomicI64>>,

    /// Identity gauge — always 1, carries watcher metadata as labels.
    pub watcher_info: Family<WatcherLabels, Gauge<i64, AtomicI64>>,
}

impl WatcherMetrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let ledger_sequence: Family<WatcherLabels, Gauge<i64, AtomicI64>> = Family::default();
        let ledger_hash: Family<LedgerHashLabels, Gauge<i64, AtomicI64>> = Family::default();
        let consensus_view: Family<WatcherLabels, Gauge<i64, AtomicI64>> = Family::default();
        let poll_errors_total: Family<WatcherLabels, Counter<u64, AtomicU64>> = Family::default();
        let last_poll_timestamp_seconds: Family<WatcherLabels, Gauge<i64, AtomicI64>> =
            Family::default();
        let watcher_info: Family<WatcherLabels, Gauge<i64, AtomicI64>> = Family::default();

        registry.register(
            "stellar_watcher_ledger_sequence",
            "Latest externalized ledger sequence observed from this vantage point",
            ledger_sequence.clone(),
        );
        registry.register(
            "stellar_watcher_ledger_hash",
            "Always 1; the 'hash' label carries the hex-encoded ledger close hash",
            ledger_hash.clone(),
        );
        registry.register(
            "stellar_watcher_consensus_view",
            "1 if the node is EXTERNALIZED (in consensus), 0 otherwise",
            consensus_view.clone(),
        );
        registry.register(
            "stellar_watcher_poll_errors_total",
            "Cumulative number of failed polls to the Stellar Core endpoint",
            poll_errors_total.clone(),
        );
        registry.register(
            "stellar_watcher_last_poll_timestamp_seconds",
            "Unix timestamp of the last successful poll",
            last_poll_timestamp_seconds.clone(),
        );
        registry.register(
            "stellar_watcher_info",
            "Always 1; carries watcher identity metadata as labels",
            watcher_info.clone(),
        );

        Self {
            registry,
            ledger_sequence,
            ledger_hash,
            consensus_view,
            poll_errors_total,
            last_poll_timestamp_seconds,
            watcher_info,
        }
    }
}

// ---------------------------------------------------------------------------
// Watcher state shared between the poll loop and the HTTP server
// ---------------------------------------------------------------------------

pub struct WatcherState {
    pub config: WatcherConfig,
    pub metrics: WatcherMetrics,
    /// Most recent observation (for health endpoint).
    pub last_observation: Option<ConsensusObservation>,
    /// Previous ledger hash — used to zero-out stale hash series.
    pub previous_hash: Option<String>,
}

impl WatcherState {
    pub fn new(config: WatcherConfig) -> Self {
        Self {
            config,
            metrics: WatcherMetrics::new(),
            last_observation: None,
            previous_hash: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Main Watcher
// ---------------------------------------------------------------------------

/// Runs the watcher: starts the metrics HTTP server and the poll loop concurrently.
pub async fn run_watcher(config: WatcherConfig) -> Result<()> {
    info!(
        watcher_id = %config.watcher_id,
        region = %config.region,
        cloud = %config.cloud,
        network = %config.network,
        endpoint = %config.node_endpoint,
        "Starting Byzantine Watcher"
    );

    let state = Arc::new(RwLock::new(WatcherState::new(config.clone())));

    // Seed the identity gauge immediately so Prometheus can discover this watcher
    // even before the first successful poll.
    {
        let st = state.read().await;
        let labels = watcher_labels(&st.config);
        st.metrics.watcher_info.get_or_create(&labels).set(1);
    }

    let metrics_addr = config.metrics_bind_addr.clone();
    let poll_interval = Duration::from_secs(config.poll_interval_secs);
    let request_timeout = Duration::from_secs(config.request_timeout_secs);
    let endpoint = config.node_endpoint.clone();

    // Build HTTP client once — reuse across polls.
    let http_client = Client::builder()
        .timeout(request_timeout)
        .user_agent("stellar-byzantine-watcher/1.0")
        .build()
        .context("Failed to build HTTP client")?;

    // Spawn metrics HTTP server.
    let server_state = Arc::clone(&state);
    let server_handle = tokio::spawn(async move {
        if let Err(e) = serve_metrics(server_state, &metrics_addr).await {
            error!("Metrics server error: {}", e);
        }
    });

    // Poll loop.
    let poll_state = Arc::clone(&state);
    let poll_handle = tokio::spawn(async move {
        loop {
            match poll_stellar_core(&http_client, &endpoint).await {
                Ok(obs) => {
                    let mut st = poll_state.write().await;
                    update_metrics(&mut st, obs);
                }
                Err(e) => {
                    warn!("Poll failed: {}", e);
                    let st = poll_state.read().await;
                    let labels = watcher_labels(&st.config);
                    st.metrics.poll_errors_total.get_or_create(&labels).inc();
                    st.metrics.consensus_view.get_or_create(&labels).set(0);
                }
            }
            sleep(poll_interval).await;
        }
    });

    // Wait for either task to exit (both should run forever).
    tokio::select! {
        res = server_handle => {
            error!("Metrics server task exited unexpectedly: {:?}", res);
        }
        res = poll_handle => {
            error!("Poll loop task exited unexpectedly: {:?}", res);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Stellar Core polling
// ---------------------------------------------------------------------------

/// Poll `GET /info` on the Stellar Core HTTP endpoint and return an observation.
async fn poll_stellar_core(
    client: &Client,
    endpoint: &str,
) -> Result<(u64, String, bool, String)> {
    let url = format!("{}/info", endpoint.trim_end_matches('/'));
    debug!("Polling Stellar Core at {}", url);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("HTTP GET {} failed", url))?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from {}", resp.status(), url);
    }

    let info: StellarCoreInfoResponse = resp
        .json()
        .await
        .with_context(|| format!("Failed to parse JSON from {}", url))?;

    let sequence = info.info.ledger.num;
    let hash = info.info.ledger.hash.clone();
    let state_str = info.info.state.to_lowercase();

    // Stellar Core reports "Synced!" when fully externalized.
    let is_externalized = state_str.contains("synced") || state_str.contains("externalize");

    debug!(
        sequence,
        hash = %hash,
        state = %info.info.state,
        is_externalized,
        "Poll successful"
    );

    Ok((sequence, hash, is_externalized, info.info.state))
}

// ---------------------------------------------------------------------------
// Metric update
// ---------------------------------------------------------------------------

fn update_metrics(
    st: &mut WatcherState,
    (sequence, hash, is_externalized, _state_str): (u64, String, bool, String),
) {
    let labels = watcher_labels(&st.config);

    // Update sequence.
    st.metrics
        .ledger_sequence
        .get_or_create(&labels)
        .set(sequence as i64);

    // Zero out the previous hash series so Prometheus doesn't keep stale series
    // with value 1 forever. We only keep the *current* hash at 1.
    if let Some(ref prev_hash) = st.previous_hash.clone() {
        if prev_hash != &hash {
            let old_hash_labels = LedgerHashLabels {
                watcher_id: st.config.watcher_id.clone(),
                cloud: st.config.cloud.clone(),
                region: st.config.region.clone(),
                network: st.config.network.clone(),
                node_endpoint: st.config.node_endpoint.clone(),
                hash: prev_hash.clone(),
            };
            st.metrics
                .ledger_hash
                .get_or_create(&old_hash_labels)
                .set(0);
        }
    }

    // Set current hash series to 1.
    let hash_labels = LedgerHashLabels {
        watcher_id: st.config.watcher_id.clone(),
        cloud: st.config.cloud.clone(),
        region: st.config.region.clone(),
        network: st.config.network.clone(),
        node_endpoint: st.config.node_endpoint.clone(),
        hash: hash.clone(),
    };
    st.metrics.ledger_hash.get_or_create(&hash_labels).set(1);
    st.previous_hash = Some(hash.clone());

    // Update consensus view.
    st.metrics
        .consensus_view
        .get_or_create(&labels)
        .set(if is_externalized { 1 } else { 0 });

    // Update last poll timestamp.
    let now_unix = chrono::Utc::now().timestamp();
    st.metrics
        .last_poll_timestamp_seconds
        .get_or_create(&labels)
        .set(now_unix);

    // Record observation for health endpoint.
    st.last_observation = Some(ConsensusObservation {
        watcher_id: st.config.watcher_id.clone(),
        cloud: st.config.cloud.clone(),
        region: st.config.region.clone(),
        network: st.config.network.clone(),
        node_endpoint: st.config.node_endpoint.clone(),
        ledger_sequence: sequence,
        ledger_hash: hash,
        is_externalized,
        observed_at: chrono::Utc::now(),
    });

    info!(
        watcher_id = %st.config.watcher_id,
        sequence,
        is_externalized,
        "Observation recorded"
    );
}

// ---------------------------------------------------------------------------
// Metrics HTTP server
// ---------------------------------------------------------------------------

type SharedState = Arc<RwLock<WatcherState>>;

async fn serve_metrics(state: SharedState, bind_addr: &str) -> Result<()> {
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(health_handler))
        .route("/readyz", get(ready_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("Failed to bind metrics server to {}", bind_addr))?;

    info!("Metrics server listening on http://{}", bind_addr);

    axum::serve(listener, app)
        .await
        .context("Metrics server error")?;

    Ok(())
}

async fn metrics_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    let mut buf = String::new();
    if let Err(e) = encode(&mut buf, &st.metrics.registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e),
        );
    }
    (StatusCode::OK, buf)
}

async fn health_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    match &st.last_observation {
        Some(obs) => {
            let age_secs = (chrono::Utc::now() - obs.observed_at).num_seconds();
            // Unhealthy if last poll was more than 3× the poll interval ago.
            let max_age = (st.config.poll_interval_secs * 3) as i64;
            if age_secs > max_age {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("Last poll was {}s ago (max {}s)", age_secs, max_age),
                )
            } else {
                (StatusCode::OK, "OK".to_string())
            }
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "No observation yet".to_string(),
        ),
    }
}

async fn ready_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    if st.last_observation.is_some() {
        (StatusCode::OK, "Ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "Not ready yet")
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn watcher_labels(config: &WatcherConfig) -> WatcherLabels {
    WatcherLabels {
        watcher_id: config.watcher_id.clone(),
        cloud: config.cloud.clone(),
        region: config.region.clone(),
        network: config.network.clone(),
        node_endpoint: config.node_endpoint.clone(),
    }
}
