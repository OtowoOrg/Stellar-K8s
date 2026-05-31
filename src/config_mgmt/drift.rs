//! Configuration Drift Detection
//!
//! Detects and remediates drift between desired state and actual cluster configuration.

use crate::crd::StellarNodeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub field: String,
    pub desired: String,
    pub actual: String,
    pub severity: DriftSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DriftSeverity {
    Critical,
    Major,
    Minor,
}

pub struct DriftDetector;

impl DriftDetector {
    /// Detects drift between the desired spec and the actual runtime configuration
    pub fn detect_drift(desired: &StellarNodeSpec, actual: &StellarNodeSpec) -> Vec<DriftReport> {
        let mut drifts = Vec::new();

        if desired.version != actual.version {
            drifts.push(DriftReport {
                field: "version".to_string(),
                desired: desired.version.clone(),
                actual: actual.version.clone(),
                severity: DriftSeverity::Critical,
            });
        }

        if desired.resources.requests != actual.resources.requests {
            drifts.push(DriftReport {
                field: "resources.requests".to_string(),
                desired: format!("{:?}", desired.resources.requests),
                actual: format!("{:?}", actual.resources.requests),
                severity: DriftSeverity::Major,
            });
        }

        drifts
    }

    /// Determines if automatic remediation should be applied
    pub fn should_remediate(drifts: &[DriftReport]) -> bool {
        drifts
            .iter()
            .any(|d| matches!(d.severity, DriftSeverity::Critical | DriftSeverity::Major))
    }
}
