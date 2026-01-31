//! Prometheus metrics for the Stellar-K8s operator

use std::sync::atomic::AtomicI64;

use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use rand::prelude::*;
use rand::prelude::*;

const DP_EPSILON: f64 = 1.0; // Privacy budget
const DP_SENSITIVITY: f64 = 1.0; // Sensitivity of the metric

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

/// Update the ledger sequence metric for a node with Differential Privacy
pub fn set_ledger_sequence_with_dp(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    sequence: u64,
) {
    let noise = generate_laplace_noise(DP_EPSILON, DP_SENSITIVITY);
    let val = (sequence as f64 + noise) as i64;

    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    LEDGER_SEQUENCE.get_or_create(&labels).set(val);
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

/// Update the ingestion lag metric for a node with Differential Privacy
pub fn set_ingestion_lag_with_dp(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    lag: i64,
) {
    let noise = generate_laplace_noise(DP_EPSILON, DP_SENSITIVITY);
    let val = (lag as f64 + noise) as i64;

    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    INGESTION_LAG.get_or_create(&labels).set(val);
}

fn generate_laplace_noise(epsilon: f64, sensitivity: f64) -> f64 {
    let scale = sensitivity / epsilon;
    let u: f64 = rand::random::<f64>() - 0.5;
    let sign = if u < 0.0 { -1.0 } else { 1.0 };
    // Laplace(0, b) sample = -b * sgn(u) * ln(1 - 2|u|)
    -scale * sign * (1.0 - 2.0 * u.abs()).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_laplace_noise() {
        let noise = generate_laplace_noise(1.0, 1.0);
        // It's random, so we can't assert exact value, but we can check it's finite
        assert!(noise.is_finite());
    }

    #[test]
    fn test_dp_metrics_update() {
        // Just verify that calling the function doesn't panic
        set_ledger_sequence_with_dp("default", "node-1", "core", "public", 100);
        set_ingestion_lag_with_dp("default", "node-1", "core", "public", 5);

        // We can't easily check the value in the global registry without exposing it more,
        // but this ensures the code path runs.
    }
}
