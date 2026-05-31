//! Continuous compliance monitoring and configuration drift detection.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::frameworks::{ClusterComplianceState, ComplianceFramework, RuleResult, ValidationPipeline};

/// Overall compliance status for a framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceStatus {
    pub framework: ComplianceFramework,
    pub compliant: bool,
    pub score_pct: f64,
    pub passed_rules: u32,
    pub total_rules: u32,
    pub failed_rules: Vec<RuleResult>,
    pub evaluated_at: DateTime<Utc>,
}

/// Configuration drift finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftFinding {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub severity: String,
    pub detected_at: DateTime<Utc>,
}

/// Continuous compliance monitor.
pub struct ComplianceMonitor {
    baseline: ClusterComplianceState,
}

impl ComplianceMonitor {
    pub fn new(baseline: ClusterComplianceState) -> Self {
        Self { baseline }
    }

    /// Evaluate compliance status for all frameworks.
    pub fn evaluate(&self, current: &ClusterComplianceState) -> Vec<ComplianceStatus> {
        ComplianceFramework::all()
            .into_iter()
            .map(|framework| {
                let results = ValidationPipeline::validate(framework, current);
                let passed = results.iter().filter(|r| r.passed).count() as u32;
                let total = results.len() as u32;
                let failed: Vec<_> = results.iter().filter(|r| !r.passed).cloned().collect();

                ComplianceStatus {
                    framework,
                    compliant: failed.is_empty(),
                    score_pct: if total > 0 {
                        (passed as f64 / total as f64) * 100.0
                    } else {
                        100.0
                    },
                    passed_rules: passed,
                    total_rules: total,
                    failed_rules: failed,
                    evaluated_at: Utc::now(),
                }
            })
            .collect()
    }

    /// Detect configuration drift from baseline.
    pub fn detect_drift(&self, current: &ClusterComplianceState) -> Vec<DriftFinding> {
        let mut findings = Vec::new();
        let now = Utc::now();

        macro_rules! check_bool {
            ($field:ident, $name:expr) => {
                if self.baseline.$field != current.$field {
                    findings.push(DriftFinding {
                        field: $name.to_string(),
                        expected: format!("{}", self.baseline.$field),
                        actual: format!("{}", current.$field),
                        severity: "critical".to_string(),
                        detected_at: now,
                    });
                }
            };
        }

        check_bool!(mtl_enabled, "mtl_enabled");
        check_bool!(audit_logging_enabled, "audit_logging_enabled");
        check_bool!(rbac_enabled, "rbac_enabled");
        check_bool!(secrets_encrypted_at_rest, "secrets_encrypted_at_rest");
        check_bool!(encryption_in_transit, "encryption_in_transit");
        check_bool!(pii_scrubbing_enabled, "pii_scrubbing_enabled");

        if self.baseline.data_retention_days != current.data_retention_days {
            findings.push(DriftFinding {
                field: "data_retention_days".to_string(),
                expected: self.baseline.data_retention_days.to_string(),
                actual: current.data_retention_days.to_string(),
                severity: "major".to_string(),
                detected_at: now,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_drift_finds_mtl_change() {
        let baseline = ClusterComplianceState {
            mtl_enabled: true,
            ..Default::default()
        };
        let current = ClusterComplianceState {
            mtl_enabled: false,
            ..Default::default()
        };
        let monitor = ComplianceMonitor::new(baseline);
        let drift = monitor.detect_drift(&current);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].field, "mtl_enabled");
    }

    #[test]
    fn evaluate_compliance_score() {
        let state = ClusterComplianceState {
            rbac_enabled: true,
            mtl_enabled: true,
            audit_logging_enabled: true,
            ..Default::default()
        };
        let monitor = ComplianceMonitor::new(state.clone());
        let statuses = monitor.evaluate(&state);
        let soc2 = statuses.iter().find(|s| s.framework == ComplianceFramework::Soc2).unwrap();
        assert!(soc2.compliant);
        assert_eq!(soc2.score_pct, 100.0);
    }
}
