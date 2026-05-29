//! SLA-aware optimization constraints for resource recommendations.

use serde::{Deserialize, Serialize};

/// SLA constraint definition for a workload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlaConstraint {
    /// Maximum acceptable p99 latency in milliseconds.
    pub max_p99_latency_ms: f64,
    /// Minimum availability percentage (0-100).
    pub min_availability_pct: f64,
    /// Maximum allowed error rate (0-1).
    pub max_error_rate: f64,
    /// Target cost savings percentage vs baseline.
    #[serde(default)]
    pub target_cost_savings_pct: f64,
}

impl Default for SlaConstraint {
    fn default() -> Self {
        Self {
            max_p99_latency_ms: 500.0,
            min_availability_pct: 99.9,
            max_error_rate: 0.01,
            target_cost_savings_pct: 10.0,
        }
    }
}

/// Current SLA metrics observed from Prometheus.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlaMetrics {
    pub p99_latency_ms: f64,
    pub availability_pct: f64,
    pub error_rate: f64,
    pub current_replicas: i32,
    pub forecast_tps: f64,
}

/// An SLA violation detected during optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlaViolation {
    pub constraint: String,
    pub observed: f64,
    pub limit: f64,
    pub severity: String,
}

/// Evaluates SLA compliance and adjusts resource recommendations.
pub struct SlaEvaluator;

impl SlaEvaluator {
    /// Check current metrics against SLA constraints.
    pub fn evaluate(constraints: &SlaConstraint, metrics: &SlaMetrics) -> Vec<SlaViolation> {
        let mut violations = Vec::new();

        if metrics.p99_latency_ms > constraints.max_p99_latency_ms {
            violations.push(SlaViolation {
                constraint: "max_p99_latency_ms".to_string(),
                observed: metrics.p99_latency_ms,
                limit: constraints.max_p99_latency_ms,
                severity: "critical".to_string(),
            });
        }

        if metrics.availability_pct < constraints.min_availability_pct {
            violations.push(SlaViolation {
                constraint: "min_availability_pct".to_string(),
                observed: metrics.availability_pct,
                limit: constraints.min_availability_pct,
                severity: "critical".to_string(),
            });
        }

        if metrics.error_rate > constraints.max_error_rate {
            violations.push(SlaViolation {
                constraint: "max_error_rate".to_string(),
                observed: metrics.error_rate,
                limit: constraints.max_error_rate,
                severity: "warning".to_string(),
            });
        }

        violations
    }

    /// Adjust recommended replicas to satisfy SLA under forecast load.
    pub fn adjust_replicas(
        constraints: &SlaConstraint,
        metrics: &SlaMetrics,
        base_replicas: i32,
        tps_per_replica: f64,
        max_replicas: i32,
    ) -> i32 {
        let mut replicas = base_replicas;

        // Scale up if latency SLA is violated
        if metrics.p99_latency_ms > constraints.max_p99_latency_ms {
            replicas = (replicas as f64 * 1.25).ceil() as i32;
        }

        // Scale up if forecast exceeds capacity
        if tps_per_replica > 0.0 {
            let needed = ((metrics.forecast_tps * 1.2) / tps_per_replica).ceil() as i32;
            replicas = replicas.max(needed);
        }

        // Scale up if availability is below target
        if metrics.availability_pct < constraints.min_availability_pct {
            replicas += 1;
        }

        replicas.clamp(1, max_replicas)
    }

    /// Returns true if all SLA constraints are satisfied.
    pub fn is_compliant(constraints: &SlaConstraint, metrics: &SlaMetrics) -> bool {
        Self::evaluate(constraints, metrics).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_latency_violation() {
        let constraints = SlaConstraint::default();
        let metrics = SlaMetrics {
            p99_latency_ms: 800.0,
            availability_pct: 99.95,
            error_rate: 0.005,
            ..Default::default()
        };
        let violations = SlaEvaluator::evaluate(&constraints, &metrics);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].constraint, "max_p99_latency_ms");
    }

    #[test]
    fn adjust_replicas_scales_for_forecast() {
        let constraints = SlaConstraint::default();
        let metrics = SlaMetrics {
            forecast_tps: 5000.0,
            p99_latency_ms: 100.0,
            availability_pct: 99.99,
            error_rate: 0.001,
            current_replicas: 2,
        };
        let adjusted = SlaEvaluator::adjust_replicas(&constraints, &metrics, 2, 1000.0, 20);
        assert!(adjusted >= 6);
    }
}
