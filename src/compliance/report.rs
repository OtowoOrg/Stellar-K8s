//! Automated compliance report generation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::evidence::EvidenceItem;
use super::frameworks::{ClusterComplianceState, ComplianceFramework, ValidationPipeline};
use super::monitor::{ComplianceMonitor, ComplianceStatus, DriftFinding};

/// Full compliance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceReport {
    pub report_id: String,
    pub generated_at: DateTime<Utc>,
    pub operator_version: String,
    pub cluster_state: ClusterComplianceState,
    pub framework_statuses: Vec<ComplianceStatus>,
    pub drift_findings: Vec<DriftFinding>,
    pub evidence: Vec<EvidenceItem>,
    pub overall_compliant: bool,
    pub overall_score_pct: f64,
}

/// Generates compliance reports from cluster state.
pub struct ReportGenerator {
    monitor: ComplianceMonitor,
}

impl ReportGenerator {
    pub fn new(baseline: ClusterComplianceState) -> Self {
        Self {
            monitor: ComplianceMonitor::new(baseline),
        }
    }

    pub fn generate(&self, current: &ClusterComplianceState) -> ComplianceReport {
        let framework_statuses = self.monitor.evaluate(current);
        let drift_findings = self.monitor.detect_drift(current);

        let mut collector = super::evidence::EvidenceCollector::new();
        let mut all_results = Vec::new();
        for framework in ComplianceFramework::all() {
            all_results.extend(ValidationPipeline::validate(framework, current));
        }
        let evidence = collector.collect_from_validation(&all_results);

        let overall_compliant = framework_statuses.iter().all(|s| s.compliant);
        let overall_score = if framework_statuses.is_empty() {
            100.0
        } else {
            framework_statuses.iter().map(|s| s.score_pct).sum::<f64>()
                / framework_statuses.len() as f64
        };

        ComplianceReport {
            report_id: format!("compliance-{}", Utc::now().timestamp()),
            generated_at: Utc::now(),
            operator_version: env!("CARGO_PKG_VERSION").to_string(),
            cluster_state: current.clone(),
            framework_statuses,
            drift_findings,
            evidence,
            overall_compliant,
            overall_score_pct: overall_score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_full_report() {
        let state = ClusterComplianceState {
            mtl_enabled: true,
            audit_logging_enabled: true,
            rbac_enabled: true,
            secrets_encrypted_at_rest: true,
            encryption_in_transit: true,
            pii_scrubbing_enabled: true,
            access_logging_enabled: true,
            vulnerability_scan_enabled: true,
            data_retention_days: 90,
            ..Default::default()
        };
        let gen = ReportGenerator::new(state.clone());
        let report = gen.generate(&state);
        assert!(report.overall_compliant);
        assert!(!report.evidence.is_empty());
        assert_eq!(report.framework_statuses.len(), 3);
    }
}
