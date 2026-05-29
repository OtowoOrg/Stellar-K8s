//! Prometheus metrics for resource optimization.

#[cfg(feature = "metrics")]
use once_cell::sync::Lazy;
#[cfg(feature = "metrics")]
use prometheus_client::encoding::EncodeLabelSet;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::counter::Counter;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::family::Family;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::gauge::Gauge;
#[cfg(feature = "metrics")]
use std::sync::atomic::{AtomicI64, AtomicU64};

#[cfg(feature = "metrics")]
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct OptimizationLabels {
    pub namespace: String,
    pub node: String,
}

/// Cost savings percentage from optimization (0-100).
#[cfg(feature = "metrics")]
pub static OPTIMIZATION_COST_SAVINGS_PCT: Lazy<Family<OptimizationLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// SLA compliance score (0-100).
#[cfg(feature = "metrics")]
pub static OPTIMIZATION_SLA_COMPLIANCE: Lazy<Family<OptimizationLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Forecast prediction accuracy MAPE (lower is better).
#[cfg(feature = "metrics")]
pub static OPTIMIZATION_PREDICTION_MAPE: Lazy<Family<OptimizationLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Total optimization cycles executed.
#[cfg(feature = "metrics")]
pub static OPTIMIZATION_CYCLES_TOTAL: Lazy<Family<OptimizationLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Record optimization metrics after a cycle.
#[cfg(feature = "metrics")]
pub fn record_optimization_metrics(
    namespace: &str,
    node: &str,
    cost_savings_pct: f64,
    sla_compliance: f64,
    prediction_mape: f64,
) {
    let labels = OptimizationLabels {
        namespace: namespace.to_string(),
        node: node.to_string(),
    };
    OPTIMIZATION_COST_SAVINGS_PCT
        .get_or_create(&labels)
        .set(cost_savings_pct as i64);
    OPTIMIZATION_SLA_COMPLIANCE
        .get_or_create(&labels)
        .set(sla_compliance as i64);
    OPTIMIZATION_PREDICTION_MAPE
        .get_or_create(&labels)
        .set(prediction_mape as i64);
    OPTIMIZATION_CYCLES_TOTAL.get_or_create(&labels).inc();
}

#[cfg(not(feature = "metrics"))]
pub fn record_optimization_metrics(
    _namespace: &str,
    _node: &str,
    _cost_savings_pct: f64,
    _sla_compliance: f64,
    _prediction_mape: f64,
) {
}
