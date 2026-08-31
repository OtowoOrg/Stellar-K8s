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
//! Security Compliance Reporting
//!
//! Generates compliance reports for security audits.

use crate::security::SecurityPosture;

pub struct ComplianceReporter;

impl ComplianceReporter {
    pub fn generate_report(posture: &SecurityPosture) -> String {
        let mut report = String::from("# Stellar-K8s Security Compliance Report\n\n");
        report.push_str(&format!("Overall Score: {:.2}\n", posture.overall_score));
        report.push_str(&format!(
            "Compliance Status: {}\n\n",
            if posture.compliance_status {
                "PASSED"
            } else {
                "FAILED"
            }
        ));

        report.push_str("## Active Findings\n\n");
        for finding in &posture.findings {
            report.push_str(&format!(
                "- [{:?}] {}: {}\n",
                finding.severity, finding.id, finding.description
            ));
        }

        report
    }
}
