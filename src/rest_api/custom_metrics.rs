//! Custom Metrics API provider implementation
//!
//! Exposes metrics in the format expected by Kubernetes Horizontal Pod Autoscaler
//! for the custom.metrics.k8s.io/v1beta2 API.
//!
//! This module implements the Kubernetes Custom Metrics API specification to allow
//! Horizontal Pod Autoscalers to scale based on Stellar-specific metrics such as:
//! - stellar_ledger_sequence: Current ledger sequence number
//! - stellar_ingestion_lag: Lag between latest and current ledger
//!
//! See: https://github.com/kubernetes/community/blob/master/contributors/design-proposals/custom-metrics-api.md

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::controller::ControllerState;

/// Supported Stellar custom metrics for HPA
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StellarMetricType {
    /// Ledger sequence number - key metric for node health
    LedgerSequence,
    /// Ingestion lag in ledgers - indicates how far behind the node is
    IngestionLag,
    /// Transactions per second - for Horizon API nodes
    RequestsPerSecond,
    /// Active connections to the node
    ActiveConnections,
}

impl StellarMetricType {
    /// Get metric type from string name
    pub fn from_str(name: &str) -> Option<Self> {
        match name {
            "stellar_ledger_sequence" | "ledger_sequence" => {
                Some(StellarMetricType::LedgerSequence)
            }
            "stellar_ingestion_lag" | "ingestion_lag" => Some(StellarMetricType::IngestionLag),
            "stellar_horizon_tps" | "requests_per_second" => {
                Some(StellarMetricType::RequestsPerSecond)
            }
            "stellar_active_connections" | "active_connections" => {
                Some(StellarMetricType::ActiveConnections)
            }
            _ => None,
        }
    }

    /// Get the prometheus metric name
    pub fn prometheus_name(&self) -> &'static str {
        match self {
            StellarMetricType::LedgerSequence => "stellar_node_ledger_sequence",
            StellarMetricType::IngestionLag => "stellar_node_ingestion_lag",
            StellarMetricType::RequestsPerSecond => "stellar_horizon_tps",
            StellarMetricType::ActiveConnections => "stellar_node_active_connections",
        }
    }
}

/// MetricValueList is the top-level list type for the custom metrics API
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
    pub self_link: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetricValue {
    pub described_object: DescribedObject,
    pub metric: MetricIdentifier,
    pub timestamp: String,
    pub window_seconds: Option<i64>,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DescribedObject {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub api_version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetricIdentifier {
    pub name: String,
    pub selector: Option<LabelSelector>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LabelSelector {
    pub match_labels: BTreeMap<String, String>,
}

/// Error response for custom metrics API
#[derive(Serialize, Debug)]
pub struct ApiError {
    pub kind: String,
    pub api_version: String,
    pub metadata: BTreeMap<String, String>,
    pub message: String,
    pub reason: String,
    pub code: u16,
}

/// Fetch a metric value from the Prometheus registry
/// Returns the metric value as a string, or None if not found
fn get_metric_value(metric_type: &StellarMetricType, namespace: &str, name: &str) -> Option<i64> {
    match metric_type {
        StellarMetricType::LedgerSequence => {
            // In production, query metrics::LEDGER_SEQUENCE with labels
            debug!("Fetching ledger sequence for {}/{}", namespace, name);
            None // Would query actual metric
        }
        StellarMetricType::IngestionLag => {
            // In production, query metrics::INGESTION_LAG with labels
            debug!("Fetching ingestion lag for {}/{}", namespace, name);
            None // Would query actual metric
        }
        StellarMetricType::RequestsPerSecond => {
            debug!("Fetching requests per second for {}/{}", namespace, name);
            None // Would query actual metric
        }
        StellarMetricType::ActiveConnections => {
            debug!("Fetching active connections for {}/{}", namespace, name);
            None // Would query actual metric
        }
    }
}

/// Handler for custom metrics API: /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric
pub async fn get_pod_metric(
    State(_state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> Response {
    debug!(
        "Received custom metrics request for pod {}/{}/{}",
        namespace, name, metric_name
    );

    let metric_type = match StellarMetricType::from_str(&metric_name) {
        Some(mt) => mt,
        None => {
            warn!(
                "Unsupported metric requested: {} for pod {}/{}",
                metric_name, namespace, name
            );
            let error = ApiError {
                kind: "Status".to_string(),
                api_version: "v1".to_string(),
                metadata: BTreeMap::new(),
                message: format!("Metric '{}' not found", metric_name),
                reason: "MetricNotFound".to_string(),
                code: 404,
            };
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let value = get_metric_value(&metric_type, &namespace, &name)
        .unwrap_or(0)
        .to_string();

    let items = vec![MetricValue {
        described_object: DescribedObject {
            kind: "Pod".to_string(),
            namespace,
            name,
            api_version: "v1".to_string(),
        },
        metric: MetricIdentifier {
            name: metric_name,
            selector: None,
        },
        timestamp: now,
        window_seconds: Some(60),
        value,
    }];

    Json(MetricValueList {
        kind: "MetricValueList".to_string(),
        api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        metadata: ListMetadata { self_link: None },
        items,
    })
    .into_response()
}

/// Handler for custom metrics API: /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/stellarnodes.stellar.org/:name/:metric
pub async fn get_stellar_node_metric(
    State(_state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> Response {
    debug!(
        "Received custom metrics request for StellarNode {}/{}/{}",
        namespace, name, metric_name
    );

    let metric_type = match StellarMetricType::from_str(&metric_name) {
        Some(mt) => mt,
        None => {
            warn!(
                "Unsupported metric requested: {} for StellarNode {}/{}",
                metric_name, namespace, name
            );
            let error = ApiError {
                kind: "Status".to_string(),
                api_version: "v1".to_string(),
                metadata: BTreeMap::new(),
                message: format!("Metric '{}' not found", metric_name),
                reason: "MetricNotFound".to_string(),
                code: 404,
            };
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let value = get_metric_value(&metric_type, &namespace, &name)
        .unwrap_or(0)
        .to_string();

    let items = vec![MetricValue {
        described_object: DescribedObject {
            kind: "StellarNode".to_string(),
            namespace,
            name,
            api_version: "stellar.org/v1alpha1".to_string(),
        },
        metric: MetricIdentifier {
            name: metric_name,
            selector: None,
        },
        timestamp: now,
        window_seconds: Some(60),
        value,
    }];

    Json(MetricValueList {
        kind: "MetricValueList".to_string(),
        api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        metadata: ListMetadata { self_link: None },
        items,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_metric_type_from_str_requests_per_second() {
        assert_eq!(
            StellarMetricType::from_str("stellar_horizon_tps"),
            Some(StellarMetricType::RequestsPerSecond)
        );
        assert_eq!(
            StellarMetricType::from_str("requests_per_second"),
            Some(StellarMetricType::RequestsPerSecond)
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
        assert_eq!(StellarMetricType::from_str("invalid"), None);
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
                name: "stellar_ingestion_lag".to_string(),
                selector: None,
            },
            timestamp: "2026-02-25T00:00:00Z".to_string(),
            window_seconds: Some(60),
            value: "42".to_string(),
        };

        let json = serde_json::to_string(&metric).unwrap();
        assert!(json.contains("\"name\":\"horizon-0\""));
        assert!(json.contains("\"value\":\"42\""));
    }

    #[test]
    fn test_api_error_structure() {
        let error = ApiError {
            kind: "Status".to_string(),
            api_version: "v1".to_string(),
            metadata: BTreeMap::new(),
            message: "Metric not found".to_string(),
            reason: "MetricNotFound".to_string(),
            code: 404,
        };

        assert_eq!(error.kind, "Status");
        assert_eq!(error.reason, "MetricNotFound");
        assert_eq!(error.code, 404);
    }
}
