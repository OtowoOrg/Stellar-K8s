//! Prometheus metrics for the Stellar-K8s operator

use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use std::sync::atomic::AtomicI64;

/// Labels for the ledger sequence metric
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct NodeLabels {
    pub namespace: String,
    pub name: String,
    pub node_type: String,
    pub network: String,
}

/// Gauge tracking ledger sequence per node
pub static LEDGER_SEQUENCE: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking ledger ingestion lag per node
pub static INGESTION_LAG: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking reconciliation count (privacy protected)
pub static RECONCILIATION_COUNT: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Internal store for actual (not noisy) reconciliation counts
static ACTUAL_RECONCILIATION_COUNTS: Lazy<
    std::sync::Mutex<std::collections::HashMap<String, u64>>,
> = Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Privacy module for differential privacy
use crate::telemetry::privacy::{PrivacyAwareMetric, PrivacyConfig};
static PRIVACY_ENGINE: Lazy<PrivacyAwareMetric> = Lazy::new(|| {
    PrivacyAwareMetric::new(PrivacyConfig {
        epsilon: 0.1,
        sensitivity: 1.0,
    })
});

/// Global metrics registry
pub static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let mut registry = Registry::default();
    registry.register(
        "stellar_node_ledger_sequence",
        "Current ledger sequence number of the Stellar node",
        LEDGER_SEQUENCE.clone(),
    );
    registry.register(
        "stellar_node_ingestion_lag",
        "Lag between latest network ledger and node ledger",
        INGESTION_LAG.clone(),
    );
    registry.register(
        "stellar_operator_reconciliations",
        "Total number of reconciliations (Differential Privacy applied)",
        RECONCILIATION_COUNT.clone(),
    );
    registry
});

/// Update the ledger sequence metric for a node
pub fn set_ledger_sequence(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    sequence: u64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    LEDGER_SEQUENCE.get_or_create(&labels).set(sequence as i64);
}

/// Update the ingestion lag metric for a node
pub fn set_ingestion_lag(namespace: &str, name: &str, node_type: &str, network: &str, lag: i64) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    INGESTION_LAG.get_or_create(&labels).set(lag);
}

/// Update the reconciliation count with differential privacy
pub fn set_reconciliation_count(namespace: &str, name: &str, node_type: &str, network: &str) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };

    // Create a unique key for the counter
    let key = format!("{}:{}:{}:{}", namespace, name, node_type, network);

    let current_val = {
        let mut counts = ACTUAL_RECONCILIATION_COUNTS.lock().unwrap();
        let count = counts.entry(key).or_insert(0);
        *count += 1;
        *count
    };

    // Apply differential privacy noise to the CUMULATIVE count
    let protected_val = PRIVACY_ENGINE.protect_count(current_val);
    RECONCILIATION_COUNT
        .get_or_create(&labels)
        .set(protected_val as i64);
}
