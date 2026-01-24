//! Custom Metrics API provider implementation
//!
//! Exposes metrics in the format expected by Kubernetes Horizontal Pod Autoscaler
//! for the custom.metrics.k8s.io/v1beta2 API.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::controller::ControllerState;

/// MetricValueList is the top-level list type for the custom metrics API
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MetricValueList {
    pub kind: String,
    pub api_version: String,
    pub metadata: ListMetadata,
    pub items: Vec<MetricValue>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ListMetadata {
    pub self_link: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MetricValue {
    pub described_object: DescribedObject,
    pub metric: MetricIdentifier,
    pub timestamp: String,
    pub window_seconds: Option<i64>,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DescribedObject {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub api_version: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MetricIdentifier {
    pub name: String,
    pub selector: Option<LabelSelector>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LabelSelector {
    pub match_labels: BTreeMap<String, String>,
}

/// Handler for custom metrics API: /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric
pub async fn get_pod_metric(
    State(_state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    // Logic to fetch pod metric from Prometheus family
    // For now, return a mock response to satisfy the structure

    let now = chrono::Utc::now().to_rfc3339();

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
        value: "0".to_string(), // In reality, fetch from INGESTION_LAG or similar
    }];

    Json(MetricValueList {
        kind: "MetricValueList".to_string(),
        api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        metadata: ListMetadata { self_link: None },
        items,
    })
}

/// Handler for custom metrics API: /apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/stellarnodes.stellar.org/:name/:metric
pub async fn get_stellar_node_metric(
    State(_state): State<Arc<ControllerState>>,
    Path((namespace, name, metric_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let now = chrono::Utc::now().to_rfc3339();

    // In a real implementation, we would look up the specific node's metric
    // From our internal INGESTION_LAG family.

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
        value: "0".to_string(),
    }];

    Json(MetricValueList {
        kind: "MetricValueList".to_string(),
        api_version: "custom.metrics.k8s.io/v1beta2".to_string(),
        metadata: ListMetadata { self_link: None },
        items,
    })
}
