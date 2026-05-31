//! Capacity Planning Metrics
//!
//! Integration with Prometheus and internal metrics for capacity analysis.

use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use std::sync::atomic::AtomicI64;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CapacityLabels {
    pub resource: String,
    pub node_type: String,
}

/// Projected resource exhaustion timestamp (Unix timestamp)
pub static CAPACITY_EXHAUSTION_PREDICTION: Lazy<Family<CapacityLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Confidence score of capacity predictions (0-100)
pub static CAPACITY_PREDICTION_CONFIDENCE: Lazy<Family<CapacityLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

pub fn record_exhaustion_prediction(
    resource: &str,
    node_type: &str,
    timestamp: i64,
    confidence: i64,
) {
    let labels = CapacityLabels {
        resource: resource.to_string(),
        node_type: node_type.to_string(),
    };
    CAPACITY_EXHAUSTION_PREDICTION
        .get_or_create(&labels)
        .set(timestamp);
    CAPACITY_PREDICTION_CONFIDENCE
        .get_or_create(&labels)
        .set(confidence);
}
