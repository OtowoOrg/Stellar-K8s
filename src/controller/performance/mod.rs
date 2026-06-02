//! Performance Optimization Framework controller (epic #868).
//!
//! This module evaluates observed performance metrics for a
//! [`StellarPerformance`](crate::crd::StellarPerformance) resource against its
//! declared budgets (SLOs) and detects regressions relative to a rolling
//! baseline. The logic here is intentionally pure and side-effect free so it
//! can be unit tested without a live cluster; a reconciler wires it to the
//! Kubernetes API and the metrics source.
//!
//! Scope of this slice: budget evaluation, regression detection, rolling
//! baseline maintenance, and phase derivation. Continuous benchmarking,
//! profiling, query optimization, caching, and load testing are tracked as
//! follow-up work in the epic.

use crate::crd::stellar_performance::{
    BudgetResult, PerformanceBudgets, PerformancePhase, PerformanceSample, RegressionPolicy,
};

/// Smoothing factor for the exponentially-weighted rolling baseline.
/// A new sample contributes `BASELINE_ALPHA`; the prior baseline keeps the
/// rest. 0.2 favours stability while still tracking sustained shifts.
const BASELINE_ALPHA: f64 = 0.2;

/// Metric identifiers used in [`BudgetResult::metric`].
pub mod metric {
    pub const P95_LATENCY_MS: &str = "p95LatencyMs";
    pub const THROUGHPUT_TPS: &str = "throughputTps";
    pub const ERROR_RATE_PCT: &str = "errorRatePct";
}

/// Evaluate an observed sample against the configured budgets.
///
/// Returns one [`BudgetResult`] per SLO. Latency and error rate are
/// upper-bounded (observed must be `<=` budget); throughput is lower-bounded
/// (observed must be `>=` budget).
pub fn evaluate_budgets(
    sample: &PerformanceSample,
    budgets: &PerformanceBudgets,
) -> Vec<BudgetResult> {
    vec![
        BudgetResult {
            metric: metric::P95_LATENCY_MS.to_string(),
            within_budget: sample.p95_latency_ms <= budgets.max_p95_latency_ms,
            observed: sample.p95_latency_ms,
            budget: budgets.max_p95_latency_ms,
        },
        BudgetResult {
            metric: metric::THROUGHPUT_TPS.to_string(),
            within_budget: sample.throughput_tps >= budgets.min_throughput_tps,
            observed: sample.throughput_tps,
            budget: budgets.min_throughput_tps,
        },
        BudgetResult {
            metric: metric::ERROR_RATE_PCT.to_string(),
            within_budget: sample.error_rate_pct <= budgets.max_error_rate_pct,
            observed: sample.error_rate_pct,
            budget: budgets.max_error_rate_pct,
        },
    ]
}

/// Whether every budget result is within budget.
pub fn all_within_budget(results: &[BudgetResult]) -> bool {
    results.iter().all(|r| r.within_budget)
}

/// Detect a regression of `current` relative to `baseline` under `policy`.
///
/// A regression is reported when p95 latency rises more than
/// `max_latency_increase_pct` above the baseline, or throughput falls more
/// than `max_throughput_decrease_pct` below it. A zero/negative baseline value
/// is treated as having no comparable history for that metric.
pub fn detect_regression(
    current: &PerformanceSample,
    baseline: &PerformanceSample,
    policy: &RegressionPolicy,
) -> bool {
    let latency_regressed = baseline.p95_latency_ms > 0.0 && {
        let increase_pct =
            (current.p95_latency_ms - baseline.p95_latency_ms) / baseline.p95_latency_ms * 100.0;
        increase_pct > policy.max_latency_increase_pct
    };

    let throughput_regressed = baseline.throughput_tps > 0.0 && {
        let decrease_pct =
            (baseline.throughput_tps - current.throughput_tps) / baseline.throughput_tps * 100.0;
        decrease_pct > policy.max_throughput_decrease_pct
    };

    latency_regressed || throughput_regressed
}

/// Fold a new sample into the rolling baseline.
///
/// With no prior baseline, the first sample becomes the baseline. Otherwise an
/// exponentially-weighted moving average smooths short-lived noise.
pub fn update_baseline(
    baseline: Option<&PerformanceSample>,
    current: &PerformanceSample,
) -> PerformanceSample {
    match baseline {
        None => current.clone(),
        Some(prev) => PerformanceSample {
            p95_latency_ms: ewma(prev.p95_latency_ms, current.p95_latency_ms),
            throughput_tps: ewma(prev.throughput_tps, current.throughput_tps),
            error_rate_pct: ewma(prev.error_rate_pct, current.error_rate_pct),
        },
    }
}

fn ewma(prev: f64, sample: f64) -> f64 {
    BASELINE_ALPHA * sample + (1.0 - BASELINE_ALPHA) * prev
}

/// Derive the overall phase from budget results and regression status.
///
/// A regression takes precedence over a budget breach because it signals a
/// trend even when absolute values are still nominally within budget.
pub fn derive_phase(results: &[BudgetResult], regressed: bool) -> PerformancePhase {
    if regressed {
        PerformancePhase::Regressed
    } else if all_within_budget(results) {
        PerformancePhase::WithinBudget
    } else {
        PerformancePhase::BudgetExceeded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budgets() -> PerformanceBudgets {
        PerformanceBudgets {
            max_p95_latency_ms: 200.0,
            min_throughput_tps: 100.0,
            max_error_rate_pct: 1.0,
        }
    }

    fn sample(p95: f64, tps: f64, err: f64) -> PerformanceSample {
        PerformanceSample {
            p95_latency_ms: p95,
            throughput_tps: tps,
            error_rate_pct: err,
        }
    }

    #[test]
    fn healthy_sample_is_within_budget() {
        let results = evaluate_budgets(&sample(120.0, 150.0, 0.2), &budgets());
        assert!(all_within_budget(&results));
        assert_eq!(derive_phase(&results, false), PerformancePhase::WithinBudget);
    }

    #[test]
    fn high_latency_breaks_budget() {
        let results = evaluate_budgets(&sample(350.0, 150.0, 0.2), &budgets());
        assert!(!all_within_budget(&results));
        let latency = results
            .iter()
            .find(|r| r.metric == metric::P95_LATENCY_MS)
            .unwrap();
        assert!(!latency.within_budget);
        assert_eq!(
            derive_phase(&results, false),
            PerformancePhase::BudgetExceeded
        );
    }

    #[test]
    fn low_throughput_breaks_budget() {
        let results = evaluate_budgets(&sample(120.0, 50.0, 0.2), &budgets());
        let tps = results
            .iter()
            .find(|r| r.metric == metric::THROUGHPUT_TPS)
            .unwrap();
        assert!(!tps.within_budget);
    }

    #[test]
    fn high_error_rate_breaks_budget() {
        let results = evaluate_budgets(&sample(120.0, 150.0, 5.0), &budgets());
        let err = results
            .iter()
            .find(|r| r.metric == metric::ERROR_RATE_PCT)
            .unwrap();
        assert!(!err.within_budget);
    }

    #[test]
    fn budget_boundary_is_inclusive() {
        // Exactly at each threshold must pass.
        let results = evaluate_budgets(&sample(200.0, 100.0, 1.0), &budgets());
        assert!(all_within_budget(&results));
    }

    #[test]
    fn latency_spike_is_a_regression() {
        let base = sample(100.0, 150.0, 0.2);
        // +20% latency vs a 10% policy.
        let now = sample(120.0, 150.0, 0.2);
        assert!(detect_regression(&now, &base, &RegressionPolicy::default()));
    }

    #[test]
    fn throughput_drop_is_a_regression() {
        let base = sample(100.0, 150.0, 0.2);
        // -20% throughput vs a 10% policy.
        let now = sample(100.0, 120.0, 0.2);
        assert!(detect_regression(&now, &base, &RegressionPolicy::default()));
    }

    #[test]
    fn small_movement_is_not_a_regression() {
        let base = sample(100.0, 150.0, 0.2);
        // +5% latency, -5% throughput — both under the 10% policy.
        let now = sample(105.0, 142.5, 0.2);
        assert!(!detect_regression(&now, &base, &RegressionPolicy::default()));
    }

    #[test]
    fn regression_takes_precedence_in_phase() {
        let within = evaluate_budgets(&sample(120.0, 150.0, 0.2), &budgets());
        assert!(all_within_budget(&within));
        // Even with budgets met, a detected regression dominates.
        assert_eq!(derive_phase(&within, true), PerformancePhase::Regressed);
    }

    #[test]
    fn first_sample_becomes_baseline() {
        let s = sample(100.0, 150.0, 0.2);
        assert_eq!(update_baseline(None, &s), s);
    }

    #[test]
    fn baseline_smooths_toward_new_sample() {
        let base = sample(100.0, 100.0, 0.0);
        let now = sample(200.0, 200.0, 1.0);
        let next = update_baseline(Some(&base), &now);
        // EWMA with alpha=0.2: 0.2*200 + 0.8*100 = 120.
        assert!((next.p95_latency_ms - 120.0).abs() < 1e-9);
        assert!((next.throughput_tps - 120.0).abs() < 1e-9);
    }
}
