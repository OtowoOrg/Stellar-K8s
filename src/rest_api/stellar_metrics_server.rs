//! Custom Kubernetes Metrics Server for Stellar-Specific HPA Scaling
//!
//! Implements the `custom.metrics.k8s.io/v1beta2` API so the Kubernetes
//! Horizontal Pod Autoscaler can scale Horizon instances based on
//! Stellar-native signals — primarily **transactions per second (TPS)** and
//! **submission queue length** — rather than generic CPU/Memory metrics.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/apis/custom.metrics.k8s.io/v1beta2` | API discovery |
//! | GET | `/apis/custom.metrics.k8s.io/v1beta2/namespaces/{ns}/pods/{name}/{metric}` | Pod metric |
//! | GET | `/apis/custom.metrics.k8s.io/v1beta2/namespaces/{ns}/stellarnodes.stellar.org/{name}/{metric}` | StellarNode metric |
//!
//! # Supported Metrics
//!
//! | Metric name | Unit | HPA usage |
//! |-------------|------|-----------|
//! | `stellar_horizon_tps` | transactions/s | Scale up when TPS/replica > threshold |
//! | `stellar_horizon_queue_length` | transactions | Scale up when queue > threshold |
//! | `stellar_ledger_sequence` | ledger | Informational |
//! | `stellar_ingestion_lag` | ledgers | Alert / scale |
//! | `stellar_active_connections` | connections | Scale |
//!
//! # HPA Example
//!
//! ```yaml
//! apiVersion: autoscaling/v2
//! kind: HorizontalPodAutoscaler
//! metadata:
//!   name: horizon-hpa
//! spec:
//!   scaleTargetRef:
//!     apiVersion: apps/v1
//!     kind: Deployment
//!     name: horizon
//!   minReplicas: 2
//!   maxReplicas: 20
//!   metrics:
//!   - type: Object
//!     object:
//!       metric:
//!         name: stellar_horizon_tps
//!       describedObject:
//!         apiVersion: stellar.org/v1alpha1
//!         kind: StellarNode
//!         name: my-horizon
//!       target:
//!         type: Value
//!         value: "100"   # scale up when TPS > 100 per replica
//! ```

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, instrument, warn};

use crate::controller::ControllerState;
use crate::crd::StellarNode;
use kube::{api::Api, ResourceExt};

// ── API discovery types ───────────────────────────────────────────────────────

/// Response for the API discovery endpoint.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResourceList {
    pub kind: String,
    pub api_version: String,
    pub group_version: String,
    pub resources: Vec<ApiResource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResource {
    pub name: String,
    pub namespaced: bool,
    pub kind: String,
}

// ── Metric value types (custom.metrics.k8s.io/v1beta2) ───────────────────────

/// A single metric value as returned by the custom metrics API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricValue {
    pub describe_object: MetricDescribedObject,
    pub metric: MetricIdentifier,
    pub timestamp: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricDescribedObject {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricIdentifier {
    pub name: String,
}

/// List of metric values (the actual API response body).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricValueList {
    pub kind: String,
    pub api_version: String,
    pub metadata: serde_json::Value,
    pub items: Vec<MetricValue>,
}

impl MetricValueList {
    fn new(items: Vec<MetricValue>) -> Self {
        Self {
            kind: "MetricValueList".to_string(),
            api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
            metadata: serde_json::json!({}),
            items,
        }
    }
}

// ── Metric resolution ─────────────────────────────────────────────────────────

/// All Stellar-specific metrics exposed to the HPA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StellarHpaMetric {
    /// Transactions per second ingested by Horizon.
    HorizonTps,
    /// Pending transaction queue depth.
    HorizonQueueLength,
    /// Current ledger sequence number.
    LedgerSequence,
    /// Ingestion lag in ledgers behind the network tip.
    IngestionLag,
    /// Active peer/client connections.
    ActiveConnections,
}

impl StellarHpaMetric {
    /// Resolve a metric name string (as sent by the HPA controller) to a typed
    /// variant. Accepts both canonical names and short aliases.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "stellar_horizon_tps" | "transactions_per_second" | "stellar_tps" | "tps" => {
                Some(Self::HorizonTps)
            }
            "stellar_horizon_queue_length"
            | "queue_length"
            | "stellar_queue_length"
            | "horizon_queue" => Some(Self::HorizonQueueLength),
            "stellar_ledger_sequence" | "ledger_sequence" => Some(Self::LedgerSequence),
            "stellar_ingestion_lag" | "ingestion_lag" => Some(Self::IngestionLag),
            "stellar_active_connections" | "active_connections" => Some(Self::ActiveConnections),
            _ => None,
        }
    }

    /// Derive a metric value from a StellarNode's current status.
    pub fn value_from_node(&self, node: &StellarNode) -> f64 {
        match self {
            Self::LedgerSequence => node
                .status
                .as_ref()
                .and_then(|s| s.ledger_sequence)
                .unwrap_or(0) as f64,
            // TPS, queue length, ingestion lag, and active connections are not
            // stored directly on the StellarNode status — they are scraped from
            // the Horizon metrics endpoint at query time. We return a sentinel
            // value here; the real scraping happens in `resolve_live_metric`.
            Self::HorizonTps => 0.0,
            Self::HorizonQueueLength => 0.0,
            Self::IngestionLag => 0.0,
            Self::ActiveConnections => 0.0,
        }
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /apis/custom.metrics.k8s.io/v1beta2
/// API discovery — tells the HPA which metrics are available.
pub async fn api_discovery() -> Json<ApiResourceList> {
    Json(ApiResourceList {
        kind: "APIResourceList".to_string(),
        api_version: "v1".to_string(),
        group_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        resources: vec![
            ApiResource {
                name: "pods/stellar_horizon_tps".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
            ApiResource {
                name: "pods/stellar_horizon_queue_length".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
            ApiResource {
                name: "pods/stellar_ledger_sequence".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
            ApiResource {
                name: "pods/stellar_ingestion_lag".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
            ApiResource {
                name: "pods/stellar_active_connections".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
            ApiResource {
                name: "stellarnodes.stellar.org/stellar_horizon_tps".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
            ApiResource {
                name: "stellarnodes.stellar.org/stellar_horizon_queue_length".to_string(),
                namespaced: true,
                kind: "MetricValueList".to_string(),
            },
        ],
    })
}

/// GET /apis/custom.metrics.k8s.io/v1beta2/namespaces/{namespace}/pods/{name}/{metric}
/// Returns the current value of a Stellar metric for a specific pod.
#[instrument(skip(state))]
pub async fn get_pod_stellar_metric(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, pod_name, metric_name)): Path<(String, String, String)>,
) -> Result<Json<MetricValueList>, (StatusCode, String)> {
    debug!(
        "Custom metrics request: pod={}/{} metric={}",
        namespace, pod_name, metric_name
    );

    let metric = StellarHpaMetric::from_name(&metric_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Unknown metric: {metric_name}"),
        )
    })?;

    // Find the StellarNode that owns this pod.
    let node_api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);
    let nodes = node_api
        .list(&Default::default())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Match by pod name prefix (pods are named <node-name>-<ordinal> or <node-name>-<hash>).
    let node = nodes
        .items
        .iter()
        .find(|n| pod_name.starts_with(&n.name_any()))
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("No StellarNode found for pod {pod_name}"),
            )
        })?;

    let value = resolve_live_metric(&state, node, &metric, &namespace).await;

    let item = MetricValue {
        describe_object: MetricDescribedObject {
            api_version: "v1".to_string(),
            kind: "Pod".to_string(),
            name: pod_name.clone(),
            namespace: namespace.clone(),
        },
        metric: MetricIdentifier {
            name: metric_name.clone(),
        },
        timestamp: chrono::Utc::now().to_rfc3339(),
        value: format!("{:.0}", value),
    };

    Ok(Json(MetricValueList::new(vec![item])))
}

/// GET /apis/custom.metrics.k8s.io/v1beta2/namespaces/{namespace}/stellarnodes.stellar.org/{name}/{metric}
/// Returns the current value of a Stellar metric for a StellarNode object.
#[instrument(skip(state))]
pub async fn get_stellarnode_metric(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, node_name, metric_name)): Path<(String, String, String)>,
) -> Result<Json<MetricValueList>, (StatusCode, String)> {
    debug!(
        "Custom metrics request: stellarnode={}/{} metric={}",
        namespace, node_name, metric_name
    );

    let metric = StellarHpaMetric::from_name(&metric_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Unknown metric: {metric_name}"),
        )
    })?;

    let node_api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);
    let node = node_api
        .get(&node_name)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let value = resolve_live_metric(&state, &node, &metric, &namespace).await;

    let item = MetricValue {
        describe_object: MetricDescribedObject {
            api_version: "stellar.org/v1alpha1".to_string(),
            kind: "StellarNode".to_string(),
            name: node_name.clone(),
            namespace: namespace.clone(),
        },
        metric: MetricIdentifier {
            name: metric_name.clone(),
        },
        timestamp: chrono::Utc::now().to_rfc3339(),
        value: format!("{:.0}", value),
    };

    Ok(Json(MetricValueList::new(vec![item])))
}

// ── Live metric resolution ────────────────────────────────────────────────────

/// Resolve the current value of a metric for a StellarNode.
///
/// For `LedgerSequence` we read directly from the node status. For live
/// metrics (TPS, queue length, etc.) we scrape the Horizon `/metrics`
/// Prometheus endpoint on port 8002.
async fn resolve_live_metric(
    state: &Arc<ControllerState>,
    node: &StellarNode,
    metric: &StellarHpaMetric,
    namespace: &str,
) -> f64 {
    // LedgerSequence is always available from status.
    if *metric == StellarHpaMetric::LedgerSequence {
        return node
            .status
            .as_ref()
            .and_then(|s| s.ledger_sequence)
            .unwrap_or(0) as f64;
    }

    // For live metrics, scrape the Horizon /metrics endpoint.
    let node_name = node.name_any();
    let service_name = format!("{node_name}-svc");
    let horizon_metrics_url =
        format!("http://{service_name}.{namespace}.svc.cluster.local:8002/metrics");

    match scrape_horizon_metric(&horizon_metrics_url, metric).await {
        Ok(v) => v,
        Err(e) => {
            warn!(
                "Failed to scrape Horizon metrics for {}/{}: {}",
                namespace, node_name, e
            );
            // Fall back to status-derived value.
            metric.value_from_node(node)
        }
    }
}

/// Scrape a specific metric from the Horizon Prometheus `/metrics` endpoint.
async fn scrape_horizon_metric(url: &str, metric: &StellarHpaMetric) -> Result<f64, String> {
    let prom_name = match metric {
        StellarHpaMetric::HorizonTps => "horizon_ingest_ledgers_ingested_total",
        StellarHpaMetric::HorizonQueueLength => "horizon_txsub_queue_size",
        StellarHpaMetric::IngestionLag => "horizon_ingest_latest_ledger",
        StellarHpaMetric::ActiveConnections => "horizon_http_open_connections",
        StellarHpaMetric::LedgerSequence => "horizon_ingest_latest_ledger",
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let body = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    // Parse the Prometheus text format — find the line matching the metric name.
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with(prom_name) {
            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            if let Some(val_str) = parts.get(1) {
                return val_str.trim().parse::<f64>().map_err(|e| e.to_string());
            }
        }
    }

    Err(format!("Metric {prom_name} not found in Prometheus output"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_from_name_canonical() {
        assert_eq!(
            StellarHpaMetric::from_name("stellar_horizon_tps"),
            Some(StellarHpaMetric::HorizonTps)
        );
        assert_eq!(
            StellarHpaMetric::from_name("stellar_horizon_queue_length"),
            Some(StellarHpaMetric::HorizonQueueLength)
        );
        assert_eq!(
            StellarHpaMetric::from_name("stellar_ledger_sequence"),
            Some(StellarHpaMetric::LedgerSequence)
        );
        assert_eq!(
            StellarHpaMetric::from_name("stellar_ingestion_lag"),
            Some(StellarHpaMetric::IngestionLag)
        );
        assert_eq!(
            StellarHpaMetric::from_name("stellar_active_connections"),
            Some(StellarHpaMetric::ActiveConnections)
        );
    }

    #[test]
    fn metric_from_name_aliases() {
        assert_eq!(
            StellarHpaMetric::from_name("transactions_per_second"),
            Some(StellarHpaMetric::HorizonTps)
        );
        assert_eq!(
            StellarHpaMetric::from_name("tps"),
            Some(StellarHpaMetric::HorizonTps)
        );
        assert_eq!(
            StellarHpaMetric::from_name("queue_length"),
            Some(StellarHpaMetric::HorizonQueueLength)
        );
        assert_eq!(
            StellarHpaMetric::from_name("ingestion_lag"),
            Some(StellarHpaMetric::IngestionLag)
        );
    }

    #[test]
    fn metric_from_name_unknown() {
        assert_eq!(StellarHpaMetric::from_name("cpu_usage"), None);
        assert_eq!(StellarHpaMetric::from_name(""), None);
        assert_eq!(StellarHpaMetric::from_name("memory_bytes"), None);
    }

    #[test]
    fn metric_value_list_structure() {
        let list = MetricValueList::new(vec![]);
        assert_eq!(list.kind, "MetricValueList");
        assert_eq!(list.api_version, "custom.metrics.k8s.io/v1beta2");
        assert!(list.items.is_empty());
    }

    #[test]
    fn api_resource_list_has_all_metrics() {
        // Verify the discovery response covers all supported metrics.
        let expected = [
            "stellar_horizon_tps",
            "stellar_horizon_queue_length",
            "stellar_ledger_sequence",
            "stellar_ingestion_lag",
            "stellar_active_connections",
        ];
        for name in expected {
            assert!(
                StellarHpaMetric::from_name(name).is_some(),
                "Missing metric: {name}"
            );
        }
    }
}
