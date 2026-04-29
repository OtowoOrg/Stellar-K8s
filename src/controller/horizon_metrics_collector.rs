//! Background task that polls Horizon and Stellar-Core endpoints for live metrics
//! and writes them into the shared [`StellarMetricsStore`].
//!
//! # Architecture
//!
//! The collector runs as a `tokio::spawn`ed background task, waking every
//! `poll_interval` (default: 30 s).  For each known Horizon node it:
//!
//! 1. Issues `GET {horizon_url}/metrics` and parses the Prometheus text format
//!    to extract `horizon_ingest_transactions_per_second` and
//!    `horizon_ingest_pending_txqueue_count`.
//! 2. Falls back to a `/info` JSON poll when the `/metrics` endpoint is
//!    unavailable (older Horizon versions or misconfigured scrape targets).
//! 3. Writes the result into [`StellarMetricsStore`] with the current timestamp.
//! 4. Also updates the shared Prometheus gauges (`HORIZON_TPS`,
//!    `ACTIVE_CONNECTIONS`) for consistency with the existing `/metrics` endpoint.
//!
//! # Horizon Metrics Text Format (excerpt)
//!
//! ```text
//! # HELP horizon_ingest_transactions_per_second Horizon ingestion TPS
//! # TYPE horizon_ingest_transactions_per_second gauge
//! horizon_ingest_transactions_per_second 42.5
//! # HELP horizon_ingest_pending_txqueue_count Pending transaction queue size
//! # TYPE horizon_ingest_pending_txqueue_count gauge
//! horizon_ingest_pending_txqueue_count 187
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::crd::{NodeType, StellarNode};
use crate::rest_api::metrics_store::{StellarMetricsSnapshot, StellarMetricsStore};

/// Horizon endpoint description — enough to poll metrics.
#[derive(Debug, Clone)]
pub struct HorizonEndpoint {
    /// Kubernetes namespace of the Horizon deployment.
    pub namespace: String,
    /// Kubernetes resource name of the Horizon StellarNode.
    pub name: String,
    /// Base URL of the Horizon HTTP server (e.g. `http://horizon.stellar-system:8000`).
    pub horizon_url: String,
    /// Node type label used for Prometheus gauge updates.
    pub node_type: String,
    /// Network label (e.g. "mainnet", "testnet").
    pub network: String,
    /// Hardware generation label.
    pub hardware_generation: String,
}

/// Spawnable background collector that polls Horizon endpoints for live metrics.
pub struct HorizonMetricsCollector {
    store: Arc<StellarMetricsStore>,
    /// Polling interval between scrape cycles.
    poll_interval: Duration,
    /// HTTP client (reqwest is already a dependency).
    http_client: reqwest::Client,
    /// Kubernetes client to discover Horizon nodes.
    client: kube::Client,
    /// Optional namespace to watch.
    watch_namespace: Option<String>,
}

impl HorizonMetricsCollector {
    /// Create a new collector.
    ///
    /// `poll_interval_secs` controls how often the collector wakes up.
    /// The default recommended value is 30 s, giving the HPA ample data freshness
    /// while not hammering Horizon.
    pub fn new(
        store: Arc<StellarMetricsStore>,
        poll_interval_secs: u64,
        client: kube::Client,
        watch_namespace: Option<String>,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");

        Self {
            store,
            poll_interval: Duration::from_secs(poll_interval_secs.max(5)),
            http_client,
            client,
            watch_namespace,
        }
    }

    /// Run the collector loop indefinitely.
    ///
    /// This is designed to be called inside `tokio::spawn`.  It never returns
    /// under normal operation — cancellation happens via task abort.
    pub async fn run(&self) {
        info!(
            "HorizonMetricsCollector starting (poll_interval={:?})",
            self.poll_interval
        );

        loop {
            // Evict expired entries from the store every cycle.
            self.store.evict_stale();

            let endpoints = self.discover_endpoints().await;
            if endpoints.is_empty() {
                debug!("No Horizon endpoints registered; skipping scrape cycle");
            }

            for ep in &endpoints {
                match self.scrape_endpoint(ep).await {
                    Ok(snap) => {
                        self.store.upsert(&ep.namespace, &ep.name, snap.clone());

                        // Keep the Prometheus gauges in sync.
                        #[cfg(feature = "metrics")]
                        {
                            crate::controller::metrics::set_horizon_tps(
                                &ep.namespace,
                                &ep.name,
                                &ep.node_type,
                                &ep.network,
                                &ep.hardware_generation,
                                snap.tps,
                            );
                            crate::controller::metrics::set_active_connections(
                                &ep.namespace,
                                &ep.name,
                                &ep.node_type,
                                &ep.network,
                                &ep.hardware_generation,
                                snap.active_connections,
                            );
                            crate::controller::metrics::set_ingestion_lag(
                                &ep.namespace,
                                &ep.name,
                                &ep.node_type,
                                &ep.network,
                                &ep.hardware_generation,
                                snap.ingestion_lag,
                            );
                        }

                        info!(
                            namespace = %ep.namespace,
                            name = %ep.name,
                            tps = snap.tps,
                            queue_length = snap.queue_length,
                            ingestion_lag = snap.ingestion_lag,
                            "Scraped Horizon metrics"
                        );
                    }
                    Err(e) => {
                        warn!(
                            namespace = %ep.namespace,
                            name = %ep.name,
                            error = %e,
                            "Failed to scrape Horizon metrics endpoint"
                        );
                    }
                }
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// Discover all Horizon endpoints by querying the Kubernetes API.
    async fn discover_endpoints(&self) -> Vec<HorizonEndpoint> {
        let stellar_nodes_api: kube::Api<StellarNode> = if let Some(ns) = &self.watch_namespace {
            kube::Api::namespaced(self.client.clone(), ns)
        } else {
            kube::Api::all(self.client.clone())
        };

        let lp = kube::api::ListParams::default();
        let nodes = match stellar_nodes_api.list(&lp).await {
            Ok(list) => list,
            Err(e) => {
                warn!(
                    "Failed to list StellarNodes for metrics collection: {:?}",
                    e
                );
                return vec![];
            }
        };

        let mut endpoints = Vec::new();
        for node in nodes.items {
            if node.spec.node_type != NodeType::Horizon {
                continue;
            }
            let namespace = node
                .metadata
                .namespace
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let name = node
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let horizon_url = format!("http://{}.{}.svc.cluster.local:8000", name, namespace);
            let node_type = "horizon".to_string();
            let network = match node.spec.network {
                crate::crd::StellarNetwork::Mainnet => "mainnet",
                crate::crd::StellarNetwork::Testnet => "testnet",
                crate::crd::StellarNetwork::Futurenet => "futurenet",
                crate::crd::StellarNetwork::Custom(_) => "custom",
            }
            .to_string();
            let hardware_generation = "unknown".to_string(); // TODO: resolve from infra

            endpoints.push(HorizonEndpoint {
                namespace,
                name,
                horizon_url,
                node_type,
                network,
                hardware_generation,
            });
        }
        endpoints
    }

    /// Attempt to scrape a single Horizon endpoint.
    ///
    /// First tries `GET /metrics` (Prometheus text format).  If that returns a
    /// non-2xx status or a network error, falls back to `GET /info` (JSON).
    async fn scrape_endpoint(
        &self,
        ep: &HorizonEndpoint,
    ) -> Result<StellarMetricsSnapshot, String> {
        // Primary: Prometheus /metrics
        let metrics_url = format!("{}/metrics", ep.horizon_url.trim_end_matches('/'));
        match self.http_client.get(&metrics_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.text().await.map_err(|e| e.to_string())?;
                return Ok(parse_prometheus_metrics(&body));
            }
            Ok(resp) => {
                debug!(
                    url = %metrics_url,
                    status = %resp.status(),
                    "Prometheus /metrics returned non-2xx; trying /info"
                );
            }
            Err(e) => {
                debug!(
                    url = %metrics_url,
                    error = %e,
                    "Could not reach /metrics; trying /info"
                );
            }
        }

        // Fallback: Horizon JSON /info
        let info_url = format!("{}/info", ep.horizon_url.trim_end_matches('/'));
        let resp = self
            .http_client
            .get(&info_url)
            .send()
            .await
            .map_err(|e| format!("GET {info_url}: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("GET {info_url}: HTTP {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {e}"))?;

        Ok(parse_info_json(&body))
    }
}

/// Parse a Prometheus text-format metrics response.
///
/// Extracts:
/// - `horizon_ingest_transactions_per_second` → `tps`
/// - `horizon_ingest_pending_txqueue_count`   → `queue_length`
/// - `horizon_ingest_ledger_ingestion_count`  → proxy for ingestion lag when
///   combined with the current ledger (best-effort)
pub fn parse_prometheus_metrics(text: &str) -> StellarMetricsSnapshot {
    let mut values: HashMap<&str, f64> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // Lines look like: `metric_name{labels} value [timestamp]`
        // We split on the first '{' or space.
        let (key, rest) = if let Some(pos) = line.find('{') {
            (&line[..pos], &line[pos..])
        } else if let Some(pos) = line.find(' ') {
            (&line[..pos], &line[pos..])
        } else {
            continue;
        };

        // The value is the first whitespace-separated token after closing '}' or after the key.
        let value_str = if rest.contains('}') {
            rest.splitn(2, '}').nth(1).unwrap_or("").trim()
        } else {
            rest.trim()
        };

        // May have trailing timestamp.
        let value_str = value_str.split_whitespace().next().unwrap_or("");
        if let Ok(v) = value_str.parse::<f64>() {
            values.insert(key, v);
        }
    }

    let tps = values
        .get("horizon_ingest_transactions_per_second")
        .copied()
        .unwrap_or(0.0) as i64;

    let queue_length = values
        .get("horizon_ingest_pending_txqueue_count")
        .copied()
        .unwrap_or(0.0) as i64;

    let ingestion_lag = values
        .get("horizon_ingest_latest_ledger_age_seconds")
        .copied()
        .map(|v| v as i64)
        .unwrap_or(0);

    let ledger_sequence = values
        .get("horizon_ingest_latest_ledger")
        .copied()
        .unwrap_or(0.0) as u64;

    let active_connections = values
        .get("horizon_active_request_count")
        .copied()
        .unwrap_or(0.0) as i64;

    StellarMetricsSnapshot {
        tps,
        queue_length,
        ingestion_lag,
        ledger_sequence,
        active_connections,
        updated_at: chrono::Utc::now(),
    }
}

/// Parse Horizon's `/info` JSON response as a best-effort metrics fallback.
///
/// Horizon's `/info` endpoint doesn't expose TPS or queue depth, so we derive
/// approximate values from the fields it does expose.
fn parse_info_json(info: &serde_json::Value) -> StellarMetricsSnapshot {
    // Ingestion lag: age of the latest ingested ledger (seconds).
    let ingestion_lag = info
        .pointer("/ingest/ledger_age")
        .and_then(|v| v.as_f64())
        .map(|v| v as i64)
        .unwrap_or(0);

    // Ledger sequence from the core state.
    let ledger_sequence = info
        .pointer("/core_latest_ledger")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    StellarMetricsSnapshot {
        tps: 0,          // not available from /info
        queue_length: 0, // not available from /info
        ingestion_lag,
        ledger_sequence,
        active_connections: 0,
        updated_at: chrono::Utc::now(),
    }
}

/// Construct a [`HorizonMetricsCollector`] and spawn it as a background task.
pub fn spawn_horizon_metrics_collector(
    store: Arc<StellarMetricsStore>,
    poll_interval_secs: u64,
    client: kube::Client,
    watch_namespace: Option<String>,
) -> tokio::task::JoinHandle<()> {
    let collector =
        HorizonMetricsCollector::new(store, poll_interval_secs, client, watch_namespace);
    tokio::spawn(async move {
        collector.run().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prometheus_metrics_basic() {
        let text = r#"
# HELP horizon_ingest_transactions_per_second Ingestion TPS
# TYPE horizon_ingest_transactions_per_second gauge
horizon_ingest_transactions_per_second 42
# HELP horizon_ingest_pending_txqueue_count Pending tx queue
# TYPE horizon_ingest_pending_txqueue_count gauge
horizon_ingest_pending_txqueue_count 187
# HELP horizon_ingest_latest_ledger Current ledger
# TYPE horizon_ingest_latest_ledger gauge
horizon_ingest_latest_ledger 49500000
"#;
        let snap = parse_prometheus_metrics(text);
        assert_eq!(snap.tps, 42);
        assert_eq!(snap.queue_length, 187);
        assert_eq!(snap.ledger_sequence, 49_500_000);
    }

    #[test]
    fn test_parse_prometheus_metrics_with_labels() {
        let text = r#"
horizon_ingest_transactions_per_second{instance="h0"} 75
horizon_ingest_pending_txqueue_count{instance="h0"} 300
"#;
        let snap = parse_prometheus_metrics(text);
        assert_eq!(snap.tps, 75);
        assert_eq!(snap.queue_length, 300);
    }

    #[test]
    fn test_parse_prometheus_metrics_missing_values() {
        let snap = parse_prometheus_metrics("");
        assert_eq!(snap.tps, 0);
        assert_eq!(snap.queue_length, 0);
        assert_eq!(snap.ledger_sequence, 0);
    }

    #[test]
    fn test_parse_prometheus_metrics_float_truncation() {
        let text = "horizon_ingest_transactions_per_second 99.7\n";
        let snap = parse_prometheus_metrics(text);
        // i64 truncation: 99.7 -> 99
        assert_eq!(snap.tps, 99);
    }

    #[test]
    fn test_parse_info_json_extracts_ingestion_lag() {
        let json = serde_json::json!({
            "ingest": { "ledger_age": 3.5 },
            "core_latest_ledger": 49_000_000_u64
        });
        let snap = parse_info_json(&json);
        assert_eq!(snap.ingestion_lag, 3);
        assert_eq!(snap.ledger_sequence, 49_000_000);
    }

    #[test]
    fn test_parse_info_json_missing_fields_returns_zeros() {
        let snap = parse_info_json(&serde_json::json!({}));
        assert_eq!(snap.tps, 0);
        assert_eq!(snap.queue_length, 0);
        assert_eq!(snap.ingestion_lag, 0);
    }

    #[test]
    fn test_collector_creation() {
        let store = Arc::new(StellarMetricsStore::new());
        let client =
            kube::Client::try_default().unwrap_or_else(|_| panic!("Need kube client for test"));
        let collector = HorizonMetricsCollector::new(store, 30, client, None);
        // Verify minimum poll interval clamping (< 5 s gets clamped to 5 s).
        let store_fast = Arc::new(StellarMetricsStore::new());
        let client_fast =
            kube::Client::try_default().unwrap_or_else(|_| panic!("Need kube client for test"));
        let collector_fast = HorizonMetricsCollector::new(store_fast, 1, client_fast, None);
        assert!(collector_fast.poll_interval >= Duration::from_secs(5));
        assert!(collector.poll_interval == Duration::from_secs(30));
    }
}
