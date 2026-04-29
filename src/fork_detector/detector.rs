//! Core fork detection logic.
//!
//! Polls the local Stellar Core node and multiple public anchor nodes,
//! compares ledger hashes, and fires alerts when divergence persists.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::future::join_all;
use prometheus_client::encoding::text::encode;
use reqwest::Client;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};

use super::metrics::{
    build_registry, CONSECUTIVE_DIVERGING_LEDGERS, FORK_ALERTS_TOTAL, LOCAL_LEDGER_SEQUENCE,
    POLL_ERRORS_TOTAL, RESPONDING_ANCHORS, SYNC_CONFIDENCE,
};
use super::types::{
    ForkCheckResult, ForkDetectorConfig, HorizonLedgersResponse, LedgerObservation,
    StellarCoreInfoResponse,
};

/// Shared state for the fork detector.
struct DetectorState {
    config: ForkDetectorConfig,
    /// Number of consecutive cycles where divergence was detected.
    consecutive_diverging: u64,
    /// Prometheus registry.
    registry: prometheus_client::registry::Registry,
}

impl DetectorState {
    fn new(config: ForkDetectorConfig) -> Self {
        let registry = build_registry();
        Self {
            config,
            consecutive_diverging: 0,
            registry,
        }
    }

    fn labels(&self) -> super::metrics::ForkDetectorLabels {
        super::metrics::ForkDetectorLabels {
            network: self.config.network.clone(),
            node_id: self.config.node_id.clone(),
        }
    }
}

/// Entry point — starts the metrics HTTP server and the detection loop.
pub async fn run_fork_detector(config: ForkDetectorConfig) -> Result<()> {
    info!(
        node_id = %config.node_id,
        network = %config.network,
        local_endpoint = %config.local_endpoint,
        anchors = config.anchor_endpoints.len(),
        poll_interval_secs = config.poll_interval_secs,
        divergence_threshold = config.divergence_threshold_ledgers,
        "Starting Stellar Fork Detector sidecar"
    );

    let state = Arc::new(RwLock::new(DetectorState::new(config.clone())));

    let http_client = Client::builder()
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .user_agent("stellar-fork-detector/1.0")
        .build()
        .context("Failed to build HTTP client")?;

    // Spawn metrics server.
    let server_state = Arc::clone(&state);
    let metrics_addr = config.metrics_bind_addr.clone();
    tokio::spawn(async move {
        if let Err(e) = serve_metrics(server_state, &metrics_addr).await {
            error!("Fork detector metrics server error: {}", e);
        }
    });

    // Detection loop.
    let poll_interval = Duration::from_secs(config.poll_interval_secs);
    loop {
        match run_detection_cycle(&http_client, &config).await {
            Ok(result) => {
                let mut st = state.write().await;
                update_state_and_metrics(&mut st, &result);
            }
            Err(e) => {
                warn!("Fork detection cycle error: {}", e);
                let st = state.read().await;
                POLL_ERRORS_TOTAL.get_or_create(&st.labels()).inc();
            }
        }
        sleep(poll_interval).await;
    }
}

/// Run one detection cycle: poll local + all anchors, compare hashes.
async fn run_detection_cycle(
    client: &Client,
    config: &ForkDetectorConfig,
) -> Result<ForkCheckResult> {
    // Poll local node.
    let local_obs = poll_local_core(client, &config.local_endpoint).await;

    let (local_sequence, local_hash) = match &local_obs {
        Some(obs) if obs.ok => (obs.sequence, obs.hash.clone()),
        _ => {
            anyhow::bail!("Local Stellar Core node is unreachable or not synced");
        }
    };

    // Poll all anchors concurrently.
    let anchor_futs: Vec<_> = config
        .anchor_endpoints
        .iter()
        .map(|ep| poll_anchor(client, ep, local_sequence))
        .collect();

    let anchor_results = join_all(anchor_futs).await;

    let responding: Vec<&LedgerObservation> = anchor_results.iter().filter(|o| o.ok).collect();
    let agreeing = responding.iter().filter(|o| o.hash == local_hash).count();

    let responding_count = responding.len();
    let sync_confidence = if responding_count == 0 {
        // No anchors reachable — can't determine confidence; treat as 1.0 (no evidence of fork)
        1.0_f64
    } else {
        agreeing as f64 / responding_count as f64
    };

    // Divergence = majority of responding anchors disagree with local hash.
    let divergence_detected = responding_count > 0 && agreeing < responding_count.div_ceil(2);

    Ok(ForkCheckResult {
        local_sequence,
        local_hash,
        agreeing_anchors: agreeing,
        responding_anchors: responding_count,
        divergence_detected,
        sync_confidence,
    })
}

/// Update in-memory state and Prometheus metrics after a detection cycle.
fn update_state_and_metrics(st: &mut DetectorState, result: &ForkCheckResult) {
    let labels = st.labels();

    // Update sequence and confidence metrics.
    LOCAL_LEDGER_SEQUENCE
        .get_or_create(&labels)
        .set(result.local_sequence as i64);

    // Encode confidence as millipercent (0–1000) to avoid float in Prometheus gauge.
    let confidence_mp = (result.sync_confidence * 1000.0) as i64;
    SYNC_CONFIDENCE.get_or_create(&labels).set(confidence_mp);

    RESPONDING_ANCHORS
        .get_or_create(&labels)
        .set(result.responding_anchors as i64);

    if result.divergence_detected {
        st.consecutive_diverging += 1;
        CONSECUTIVE_DIVERGING_LEDGERS
            .get_or_create(&labels)
            .set(st.consecutive_diverging as i64);

        warn!(
            sequence = result.local_sequence,
            local_hash = %result.local_hash,
            agreeing = result.agreeing_anchors,
            responding = result.responding_anchors,
            consecutive = st.consecutive_diverging,
            confidence_pct = format!("{:.1}%", result.sync_confidence * 100.0),
            "Fork divergence detected"
        );

        // Fire alert if divergence persists beyond threshold.
        if st.consecutive_diverging >= st.config.divergence_threshold_ledgers {
            FORK_ALERTS_TOTAL.get_or_create(&labels).inc();
            error!(
                sequence = result.local_sequence,
                local_hash = %result.local_hash,
                consecutive_diverging = st.consecutive_diverging,
                threshold = st.config.divergence_threshold_ledgers,
                network = %st.config.network,
                node_id = %st.config.node_id,
                "FORK ALERT: Local ledger hash has diverged from anchor majority for {} consecutive ledgers. \
                 Possible network fork detected. Investigate immediately.",
                st.consecutive_diverging
            );
        }
    } else {
        // Reset consecutive counter on agreement.
        if st.consecutive_diverging > 0 {
            info!(
                sequence = result.local_sequence,
                "Fork divergence resolved after {} consecutive diverging ledgers",
                st.consecutive_diverging
            );
        }
        st.consecutive_diverging = 0;
        CONSECUTIVE_DIVERGING_LEDGERS.get_or_create(&labels).set(0);
    }

    info!(
        sequence = result.local_sequence,
        agreeing = result.agreeing_anchors,
        responding = result.responding_anchors,
        confidence_pct = format!("{:.1}%", result.sync_confidence * 100.0),
        divergence = result.divergence_detected,
        "Fork detection cycle complete"
    );
}

// ---------------------------------------------------------------------------
// Node polling helpers
// ---------------------------------------------------------------------------

/// Poll the local Stellar Core `/info` endpoint.
async fn poll_local_core(client: &Client, endpoint: &str) -> Option<LedgerObservation> {
    let url = format!("{}/info", endpoint.trim_end_matches('/'));
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<StellarCoreInfoResponse>().await {
                Ok(info) => {
                    let state_lower = info.info.state.to_lowercase();
                    let synced =
                        state_lower.contains("synced") || state_lower.contains("externalize");
                    if !synced {
                        warn!(state = %info.info.state, "Local node not yet synced");
                        return None;
                    }
                    Some(LedgerObservation {
                        source: endpoint.to_string(),
                        sequence: info.info.ledger.num,
                        hash: info.info.ledger.hash,
                        ok: true,
                    })
                }
                Err(e) => {
                    warn!("Failed to parse local core /info response: {}", e);
                    None
                }
            }
        }
        Ok(resp) => {
            warn!("Local core /info returned HTTP {}", resp.status());
            None
        }
        Err(e) => {
            warn!("Failed to reach local core at {}: {}", url, e);
            None
        }
    }
}

/// Poll a Horizon anchor endpoint for the latest ledger at or near `target_sequence`.
/// Horizon's `/ledgers?order=desc&limit=1` returns the most recent ledger.
async fn poll_anchor(client: &Client, endpoint: &str, _target_sequence: u64) -> LedgerObservation {
    // Horizon public API: GET /ledgers?order=desc&limit=1
    let url = if endpoint.contains("/ledgers") {
        endpoint.to_string()
    } else {
        format!(
            "{}/ledgers?order=desc&limit=1",
            endpoint.trim_end_matches('/')
        )
    };

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<HorizonLedgersResponse>().await {
                Ok(data) => {
                    if let Some(ledger) = data.embedded.records.first() {
                        return LedgerObservation {
                            source: endpoint.to_string(),
                            sequence: ledger.sequence,
                            hash: ledger.hash.clone(),
                            ok: true,
                        };
                    }
                    warn!("Anchor {} returned empty ledger list", endpoint);
                    LedgerObservation {
                        source: endpoint.to_string(),
                        sequence: 0,
                        hash: String::new(),
                        ok: false,
                    }
                }
                Err(e) => {
                    warn!("Failed to parse anchor {} response: {}", endpoint, e);
                    LedgerObservation {
                        source: endpoint.to_string(),
                        sequence: 0,
                        hash: String::new(),
                        ok: false,
                    }
                }
            }
        }
        Ok(resp) => {
            warn!("Anchor {} returned HTTP {}", endpoint, resp.status());
            LedgerObservation {
                source: endpoint.to_string(),
                sequence: 0,
                hash: String::new(),
                ok: false,
            }
        }
        Err(e) => {
            warn!("Failed to reach anchor {}: {}", endpoint, e);
            LedgerObservation {
                source: endpoint.to_string(),
                sequence: 0,
                hash: String::new(),
                ok: false,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Metrics HTTP server
// ---------------------------------------------------------------------------

type SharedState = Arc<RwLock<DetectorState>>;

async fn serve_metrics(state: SharedState, bind_addr: &str) -> Result<()> {
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(health_handler))
        .route("/readyz", get(ready_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| {
            format!(
                "Failed to bind fork detector metrics server to {}",
                bind_addr
            )
        })?;

    info!(
        "Fork detector metrics server listening on http://{}",
        bind_addr
    );

    axum::serve(listener, app)
        .await
        .context("Fork detector metrics server error")?;

    Ok(())
}

async fn metrics_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    let mut buf = String::new();
    if let Err(e) = encode(&mut buf, &st.registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e),
        );
    }
    (StatusCode::OK, buf)
}

async fn health_handler(_state: State<SharedState>) -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn ready_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    // Ready once we have at least one successful poll (consecutive_diverging was set or reset).
    // We use the local sequence metric as a proxy.
    let labels = st.labels();
    let seq = LOCAL_LEDGER_SEQUENCE.get_or_create(&labels).get();
    if seq > 0 {
        (StatusCode::OK, "Ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "Not ready yet")
    }
}
