//! Advanced Security Scanning and Vulnerability Management Module
//!
//! Provides automated scanning, runtime monitoring, and automated remediation.

pub mod compliance;
pub mod policy;
pub mod remediation;
pub mod runtime;
pub mod vulnerability;

use serde::{Deserialize, Serialize};

/// Security finding summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub id: String,
    pub component: String,
    pub severity: SecuritySeverity,
    pub description: String,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum SecuritySeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// Security posture report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPosture {
    pub overall_score: f32,
    pub findings: Vec<SecurityFinding>,
    pub compliance_status: bool,
}
