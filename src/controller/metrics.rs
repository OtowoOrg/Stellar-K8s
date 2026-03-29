//! Prometheus metrics for the Stellar-K8s operator
//!
//! # Exported metrics
//! The `/metrics` endpoint (when built with `--features metrics`) exports the following metrics:
//! - `reconcile_duration_seconds` (histogram): reconcile duration labeled by controller.
//! - `stellar_reconcile_duration_seconds` (histogram): reconcile duration labeled by controller.
//! - `stellar_reconcile_errors_total` (counter): reconcile errors labeled by controller and kind.
//! - `stellar_operator_reconcile_errors_total` (counter): operator reconcile errors labeled by controller and kind.
//! - `stellar_node_ledger_sequence` (gauge): ledger sequence labeled by namespace/name/node_type/network/hardware_generation.
//! - `stellar_node_ingestion_lag` (gauge): ingestion lag labeled by namespace/name/node_type/network/hardware_generation.
//! - `stellar_horizon_tps` (gauge): Horizon TPS labeled by namespace/name/node_type/network/hardware_generation.
//! - `stellar_node_active_connections` (gauge): active peer connections labeled by namespace/name/node_type/network/hardware_generation.

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

/// Labels for reactive updates
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ReactiveLabels {
    pub namespace: String,
    pub name: String,
}

/// Counter tracking reactive status updates
pub static REACTIVE_STATUS_UPDATES_TOTAL: Lazy<Family<ReactiveLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Counter tracking API polls avoided due to reactive updates
pub static API_POLLS_AVOIDED_TOTAL: Lazy<Family<ReactiveLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Labels for the ledger sequence metric
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct NodeLabels {
    pub namespace: String,
    pub name: String,
    pub node_type: String,
    pub network: String,
    pub hardware_generation: String,
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

/// Gauge tracking number of critical nodes in the quorum
pub static QUORUM_CRITICAL_NODES: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking minimum quorum overlap count
pub static QUORUM_MIN_OVERLAP: Lazy<Family<NodeLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Histogram tracking consensus latency per validator
pub static QUORUM_CONSENSUS_LATENCY_MS: Lazy<Family<NodeLabels, Histogram>> = Lazy::new(|| {
    fn latency_histogram() -> Histogram {
        // 1ms .. ~32s across 16 buckets
        Histogram::new(exponential_buckets(1.0, 2.0, 16))
    }
    Family::new_with_constructor(latency_histogram)
});

/// Gauge tracking quorum fragility score (0.0 to 1.0)
pub static QUORUM_FRAGILITY_SCORE: Lazy<Family<NodeLabels, Gauge<f64, AtomicU64>>> =
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

/// Labels for Soroban-specific metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SorobanLabels {
    pub namespace: String,
    pub name: String,
    pub network: String,
    pub contract_id: String,
}

/// Labels for contract invocation metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ContractInvocationLabels {
    pub namespace: String,
    pub name: String,
    pub network: String,
    pub contract_type: String,
}

/// Labels for transaction result metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TransactionResultLabels {
    pub namespace: String,
    pub name: String,
    pub network: String,
    pub result: String, // "success" or "failed"
}

/// Histogram tracking reconcile duration (seconds)
pub static RECONCILE_DURATION_SECONDS: Lazy<Family<ReconcileLabels, Histogram>> = Lazy::new(|| {
    fn reconcile_histogram() -> Histogram {
        // 1ms .. ~32s across 16 buckets.
        Histogram::new(exponential_buckets(0.001, 2.0, 16))
    }

    Family::new_with_constructor(reconcile_histogram)
});

/// Histogram tracking reconcile duration (seconds) under the non-prefixed metric name.
pub static RAW_RECONCILE_DURATION_SECONDS: Lazy<Family<ReconcileLabels, Histogram>> =
    Lazy::new(|| {
        fn reconcile_histogram() -> Histogram {
            // 1ms .. ~32s across 16 buckets.
            Histogram::new(exponential_buckets(0.001, 2.0, 16))
        }

        Family::new_with_constructor(reconcile_histogram)
    });

/// Counter tracking reconcile errors
pub static RECONCILE_ERRORS_TOTAL: Lazy<Family<ErrorLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Counter tracking operator-level reconcile errors
pub static OPERATOR_RECONCILE_ERRORS_TOTAL: Lazy<Family<ErrorLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Soroban-specific metrics
/// Histogram tracking Wasm execution time in microseconds
pub static WASM_EXECUTION_DURATION_MICROSECONDS: Lazy<Family<SorobanLabels, Histogram>> =
    Lazy::new(|| {
        fn wasm_histogram() -> Histogram {
            // 1µs .. ~65ms across 16 buckets
            Histogram::new(exponential_buckets(1.0, 2.0, 16))
        }
        Family::new_with_constructor(wasm_histogram)
    });

/// Histogram tracking contract storage fees in stroops
pub static CONTRACT_STORAGE_FEE_STROOPS: Lazy<Family<SorobanLabels, Histogram>> = Lazy::new(|| {
    fn fee_histogram() -> Histogram {
        // 1 stroop .. ~65k stroops across 16 buckets
        Histogram::new(exponential_buckets(1.0, 2.0, 16))
    }
    Family::new_with_constructor(fee_histogram)
});

/// Gauge tracking Wasm VM memory usage in bytes
pub static WASM_VM_MEMORY_BYTES: Lazy<Family<SorobanLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking CPU instructions per contract invocation
pub static CONTRACT_INVOCATION_CPU_INSTRUCTIONS: Lazy<
    Family<SorobanLabels, Gauge<i64, AtomicI64>>,
> = Lazy::new(Family::default);

/// Gauge tracking memory bytes per contract invocation
pub static CONTRACT_INVOCATION_MEMORY_BYTES: Lazy<Family<SorobanLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Counter tracking contract invocations by type
pub static CONTRACT_INVOCATIONS_TOTAL: Lazy<
    Family<ContractInvocationLabels, Counter<u64, AtomicU64>>,
> = Lazy::new(Family::default);

/// Counter tracking transaction results (success/failure)
pub static TRANSACTION_RESULT_TOTAL: Lazy<
    Family<TransactionResultLabels, Counter<u64, AtomicU64>>,
> = Lazy::new(Family::default);

/// Counter tracking host function calls
pub static HOST_FUNCTION_CALLS_TOTAL: Lazy<Family<SorobanLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Labels for DR drill metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct DRDrillLabels {
    pub namespace: String,
    pub name: String,
    pub status: String, // "success", "failed", "rolled_back"
}

/// Histogram tracking DR drill execution time in milliseconds
pub static DR_DRILL_EXECUTION_TIME_MS: Lazy<Family<DRDrillLabels, Histogram>> = Lazy::new(|| {
    fn drill_histogram() -> Histogram {
        // 100ms .. ~100s across 16 buckets
        Histogram::new(exponential_buckets(100.0, 2.0, 16))
    }
    Family::new_with_constructor(drill_histogram)
});

/// Counter tracking DR drill executions
pub static DR_DRILL_EXECUTIONS_TOTAL: Lazy<Family<DRDrillLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Gauge tracking Time to Recovery (TTR) in milliseconds
pub static DR_DRILL_TIME_TO_RECOVERY_MS: Lazy<Family<DRDrillLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Global metrics registry
pub static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let mut registry = Registry::default();

    registry.register(
        "reconcile_duration_seconds",
        "Duration of reconcile loops in seconds",
        RAW_RECONCILE_DURATION_SECONDS.clone(),
    );

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
        "stellar_operator_reconcile_errors_total",
        "Total number of operator reconcile errors",
        OPERATOR_RECONCILE_ERRORS_TOTAL.clone(),
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

    // Register reactive update metrics (from HEAD)
    registry.register(
        "stellar_reactive_status_updates_total",
        "Total number of reactive status updates from DB triggers",
        REACTIVE_STATUS_UPDATES_TOTAL.clone(),
    );
    registry.register(
        "stellar_api_polls_avoided_total",
        "Total number of API health check polls avoided",
        API_POLLS_AVOIDED_TOTAL.clone(),
    );

    // Register quorum analysis metrics (from feat/spec branch)
    registry.register(
        "stellar_quorum_critical_nodes",
        "Number of critical nodes in the quorum whose failure would break consensus",
        QUORUM_CRITICAL_NODES.clone(),
    );
    registry.register(
        "stellar_quorum_min_overlap",
        "Minimum overlap count between quorum slices",
        QUORUM_MIN_OVERLAP.clone(),
    );
    registry.register(
        "stellar_quorum_consensus_latency_ms",
        "Consensus latency per validator in milliseconds",
        QUORUM_CONSENSUS_LATENCY_MS.clone(),
    );
    registry.register(
        "stellar_quorum_fragility_score",
        "Quorum fragility score (0.0 = resilient, 1.0 = fragile)",
        QUORUM_FRAGILITY_SCORE.clone(),
    );

    // Register Soroban-specific metrics
    registry.register(
        "soroban_rpc_wasm_execution_duration_microseconds",
        "Wasm host function execution time in microseconds",
        WASM_EXECUTION_DURATION_MICROSECONDS.clone(),
    );
    registry.register(
        "soroban_rpc_contract_storage_fee_stroops",
        "Contract storage fees in stroops",
        CONTRACT_STORAGE_FEE_STROOPS.clone(),
    );
    registry.register(
        "soroban_rpc_wasm_vm_memory_bytes",
        "Wasm VM memory usage in bytes",
        WASM_VM_MEMORY_BYTES.clone(),
    );
    registry.register(
        "soroban_rpc_contract_invocation_cpu_instructions",
        "CPU instructions consumed per contract invocation",
        CONTRACT_INVOCATION_CPU_INSTRUCTIONS.clone(),
    );
    registry.register(
        "soroban_rpc_contract_invocation_memory_bytes",
        "Memory bytes consumed per contract invocation",
        CONTRACT_INVOCATION_MEMORY_BYTES.clone(),
    );
    registry.register(
        "soroban_rpc_contract_invocations_total",
        "Total number of contract invocations by type",
        CONTRACT_INVOCATIONS_TOTAL.clone(),
    );
    registry.register(
        "soroban_rpc_transaction_result_total",
        "Total number of transactions by result (success/failed)",
        TRANSACTION_RESULT_TOTAL.clone(),
    );
    registry.register(
        "soroban_rpc_host_function_calls_total",
        "Total number of host function calls",
        HOST_FUNCTION_CALLS_TOTAL.clone(),
    );

    // Register DR drill metrics
    registry.register(
        "stellar_dr_drill_execution_time_ms",
        "DR drill execution time in milliseconds",
        DR_DRILL_EXECUTION_TIME_MS.clone(),
    );
    registry.register(
        "stellar_dr_drill_executions_total",
        "Total number of DR drill executions",
        DR_DRILL_EXECUTIONS_TOTAL.clone(),
    );
    registry.register(
        "stellar_dr_drill_time_to_recovery_ms",
        "Time to Recovery (TTR) for DR drills in milliseconds",
        DR_DRILL_TIME_TO_RECOVERY_MS.clone(),
    );

    // Register operator build-info and leader metrics
    registry.register(
        "stellar_operator_info",
        "Operator build information (version, git_sha, rust_version); always 1",
        OPERATOR_INFO.clone(),
    );
    registry.register(
        "stellar_operator_leader_status",
        "1 if this operator instance is the current leader, 0 otherwise",
        OPERATOR_LEADER_STATUS.clone(),
    );
    registry.register(
        "stellar_operator_uptime_seconds",
        "Total uptime of the operator process in seconds",
        OPERATOR_UPTIME_SECONDS.clone(),
    );

    registry.register(
        "stellar_operator_ready",
        "1 if the operator is ready (K8s watch healthy and first reconcile complete), 0 otherwise",
        OPERATOR_READY_STATUS.clone(),
    );

    registry
});

/// Observe a reconcile duration in seconds.
pub fn observe_reconcile_duration_seconds(controller: &str, seconds: f64) {
    let labels = ReconcileLabels {
        controller: controller.to_string(),
    };
    RAW_RECONCILE_DURATION_SECONDS
        .get_or_create(&labels)
        .observe(seconds);
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

/// Increment the operator reconcile error counter.
pub fn inc_operator_reconcile_error(controller: &str, kind: &str) {
    let labels = ErrorLabels {
        controller: controller.to_string(),
        kind: kind.to_string(),
    };
    OPERATOR_RECONCILE_ERRORS_TOTAL.get_or_create(&labels).inc();
}

/// Increment reactive status updates counter
pub fn inc_reactive_status_update(namespace: &str, name: &str) {
    let labels = ReactiveLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
    };
    REACTIVE_STATUS_UPDATES_TOTAL.get_or_create(&labels).inc();
}

/// Increment API polls avoided counter
pub fn inc_api_polls_avoided(namespace: &str, name: &str) {
    let labels = ReactiveLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
    };
    API_POLLS_AVOIDED_TOTAL.get_or_create(&labels).inc();
}

/// Update the ledger sequence metric for a node
pub fn set_ledger_sequence(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    sequence: u64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    LEDGER_SEQUENCE.get_or_create(&labels).set(sequence as i64);
}

/// Update the ledger sequence metric for a node with Differential Privacy
pub fn set_ledger_sequence_with_dp(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    sequence: u64,
) {
    let noise = generate_laplace_noise(DP_EPSILON, DP_SENSITIVITY);
    let val = (sequence as f64 + noise) as i64;

    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    LEDGER_SEQUENCE.get_or_create(&labels).set(val);
}

/// Update the ingestion lag metric for a node
pub fn set_ingestion_lag(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    lag: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    INGESTION_LAG.get_or_create(&labels).set(lag);
}

/// Update the ingestion lag metric for a node with Differential Privacy
pub fn set_ingestion_lag_with_dp(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    lag: i64,
) {
    let noise = generate_laplace_noise(DP_EPSILON, DP_SENSITIVITY);
    let val = (lag as f64 + noise) as i64;

    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
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
    hardware_generation: &str,
    lag: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    ARCHIVE_LEDGER_LAG.get_or_create(&labels).set(lag);
}

/// Update the Horizon TPS metric for a node
pub fn set_horizon_tps(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    tps: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    HORIZON_TPS.get_or_create(&labels).set(tps);
}

/// Update the active connections metric for a node
pub fn set_active_connections(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    connections: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
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

/// Observe Wasm execution duration in microseconds
pub fn observe_wasm_execution_duration(
    namespace: &str,
    name: &str,
    network: &str,
    contract_id: &str,
    duration_us: f64,
) {
    let labels = SorobanLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_id: contract_id.to_string(),
    };
    WASM_EXECUTION_DURATION_MICROSECONDS
        .get_or_create(&labels)
        .observe(duration_us);
}

/// Observe contract storage fee in stroops
pub fn observe_contract_storage_fee(
    namespace: &str,
    name: &str,
    network: &str,
    contract_id: &str,
    fee_stroops: f64,
) {
    let labels = SorobanLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_id: contract_id.to_string(),
    };
    CONTRACT_STORAGE_FEE_STROOPS
        .get_or_create(&labels)
        .observe(fee_stroops);
}

/// Set Wasm VM memory usage in bytes
pub fn set_wasm_vm_memory(
    namespace: &str,
    name: &str,
    network: &str,
    contract_id: &str,
    memory_bytes: i64,
) {
    let labels = SorobanLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_id: contract_id.to_string(),
    };
    WASM_VM_MEMORY_BYTES
        .get_or_create(&labels)
        .set(memory_bytes);
}

/// Set CPU instructions per contract invocation
pub fn set_contract_invocation_cpu(
    namespace: &str,
    name: &str,
    network: &str,
    contract_id: &str,
    cpu_instructions: i64,
) {
    let labels = SorobanLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_id: contract_id.to_string(),
    };
    CONTRACT_INVOCATION_CPU_INSTRUCTIONS
        .get_or_create(&labels)
        .set(cpu_instructions);
}

/// Set memory bytes per contract invocation
pub fn set_contract_invocation_memory(
    namespace: &str,
    name: &str,
    network: &str,
    contract_id: &str,
    memory_bytes: i64,
) {
    let labels = SorobanLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_id: contract_id.to_string(),
    };
    CONTRACT_INVOCATION_MEMORY_BYTES
        .get_or_create(&labels)
        .set(memory_bytes);
}

/// Increment contract invocation counter
pub fn inc_contract_invocation(namespace: &str, name: &str, network: &str, contract_type: &str) {
    let labels = ContractInvocationLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_type: contract_type.to_string(),
    };
    CONTRACT_INVOCATIONS_TOTAL.get_or_create(&labels).inc();
}

/// Increment transaction result counter
pub fn inc_transaction_result(namespace: &str, name: &str, network: &str, success: bool) {
    let labels = TransactionResultLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        result: if success {
            "success".to_string()
        } else {
            "failed".to_string()
        },
    };
    TRANSACTION_RESULT_TOTAL.get_or_create(&labels).inc();
}

/// Increment host function call counter
pub fn inc_host_function_call(namespace: &str, name: &str, network: &str, contract_id: &str) {
    let labels = SorobanLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        network: network.to_string(),
        contract_id: contract_id.to_string(),
    };
    HOST_FUNCTION_CALLS_TOTAL.get_or_create(&labels).inc();
}

/// Set the number of critical nodes in the quorum
pub fn set_quorum_critical_nodes(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    count: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    QUORUM_CRITICAL_NODES.get_or_create(&labels).set(count);
}

/// Set the minimum quorum overlap count
pub fn set_quorum_min_overlap(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    overlap: i64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    QUORUM_MIN_OVERLAP.get_or_create(&labels).set(overlap);
}

/// Observe consensus latency in milliseconds
pub fn observe_consensus_latency(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    latency_ms: f64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    QUORUM_CONSENSUS_LATENCY_MS
        .get_or_create(&labels)
        .observe(latency_ms);
}

/// Set the quorum fragility score (0.0 to 1.0)
pub fn set_quorum_fragility_score(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    hardware_generation: &str,
    score: f64,
) {
    let labels = NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: hardware_generation.to_string(),
    };
    QUORUM_FRAGILITY_SCORE.get_or_create(&labels).set(score);
}

/// Record a DR drill execution
pub fn observe_dr_drill_execution(
    namespace: &str,
    name: &str,
    status: &str,
    execution_time_ms: f64,
) {
    let labels = DRDrillLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        status: status.to_string(),
    };
    DR_DRILL_EXECUTION_TIME_MS
        .get_or_create(&labels)
        .observe(execution_time_ms);
    DR_DRILL_EXECUTIONS_TOTAL.get_or_create(&labels).inc();
}

/// Set the Time to Recovery (TTR) for a DR drill
pub fn set_dr_drill_time_to_recovery(namespace: &str, name: &str, status: &str, ttr_ms: i64) {
    let labels = DRDrillLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        status: status.to_string(),
    };
    DR_DRILL_TIME_TO_RECOVERY_MS
        .get_or_create(&labels)
        .set(ttr_ms);
}

// ============================================================================
// Operator build-info and leader metrics (Issue #301)
// ============================================================================

/// Labels for the operator info gauge.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct OperatorInfoLabels {
    /// Semantic version from `CARGO_PKG_VERSION`.
    pub version: String,
    /// Git commit SHA from `GIT_SHA` build env var.
    pub git_sha: String,
    /// Rust compiler version from `RUST_VERSION` build env var.
    pub rust_version: String,
}

/// Gauge that is always set to `1` and carries version/build labels.
/// Equivalent to the common `build_info` pattern used by many Prometheus exporters.
pub static OPERATOR_INFO: Lazy<Family<OperatorInfoLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Gauge tracking whether this instance is the current leader (1 = leader, 0 = follower).
pub static OPERATOR_LEADER_STATUS: Lazy<Gauge<i64, AtomicI64>> = Lazy::new(Gauge::default);

/// Counter tracking operator uptime in seconds since process start.
pub static OPERATOR_UPTIME_SECONDS: Lazy<Counter<u64, AtomicU64>> = Lazy::new(Counter::default);

/// Gauge tracking whether the operator is ready (1 = ready, 0 = not ready).
pub static OPERATOR_READY_STATUS: Lazy<Gauge<i64, AtomicI64>> = Lazy::new(Gauge::default);

/// Initialise the `stellar_operator_info` gauge with build-time labels.
/// Call once at startup after the registry is first accessed.
pub fn init_operator_info() {
    let labels = OperatorInfoLabels {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: option_env!("GIT_SHA").unwrap_or("unknown").to_string(),
        rust_version: option_env!("RUST_VERSION").unwrap_or("unknown").to_string(),
    };
    OPERATOR_INFO.get_or_create(&labels).set(1);
}

/// Update the leader-status gauge. Call from the leader-election loop.
pub fn set_leader_status(is_leader: bool) {
    OPERATOR_LEADER_STATUS.set(if is_leader { 1 } else { 0 });
}

/// Increment the uptime counter by `delta_secs`. Call from a periodic task.
pub fn inc_uptime_seconds(delta_secs: u64) {
    OPERATOR_UPTIME_SECONDS.inc_by(delta_secs);
}

/// Set the operator readiness status gauge.
pub fn set_ready_status(ready: bool) {
    OPERATOR_READY_STATUS.set(if ready { 1 } else { 0 });
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
        set_ledger_sequence_with_dp("default", "node-1", "core", "public", "unknown", 100);
        set_ingestion_lag_with_dp("default", "node-1", "core", "public", "unknown", 5);

        // We can't easily check the value in the global registry without exposing it more,
        // but this ensures the code path runs.
    }

    #[test]
    fn test_set_ledger_sequence() {
        set_ledger_sequence(
            "default",
            "test-node",
            "horizon",
            "testnet",
            "Intel Icelake",
            12345,
        );
        // Function should not panic
    }

    #[test]
    fn test_set_ingestion_lag() {
        set_ingestion_lag(
            "default",
            "test-node",
            "core",
            "testnet",
            "Intel Icelake",
            5,
        );
        // Function should not panic
    }

    #[test]
    fn test_set_horizon_tps() {
        set_horizon_tps(
            "default",
            "horizon-1",
            "horizon",
            "testnet",
            "Intel Icelake",
            500,
        );
        // Function should not panic
    }

    #[test]
    fn test_set_active_connections() {
        set_active_connections(
            "default",
            "validator-1",
            "core",
            "testnet",
            "Intel Icelake",
            25,
        );
        // Function should not panic
    }

    #[test]
    fn test_node_labels_creation() {
        let labels = NodeLabels {
            namespace: "stellar-system".to_string(),
            name: "horizon-prod".to_string(),
            node_type: "horizon".to_string(),
            network: "mainnet".to_string(),
            hardware_generation: "Intel Icelake".to_string(),
        };

        assert_eq!(labels.namespace, "stellar-system");
        assert_eq!(labels.name, "horizon-prod");
        assert_eq!(labels.node_type, "horizon");
        assert_eq!(labels.network, "mainnet");
        assert_eq!(labels.hardware_generation, "Intel Icelake");
    }

    #[test]
    fn test_registry_registration() {
        // Access the registry to ensure metrics are registered
        let _registry = &*REGISTRY;
        // If this doesn't panic, metrics are properly registered
    }

    #[test]
    fn test_soroban_wasm_execution_duration() {
        observe_wasm_execution_duration("default", "soroban-1", "testnet", "contract123", 1500.0);
        // Function should not panic
    }

    #[test]
    fn test_soroban_contract_storage_fee() {
        observe_contract_storage_fee("default", "soroban-1", "testnet", "contract123", 1000.0);
        // Function should not panic
    }

    #[test]
    fn test_soroban_wasm_vm_memory() {
        set_wasm_vm_memory("default", "soroban-1", "testnet", "contract123", 1048576);
        // Function should not panic
    }

    #[test]
    fn test_soroban_contract_invocation_cpu() {
        set_contract_invocation_cpu("default", "soroban-1", "testnet", "contract123", 50000);
        // Function should not panic
    }

    #[test]
    fn test_soroban_contract_invocation_memory() {
        set_contract_invocation_memory("default", "soroban-1", "testnet", "contract123", 524288);
        // Function should not panic
    }

    #[test]
    fn test_soroban_contract_invocation_counter() {
        inc_contract_invocation("default", "soroban-1", "testnet", "token");
        inc_contract_invocation("default", "soroban-1", "testnet", "defi");
        // Function should not panic
    }

    #[test]
    fn test_soroban_transaction_result() {
        inc_transaction_result("default", "soroban-1", "testnet", true);
        inc_transaction_result("default", "soroban-1", "testnet", false);
        // Function should not panic
    }

    #[test]
    fn test_soroban_host_function_calls() {
        inc_host_function_call("default", "soroban-1", "testnet", "contract123");
        // Function should not panic
    }

    #[test]
    fn test_soroban_labels_creation() {
        let labels = SorobanLabels {
            namespace: "stellar-system".to_string(),
            name: "soroban-prod".to_string(),
            network: "mainnet".to_string(),
            contract_id: "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC".to_string(),
        };

        assert_eq!(labels.namespace, "stellar-system");
        assert_eq!(labels.name, "soroban-prod");
        assert_eq!(labels.network, "mainnet");
        assert!(labels.contract_id.starts_with("CDLZFC"));
    }

    #[test]
    fn test_inc_operator_reconcile_error() {
        // Test that incrementing operator reconcile error doesn't panic
        inc_operator_reconcile_error("stellarnode", "kube");
        inc_operator_reconcile_error("stellarnode", "validation");
        inc_operator_reconcile_error("stellarnode", "config");
        // Function should not panic
    }

    #[test]
    fn test_operator_reconcile_errors_total_registered() {
        // Verify the new metric is registered in the global registry
        let _registry = &*REGISTRY;
        // Access the metric to ensure it's initialized
        let labels = ErrorLabels {
            controller: "stellarnode".to_string(),
            kind: "test".to_string(),
        };
        let counter = OPERATOR_RECONCILE_ERRORS_TOTAL.get_or_create(&labels);
        counter.inc();
        // If this doesn't panic, the metric is properly registered and functional
    }

    #[test]
    fn test_operator_reconcile_error_labels() {
        // Test that error labels are created correctly for operator errors
        let labels = ErrorLabels {
            controller: "stellarnode".to_string(),
            kind: "unknown".to_string(),
        };

        assert_eq!(labels.controller, "stellarnode");
        assert_eq!(labels.kind, "unknown");

        // Test with different error kinds
        inc_operator_reconcile_error("stellarnode", "kube");
        inc_operator_reconcile_error("stellarnode", "validation");
        inc_operator_reconcile_error("stellarnode", "config");
        inc_operator_reconcile_error("stellarnode", "unknown");
        // Function should not panic with various error kinds
    }
}
