//! Custom Metrics API provider implementation
//!
//! Exposes Stellar-specific metrics in the format expected by the Kubernetes
//! Horizontal Pod Autoscaler via the `custom.metrics.k8s.io/v1beta2` API group.
//!
//! # Supported Metrics
//!
//! | Metric name                    | Aliases                                    | Description                                |
//! |-------------------------------|--------------------------------------------|--------------------------------------------|
//! | `stellar_horizon_tps`         | `transactions_per_second`, `stellar_tps`   | Transactions per second ingested by Horizon|
//! | `stellar_horizon_queue_length`| `queue_length`, `stellar_queue_length`     | Pending transaction queue depth            |
//! | `stellar_ledger_sequence`     | `ledger_sequence`                          | Current ledger sequence number             |
//! | `stellar_ingestion_lag`       | `ingestion_lag`                            | Lag (ledgers) behind network tip           |
//! | `stellar_active_connections`  | `active_connections`                       | Active peer/client connections             |
//!
//! # Kubernetes Integration
//!
//! The operator registers as an API extension server for `custom.metrics.k8s.io/v1beta2`
//! via an `APIService` resource (see `config/custom-metrics-apiservice.yaml`).
//! The HPA controller calls these endpoints to fetch metric values for scaling decisions.
//!
//! See: <https://github.com/kubernetes/community/blob/master/contributors/design-proposals/custom-metrics-api.md>

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use prometheus_client::encoding::text::encode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::controller::ControllerState;

// ---------------------------------------------------------------------------
// Metric type enum
// ---------------------------------------------------------------------------

/// All Stellar custom metrics supported by the HPA integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StellarMetricType {
    /// Transactions per second ingested by Horizon.
    TransactionsPerSecond,
    /// Number of transactions waiting in Horizon's submission queue.
    QueueLength,
    /// Ledger sequence number — key metric for node health.
    LedgerSequence,
    /// Ingestion lag in ledgers — indicates how far behind the node is.
    IngestionLag,
    /// Active connections to the node.
    ActiveConnections,
    /// Horizon request queue length
    HorizonQueueLength,
}

impl StellarMetricType {
    /// Resolve a metric name string (as sent by the HPA) to a typed variant.
    ///
    /// Accepts both canonical names and convenient short aliases so that HPA
    /// manifests can use either `stellar_horizon_tps` or `transactions_per_second`.
    pub fn from_str(name: &str) -> Option<Self> {
        match name {
            // TPS — primary HPA metric for Horizon load scaling
            "stellar_horizon_tps" | "transactions_per_second" | "stellar_tps" => {
                Some(StellarMetricType::TransactionsPerSecond)
            }
            // Queue depth — secondary HPA metric for queue-back-pressure scaling
            "stellar_horizon_queue_length" | "queue_length" | "stellar_queue_length" => {
                Some(StellarMetricType::QueueLength)
            }
            // Ledger sequence
            "stellar_ledger_sequence" | "ledger_sequence" => {
                Some(StellarMetricType::LedgerSequence)
            }
            // Ingestion lag
            "stellar_ingestion_lag" | "ingestion_lag" => Some(StellarMetricType::IngestionLag),
            // Active connections
            "stellar_horizon_tps" | "requests_per_second" => {
                Some(StellarMetricType::RequestsPerSecond)
            }
            "stellar_queue_length" | "queue_length" | "horizon_queue_length" => {
                Some(StellarMetricType::HorizonQueueLength)
            }
            "stellar_active_connections" | "active_connections" => {
                Some(StellarMetricType::ActiveConnections)
            }
            _ => None,
        }
    }

    /// Canonical Prometheus metric name for this type.
    pub fn prometheus_name(&self) -> &'static str {
        match self {
            StellarMetricType::TransactionsPerSecond => "stellar_horizon_tps",
            StellarMetricType::QueueLength => "stellar_horizon_queue_length",
            StellarMetricType::LedgerSequence => "stellar_node_ledger_sequence",
            StellarMetricType::IngestionLag => "stellar_node_ingestion_lag",
            StellarMetricType::RequestsPerSecond => "stellar_horizon_tps",
            StellarMetricType::HorizonQueueLength => "stellar_horizon_queue_length",
            StellarMetricType::ActiveConnections => "stellar_node_active_connections",
        }
    }

    /// Human-readable description shown in the API discovery response.
    pub fn description(&self) -> &'static str {
        match self {
            StellarMetricType::TransactionsPerSecond => {
                "Transactions per second ingested by Horizon"
            }
            StellarMetricType::QueueLength => {
                "Number of transactions waiting in Horizon's submission queue"
            }
            StellarMetricType::LedgerSequence => "Current ledger sequence number of the node",
            StellarMetricType::IngestionLag => {
                "Lag in ledgers between the network tip and this node"
            }
            StellarMetricType::ActiveConnections => "Number of active peer or client connections",
        }
    }

    /// All known metric names for inclusion in the discovery list.
    pub fn all_names() -> Vec<&'static str> {
        vec![
            "stellar_horizon_tps",
            "stellar_horizon_queue_length",
            "stellar_ledger_sequence",
            "stellar_ingestion_lag",
            "stellar_active_connections",
        ]
    }
}

// ---------------------------------------------------------------------------
// Wire types (Kubernetes custom metrics API v1beta2)
// ---------------------------------------------------------------------------

/// Top-level list returned by the custom metrics API.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetricValueList {
    pub kind: String,
    pub api_version: String,
    pub metadata: ListMetadata,
    pub items: Vec<MetricValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_link: Option<String>,
}

/// A single metric value for one object.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetricValue {
    pub described_object: DescribedObject,
    pub metric: MetricIdentifier,
    /// RFC 3339 timestamp of the observation.
    pub timestamp: String,
    /// Aggregation window in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<i64>,
    /// Kubernetes `quantity` string (e.g. `"125"`, `"1k"`).
    pub value: String,
}

/// The Kubernetes object this metric is associated with.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DescribedObject {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub api_version: String,
}

/// Metric identifier including optional label selector.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetricIdentifier {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<LabelSelector>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LabelSelector {
    #[serde(rename = "matchLabels")]
    pub match_labels: BTreeMap<String, String>,
}

/// Kubernetes-style error response for the custom metrics API.
#[derive(Serialize, Debug)]
pub struct ApiError {
    pub kind: String,
    pub api_version: String,
    pub metadata: BTreeMap<String, String>,
    pub message: String,
    pub reason: String,
    pub code: u16,
}

/// APIResource entry returned by the discovery endpoint.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiResource {
    pub name: String,
    pub singular_name: String,
    pub namespaced: bool,
    pub kind: String,
    pub verbs: Vec<String>,
}

/// APIResourceList returned by `GET /apis/custom.metrics.k8s.io/v1beta2`.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiResourceList {
    pub kind: String,
    pub api_version: String,
    pub group_version: String,
    pub resources: Vec<ApiResource>,
}

// ---------------------------------------------------------------------------
// Metric value lookup — reads from StellarMetricsStore
// ---------------------------------------------------------------------------

/// Retrieve a metric value from the shared `StellarMetricsStore`.
///
/// Returns `0` when no fresh data is available, preventing the HPA from making
/// scaling decisions on stale information.
fn get_metric_value(
    state: &ControllerState,
    metric_type: &StellarMetricType,
    namespace: &str,
    name: &str,
) -> i64 {
    #[cfg(feature = "rest-api")]
    {
        let store = &state.metrics_store;
        match metric_type {
            StellarMetricType::TransactionsPerSecond => {
                debug!("Fetching TPS from store for {}/{}", namespace, name);
                store.tps(namespace, name)
            }
            StellarMetricType::QueueLength => {
                debug!(
                    "Fetching queue_length from store for {}/{}",
                    namespace, name
                );
                store.queue_length(namespace, name)
            }
            StellarMetricType::IngestionLag => {
                debug!(
                    "Fetching ingestion_lag from store for {}/{}",
                    namespace, name
                );
                store.ingestion_lag(namespace, name)
            }
            StellarMetricType::LedgerSequence => {
                debug!(
                    "Fetching ledger_sequence from store for {}/{}",
                    namespace, name
                );
                store.ledger_sequence(namespace, name)
            }
            StellarMetricType::ActiveConnections => {
                debug!(
                    "Fetching active_connections from store for {}/{}",
                    namespace, name
                );
                store.active_connections(namespace, name)
            }
        }
    }
    #[cfg(not(feature = "rest-api"))]
    {
        let _ = (state, metric_type, namespace, name);
        0
    }
}

// ---------------------------------------------------------------------------
// Helper — build a MetricValueList response
// ---------------------------------------------------------------------------

fn metric_value_list(
    namespace: String,
    name: String,
    kind: &str,
    api_version: &str,
    metric_name: String,
    value: i64,
) -> MetricValueList {
    let now = chrono::Utc::now().to_rfc3339();
    MetricValueList {
        kind: "MetricValueList".to_string(),
        api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        metadata: ListMetadata { self_link: None },
        items: vec![MetricValue {
            described_object: DescribedObject {
                kind: kind.to_string(),
                namespace,
                name,
                api_version: api_version.to_string(),
            },
            metric: MetricIdentifier {
                name: metric_name,
                selector: None,
            },
            timestamp: now,
            window_seconds: Some(60),
            value: value.to_string(),
        }],
    }
}

fn not_found_error(metric_name: &str) -> Response {
    let error = ApiError {
        kind: "Status".to_string(),
        api_version: "v1".to_string(),
        metadata: BTreeMap::new(),
        message: format!("Metric '{metric_name}' not found"),
        reason: "MetricNotFound".to_string(),
        code: 404,
    };
    (StatusCode::NOT_FOUND, Json(error)).into_response()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /apis/custom.metrics.k8s.io/v1beta2`
///
/// Returns the API discovery document listing all supported Stellar metrics.
/// Required by the Kubernetes aggregation layer — the HPA queries this before
/// making metric value requests.
pub async fn get_metrics_discovery() -> Response {
    let resources: Vec<ApiResource> = StellarMetricType::all_names()
        .into_iter()
        .flat_map(|metric_name| {
            // Expose each metric for both Pod and StellarNode resource kinds.
            vec![
                ApiResource {
                    name: format!("pods/{metric_name}"),
                    singular_name: String::new(),
                    namespaced: true,
                    kind: "MetricValueList".to_string(),
                    verbs: vec!["get".to_string()],
                },
                ApiResource {
                    name: format!("stellarnodes.stellar.org/{metric_name}"),
                    singular_name: String::new(),
                    namespaced: true,
                    kind: "MetricValueList".to_string(),
                    verbs: vec!["get".to_string()],
                },
                ApiResource {
                    name: format!("horizons.stellar.org/{metric_name}"),
                    singular_name: String::new(),
                    namespaced: true,
                    kind: "MetricValueList".to_string(),
                    verbs: vec!["get".to_string()],
                },
            ]
        })
        .collect();

    Json(ApiResourceList {
        kind: "APIResourceList".to_string(),
        api_version: "v1".to_string(),
        group_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        resources,
    })
    .into_response()
}

/// `GET /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric`
///
/// Returns the current value of a Stellar metric for a specific Pod.
/// The HPA uses this to scale Horizon Deployments based on per-pod metrics.
/// Fetch a metric value from the Prometheus registry
/// Returns the metric value as a string, or None if not found
fn get_metric_value(metric_type: &StellarMetricType, namespace: &str, name: &str) -> Option<i64> {
    let metric_name = metric_type.prometheus_name();
    let mut buffer = String::new();
    if encode(&mut buffer, &crate::controller::metrics::REGISTRY).is_err() {
        warn!("Failed to encode metrics registry for {}", metric_name);
        return None;
    }

    buffer.lines().find_map(|line| match_metric_line(line, metric_name, namespace, name))
}

fn match_metric_line(line: &str, metric_name: &str, namespace: &str, name: &str) -> Option<i64> {
    let prefix = format!("{metric_name}{{");
    if !line.starts_with(&prefix) {
        return None;
    }

    let parts: Vec<&str> = line[prefix.len()..].splitn(2, '}').collect();
    if parts.len() != 2 {
        return None;
    }

    let (labels, value_str) = (parts[0], parts[1].trim_start());
    let namespace_label = extract_label(labels, "namespace")?;
    let name_label = extract_label(labels, "name")?;

    if namespace_label != namespace || name_label != name {
        return None;
    }

    parse_metric_value(value_str)
}

fn extract_label(labels: &str, key: &str) -> Option<String> {
    for part in labels.split(',') {
        let mut kv = part.splitn(2, '=');
        let label_key = kv.next()?.trim();
        let label_value = kv.next()?.trim();
        if label_key != key {
            continue;
        }

        if let Some(stripped) = label_value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            return Some(stripped.replace("\\\"", "\""));
        }

        return Some(label_value.to_string());
    }
    None
}

fn parse_metric_value(value_str: &str) -> Option<i64> {
    let value_token = value_str.split_whitespace().next()?;
    value_token
        .parse::<f64>()
        .ok()
        .map(|value| value as i64)
}

/// Handler for custom metrics API: /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric
#[tracing::instrument(
    skip(state),
    fields(namespace = %namespace, name = %name, metric = %metric_name, reconcile_id = "-")
)]
pub async fn get_pod_metric(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> Response {
    debug!(
        "Custom metrics request: pod {}/{}/{}",
        namespace, name, metric_name
    );

    let metric_type = match StellarMetricType::from_str(&metric_name) {
        Some(mt) => mt,
        None => {
            warn!(
                "Unsupported metric '{}' requested for pod {}/{}",
                metric_name, namespace, name
            );
            return not_found_error(&metric_name);
        }
    };

    let value = get_metric_value(&state, &metric_type, &namespace, &name);

    Json(metric_value_list(
        namespace,
        name,
        "Pod",
        "v1",
        metric_name,
        value,
    ))
    .into_response()
}

/// `GET /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/stellarnodes.stellar.org/:name/:metric`
///
/// Returns the current value of a Stellar metric for a specific `StellarNode` CR.
/// Used by HPAs that target the StellarNode object directly.
#[tracing::instrument(
    skip(state),
    fields(namespace = %namespace, name = %name, metric = %metric_name, reconcile_id = "-")
)]
pub async fn get_stellar_node_metric(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> Response {
    debug!(
        "Custom metrics request: StellarNode {}/{}/{}",
        namespace, name, metric_name
    );

    let metric_type = match StellarMetricType::from_str(&metric_name) {
        Some(mt) => mt,
        None => {
            warn!(
                "Unsupported metric '{}' requested for StellarNode {}/{}",
                metric_name, namespace, name
            );
            return not_found_error(&metric_name);
        }
    };

    let value = get_metric_value(&state, &metric_type, &namespace, &name);

    Json(metric_value_list(
        namespace,
        name,
        "StellarNode",
        "stellar.org/v1alpha1",
        metric_name,
        value,
    ))
    .into_response()
}

/// `GET /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/horizons.stellar.org/:name/:metric`
///
/// Convenience endpoint for Horizon-specific objects, functionally identical to
/// the StellarNode endpoint but labelled as `Horizon` for clarity in HPA configs.
#[tracing::instrument(
    skip(state),
    fields(namespace = %namespace, name = %name, metric = %metric_name, reconcile_id = "-")
)]
pub async fn get_horizon_metric(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> Response {
    debug!(
        "Custom metrics request: Horizon {}/{}/{}",
        namespace, name, metric_name
    );

    let metric_type = match StellarMetricType::from_str(&metric_name) {
        Some(mt) => mt,
        None => {
            warn!(
                "Unsupported metric '{}' requested for Horizon {}/{}",
                metric_name, namespace, name
            );
            return not_found_error(&metric_name);
        }
    };

    let value = get_metric_value(&state, &metric_type, &namespace, &name);

    Json(metric_value_list(
        namespace,
        name,
        "StellarNode",
        "stellar.org/v1alpha1",
        metric_name,
        value,
    ))
    .into_response()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- StellarMetricType::from_str ----------------------------------------

    #[test]
    fn test_tps_metric_type_aliases() {
        assert_eq!(
            StellarMetricType::from_str("stellar_horizon_tps"),
            Some(StellarMetricType::TransactionsPerSecond)
        );
        assert_eq!(
            StellarMetricType::from_str("transactions_per_second"),
            Some(StellarMetricType::TransactionsPerSecond)
        );
        assert_eq!(
            StellarMetricType::from_str("stellar_tps"),
            Some(StellarMetricType::TransactionsPerSecond)
        );
    }

    #[test]
    fn test_queue_length_metric_type_aliases() {
        assert_eq!(
            StellarMetricType::from_str("stellar_horizon_queue_length"),
            Some(StellarMetricType::QueueLength)
        );
        assert_eq!(
            StellarMetricType::from_str("queue_length"),
            Some(StellarMetricType::QueueLength)
        );
        assert_eq!(
            StellarMetricType::from_str("stellar_queue_length"),
            Some(StellarMetricType::QueueLength)
        );
    }

    #[test]
    fn test_metric_type_from_str_ledger_sequence() {
        assert_eq!(
            StellarMetricType::from_str("stellar_ledger_sequence"),
            Some(StellarMetricType::LedgerSequence)
        );
        assert_eq!(
            StellarMetricType::from_str("ledger_sequence"),
            Some(StellarMetricType::LedgerSequence)
        );
    }

    #[test]
    fn test_metric_type_from_str_ingestion_lag() {
        assert_eq!(
            StellarMetricType::from_str("stellar_ingestion_lag"),
            Some(StellarMetricType::IngestionLag)
        );
        assert_eq!(
            StellarMetricType::from_str("ingestion_lag"),
            Some(StellarMetricType::IngestionLag)
        );
    }

    #[test]
    fn test_metric_type_from_str_active_connections() {
        assert_eq!(
            StellarMetricType::from_str("stellar_active_connections"),
            Some(StellarMetricType::ActiveConnections)
        );
        assert_eq!(
            StellarMetricType::from_str("active_connections"),
            Some(StellarMetricType::ActiveConnections)
        );
    }

    #[test]
    fn test_metric_type_from_str_unsupported() {
        assert_eq!(StellarMetricType::from_str("unknown_metric"), None);
        assert_eq!(StellarMetricType::from_str("cpu"), None);
        assert_eq!(StellarMetricType::from_str(""), None);
    }

    #[test]
    fn test_prometheus_name_tps() {
        assert_eq!(
            StellarMetricType::TransactionsPerSecond.prometheus_name(),
            "stellar_horizon_tps"
        );
    }

    #[test]
    fn test_prometheus_name_queue_length() {
        assert_eq!(
            StellarMetricType::QueueLength.prometheus_name(),
            "stellar_horizon_queue_length"
        );
    }

    #[test]
    fn test_prometheus_name_ledger_sequence() {
        assert_eq!(
            StellarMetricType::LedgerSequence.prometheus_name(),
            "stellar_node_ledger_sequence"
        );
    }

    #[test]
    fn test_prometheus_name_ingestion_lag() {
        assert_eq!(
            StellarMetricType::IngestionLag.prometheus_name(),
            "stellar_node_ingestion_lag"
        );
    }

    // ---- Wire types ---------------------------------------------------------

    #[test]
    fn test_metric_value_list_structure() {
        let list = MetricValueList {
            kind: "MetricValueList".to_string(),
            api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
            metadata: ListMetadata { self_link: None },
            items: vec![],
        };
        assert_eq!(list.kind, "MetricValueList");
        assert_eq!(list.api_version, "custom.metrics.k8s.io/v1beta2");
        assert!(list.items.is_empty());
    }

    #[test]
    fn test_metric_value_serialization() {
        let metric = MetricValue {
            described_object: DescribedObject {
                kind: "Pod".to_string(),
                namespace: "default".to_string(),
                name: "horizon-0".to_string(),
                api_version: "v1".to_string(),
            },
            metric: MetricIdentifier {
                name: "stellar_horizon_tps".to_string(),
                selector: None,
            },
            timestamp: "2026-04-28T22:00:00Z".to_string(),
            window_seconds: Some(60),
            value: "125".to_string(),
        };
        let json = serde_json::to_string(&metric).unwrap();
        assert!(json.contains("\"name\":\"horizon-0\""));
        assert!(json.contains("\"value\":\"125\""));
        assert!(json.contains("\"windowSeconds\":60"));
    }

    #[test]
    fn test_api_error_structure() {
        let error = ApiError {
            kind: "Status".to_string(),
            api_version: "v1".to_string(),
            metadata: BTreeMap::new(),
            message: "Metric 'foo' not found".to_string(),
            reason: "MetricNotFound".to_string(),
            code: 404,
        };
        assert_eq!(error.kind, "Status");
        assert_eq!(error.reason, "MetricNotFound");
        assert_eq!(error.code, 404);
    }

    #[test]
    fn test_discovery_resource_count() {
        // Each metric name → 3 resource kinds (Pod, StellarNode, Horizon).
        let expected = StellarMetricType::all_names().len() * 3;
        let resources: Vec<ApiResource> = StellarMetricType::all_names()
            .into_iter()
            .flat_map(|metric_name| {
                vec![
                    ApiResource {
                        name: format!("pods/{metric_name}"),
                        singular_name: String::new(),
                        namespaced: true,
                        kind: "MetricValueList".to_string(),
                        verbs: vec!["get".to_string()],
                    },
                    ApiResource {
                        name: format!("stellarnodes.stellar.org/{metric_name}"),
                        singular_name: String::new(),
                        namespaced: true,
                        kind: "MetricValueList".to_string(),
                        verbs: vec!["get".to_string()],
                    },
                    ApiResource {
                        name: format!("horizons.stellar.org/{metric_name}"),
                        singular_name: String::new(),
                        namespaced: true,
                        kind: "MetricValueList".to_string(),
                        verbs: vec!["get".to_string()],
                    },
                ]
            })
            .collect();
        assert_eq!(resources.len(), expected);
    }

    #[test]
    fn test_all_metric_names_non_empty() {
        let names = StellarMetricType::all_names();
        assert!(!names.is_empty());
        for name in names {
            assert!(!name.is_empty());
            // Each canonical name must round-trip through from_str.
            assert!(
                StellarMetricType::from_str(name).is_some(),
                "Canonical name '{name}' did not round-trip"
            );
        }
    }

    #[test]
    fn test_metric_value_list_helper() {
        let list = metric_value_list(
            "default".to_string(),
            "horizon-0".to_string(),
            "Pod",
            "v1",
            "stellar_horizon_tps".to_string(),
            42,
        );
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].value, "42");
        assert_eq!(list.items[0].described_object.kind, "Pod");
        assert_eq!(list.items[0].described_object.namespace, "default");
    }

    #[test]
    fn test_get_metric_value_horizon_tps() {
        let labels = crate::controller::metrics::NodeLabels {
            namespace: "test-ns".to_string(),
            name: "horizon-pod-0".to_string(),
            node_type: "Horizon".to_string(),
            network: "Testnet".to_string(),
            hardware_generation: "gen1".to_string(),
        };

        crate::controller::metrics::HORIZON_TPS
            .get_or_create(&labels)
            .set(123);

        assert_eq!(
            get_metric_value(
                &StellarMetricType::RequestsPerSecond,
                "test-ns",
                "horizon-pod-0"
            ),
            Some(123)
        );
    }

    #[test]
    fn test_get_metric_value_queue_length() {
        let labels = crate::controller::metrics::NodeLabels {
            namespace: "test-ns".to_string(),
            name: "horizon-pod-0".to_string(),
            node_type: "Horizon".to_string(),
            network: "Testnet".to_string(),
            hardware_generation: "gen1".to_string(),
        };

        crate::controller::metrics::HORIZON_QUEUE_LENGTH
            .get_or_create(&labels)
            .set(42);

        assert_eq!(
            get_metric_value(&StellarMetricType::HorizonQueueLength, "test-ns", "horizon-pod-0"),
            Some(42)
        );
    }
}
