//! Prometheus metrics for the Stellar-K8s operator
//!
//! # Exported metrics
//! The `/metrics` endpoint (when built with `--features metrics`) exports the following metrics:
//! - `stellar_reconcile_duration_seconds` (histogram): reconcile duration labeled by controller.
//! - `stellar_reconcile_errors_total` (counter): reconcile errors labeled by controller and kind.
//! - `stellar_node_ledger_sequence` (gauge): ledger sequence labeled by namespace/name/node_type/network.
//! - `stellar_node_ingestion_lag` (gauge): ingestion lag labeled by namespace/name/node_type/network.
//! - `stellar_horizon_tps` (gauge): Horizon TPS labeled by namespace/name/node_type/network.
//! - `stellar_node_active_connections` (gauge): active peer connections labeled by namespace/name/node_type/network.

use std::sync::atomic::{AtomicI64, AtomicU64};

use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;

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

/// Gauge tracking requests per second for Horizon nodes
pub static HORIZON_TPS: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking active connections per node
pub static ACTIVE_CONNECTIONS: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking how many ledgers the history archive is behind the validator node.
/// A sustained non-zero value above the configured threshold fires a Prometheus alert.
pub static ARCHIVE_LEDGER_LAG: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Labels for operator reconcile metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ReconcileLabels {
    /// Controller name, e.g. "stellarnode"
    pub controller: String,
}

/// Labels for operator error metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ErrorLabels {
    /// Controller name, e.g. "stellarnode"
    pub controller: String,
    /// Error kind/category, e.g. "kube", "validation", "unknown"
    pub kind: String,
}

/// Histogram tracking reconcile duration (seconds)
pub static RECONCILE_DURATION_SECONDS: Lazy<Family<ReconcileLabels, Histogram>> = Lazy::new(|| {
    fn reconcile_histogram() -> Histogram {
        // 1ms .. ~32s across 16 buckets.
        Histogram::new(exponential_buckets(0.001, 2.0, 16))
    }

    Family::new_with_constructor(reconcile_histogram)
});

/// Counter tracking reconcile errors
pub static RECONCILE_ERRORS_TOTAL: Lazy<Family<ErrorLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Global metrics registry
pub static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let mut registry = Registry::default();

    registry.register(
        "stellar_reconcile_duration_seconds",
        "Duration of reconcile loops in seconds",
        RECONCILE_DURATION_SECONDS.clone(),
    );

    registry.register(
        "stellar_reconcile_errors_total",
        "Total number of reconcile errors",
        RECONCILE_ERRORS_TOTAL.clone(),
    );

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
        "stellar_horizon_tps",
        "Transactions per second for Horizon API nodes",
        HORIZON_TPS.clone(),
    );
    registry.register(
        "stellar_node_active_connections",
        "Number of active peer connections",
        ACTIVE_CONNECTIONS.clone(),
    );
    registry.register(
        "stellar_archive_ledger_lag",
        "Ledgers the history archive is behind the validator node (0 = in-sync)",
        ARCHIVE_LEDGER_LAG.clone(),
    );
    registry
});

/// Observe a reconcile duration in seconds.
pub fn observe_reconcile_duration_seconds(controller: &str, seconds: f64) {
    let labels = ReconcileLabels {
        controller: controller.to_string(),
    };
    RECONCILE_DURATION_SECONDS
        .get_or_create(&labels)
        .observe(seconds);
}

/// Increment the reconcile error counter.
pub fn inc_reconcile_error(controller: &str, kind: &str) {
    let labels = ErrorLabels {
        controller: controller.to_string(),
        kind: kind.to_string(),
    };
    RECONCILE_ERRORS_TOTAL.get_or_create(&labels).inc();
}

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

/// Set the archive ledger lag metric for a node.
///
/// `lag` is the number of ledgers the history archive is behind the validator node.
/// A value above [`crate::controller::archive_health::ARCHIVE_LAG_THRESHOLD`] indicates
/// the archive is significantly stale and a Prometheus alert should fire.
pub fn set_archive_ledger_lag(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    lag: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    ARCHIVE_LEDGER_LAG.get_or_create(&labels).set(lag);
}

/// Update the Horizon TPS metric for a node
pub fn set_horizon_tps(namespace: &str, name: &str, node_type: &str, network: &str, tps: i64) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    HORIZON_TPS.get_or_create(&labels).set(tps);
}

/// Update the active connections metric for a node
pub fn set_active_connections(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    connections: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
    };
    ACTIVE_CONNECTIONS.get_or_create(&labels).set(connections);
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

    #[test]
    fn test_set_ledger_sequence() {
        set_ledger_sequence("default", "test-node", "horizon", "testnet", 12345);
        // Function should not panic
    }

    #[test]
    fn test_set_ingestion_lag() {
        set_ingestion_lag("default", "test-node", "core", "testnet", 5);
        // Function should not panic
    }

    #[test]
    fn test_set_horizon_tps() {
        set_horizon_tps("default", "horizon-1", "horizon", "testnet", 500);
        // Function should not panic
    }

    #[test]
    fn test_set_active_connections() {
        set_active_connections("default", "validator-1", "core", "testnet", 25);
        // Function should not panic
    }

    #[test]
    fn test_node_labels_creation() {
        let labels = NodeLabels {
            namespace: "stellar-system".to_string(),
            name: "horizon-prod".to_string(),
            node_type: "horizon".to_string(),
            network: "mainnet".to_string(),
        };

        assert_eq!(labels.namespace, "stellar-system");
        assert_eq!(labels.name, "horizon-prod");
        assert_eq!(labels.node_type, "horizon");
        assert_eq!(labels.network, "mainnet");
    }

    #[test]
    fn test_registry_registration() {
        // Access the registry to ensure metrics are registered
        let _registry = &*REGISTRY;
        // If this doesn't panic, metrics are properly registered
    }
}
