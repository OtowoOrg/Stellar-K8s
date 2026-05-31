//! SOC2 and ISO27001 compliance report generation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::policy_engine::PolicyViolation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Soc2Report {
    pub report_id: String,
    pub generated_at: DateTime<Utc>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub overall_compliance: bool,
    pub trust_service_criteria: Vec<TrustServiceCriteria>,
    pub violations: Vec<PolicyViolation>,
    pub evidence_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustServiceCriteria {
    pub id: String,
    pub name: String,
    pub description: String,
    pub compliant: bool,
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Iso27001Report {
    pub report_id: String,
    pub generated_at: DateTime<Utc>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub overall_compliance: bool,
    pub controls: Vec<IsoControl>,
    pub violations: Vec<PolicyViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsoControl {
    pub id: String,
    pub name: String,
    pub compliant: bool,
    pub notes: String,
}

pub struct Soc2ReportGenerator;

impl Soc2ReportGenerator {
    pub fn generate(
        violations: &[PolicyViolation],
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        evidence_count: usize,
    ) -> Soc2Report {
        let soc2_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.policy_id.starts_with("soc2"))
            .cloned()
            .collect();

        let criteria = vec![
            TrustServiceCriteria {
                id: "CC6".to_string(),
                name: "Logical and Physical Access Controls".to_string(),
                description: "Access to systems is restricted to authorized users".to_string(),
                compliant: !soc2_violations
                    .iter()
                    .any(|v| v.policy_id.contains("access")),
                findings: soc2_violations
                    .iter()
                    .filter(|v| v.policy_id.contains("access"))
                    .map(|v| v.violation_message.clone())
                    .collect(),
            },
            TrustServiceCriteria {
                id: "CC7".to_string(),
                name: "System Operations".to_string(),
                description: "Systems are monitored and incidents are managed".to_string(),
                compliant: !soc2_violations.iter().any(|v| v.policy_id.contains("tls")),
                findings: soc2_violations
                    .iter()
                    .filter(|v| v.policy_id.contains("tls"))
                    .map(|v| v.violation_message.clone())
                    .collect(),
            },
            TrustServiceCriteria {
                id: "CC8".to_string(),
                name: "Change Management".to_string(),
                description: "Changes to systems are authorized and tested".to_string(),
                compliant: !soc2_violations
                    .iter()
                    .any(|v| v.policy_id.contains("resource")),
                findings: soc2_violations
                    .iter()
                    .filter(|v| v.policy_id.contains("resource"))
                    .map(|v| v.violation_message.clone())
                    .collect(),
            },
        ];

        Soc2Report {
            report_id: format!("SOC2-{}", Utc::now().format("%Y%m%d")),
            generated_at: Utc::now(),
            period_start,
            period_end,
            overall_compliance: soc2_violations.is_empty(),
            trust_service_criteria: criteria,
            violations: soc2_violations,
            evidence_count,
        }
    }
}

pub struct Iso27001ReportGenerator;

impl Iso27001ReportGenerator {
    pub fn generate(
        violations: &[PolicyViolation],
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Iso27001Report {
        let iso_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.policy_id.starts_with("iso27001"))
            .cloned()
            .collect();

        let controls = vec![
            IsoControl {
                id: "A.9".to_string(),
                name: "Access Control".to_string(),
                compliant: !iso_violations
                    .iter()
                    .any(|v| v.policy_id.contains("access")),
                notes: "Network policies and RBAC controls".to_string(),
            },
            IsoControl {
                id: "A.12".to_string(),
                name: "Operations Security".to_string(),
                compliant: !iso_violations
                    .iter()
                    .any(|v| v.policy_id.contains("network")),
                notes: "Network policy enforcement".to_string(),
            },
            IsoControl {
                id: "A.14".to_string(),
                name: "System Acquisition, Development and Maintenance".to_string(),
                compliant: !iso_violations.iter().any(|v| v.policy_id.contains("image")),
                notes: "Image pinning and supply chain security".to_string(),
            },
        ];

        Iso27001Report {
            report_id: format!("ISO27001-{}", Utc::now().format("%Y%m%d")),
            generated_at: Utc::now(),
            period_start,
            period_end,
            overall_compliance: iso_violations.is_empty(),
            controls,
            violations: iso_violations,
        }
    }
}
