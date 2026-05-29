//! Security Compliance Reporting
//!
//! Generates compliance reports for security audits.

use crate::security::{SecurityPosture, SecurityFinding};
use serde::{Deserialize, Serialize};

pub struct ComplianceReporter;

impl ComplianceReporter {
    pub fn generate_report(posture: &SecurityPosture) -> String {
        let mut report = String::from("# Stellar-K8s Security Compliance Report\n\n");
        report.push_str(&format!("Overall Score: {:.2}\n", posture.overall_score));
        report.push_str(&format!("Compliance Status: {}\n\n", if posture.compliance_status { "PASSED" } else { "FAILED" }));
        
        report.push_str("## Active Findings\n\n");
        for finding in &posture.findings {
            report.push_str(&format!("- [{}] {}: {}\n", finding.severity, finding.id, finding.description));
        }
        
        report
    }
}
