// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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
