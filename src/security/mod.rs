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
//! Advanced Security Scanning and Vulnerability Management Module
//!
//! Provides automated scanning, runtime monitoring, and automated remediation.

pub mod cert_rotation;
pub mod compliance;
pub mod kms;
pub mod policy;
pub mod remediation;
pub mod runtime;
pub mod secret_audit;
pub mod secret_metrics;
pub mod secret_rotation;
pub mod secret_sync;
pub mod vulnerability;

pub use cert_rotation::{
    CertExpiryAlert, CertIssuanceRequest, CertIssuanceResponse, CertRecord, CertRotationController,
    CertRotationError, ExpiryMonitor, ExpiryMonitorConfig, ExpirySeverity, LocalCaBackend,
    PkiBackend, RotationAuditLog, RotationEvent, RotationTrigger, VaultPkiBackend, VaultPkiConfig,
};

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

impl std::fmt::Display for SecuritySeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecuritySeverity::Critical => write!(f, "CRITICAL"),
            SecuritySeverity::High => write!(f, "HIGH"),
            SecuritySeverity::Medium => write!(f, "MEDIUM"),
            SecuritySeverity::Low => write!(f, "LOW"),
        }
    }
}

/// Security posture report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPosture {
    pub overall_score: f32,
    pub findings: Vec<SecurityFinding>,
    pub compliance_status: bool,
}
