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
//! Log-based Alerting and Anomaly Detection
//!
//! Fires alerts based on log patterns and error rates.

use crate::logging::analytics::LogPattern;
use std::time::Instant;

pub struct AlertingSystem {
    pub error_threshold_per_min: u64,
    pub last_alert: Option<Instant>,
}

impl AlertingSystem {
    pub fn new(error_threshold_per_min: u64) -> Self {
        Self {
            error_threshold_per_min,
            last_alert: None,
        }
    }

    /// Check if an alert should be fired based on detected patterns
    pub fn check_anomalies(&self, patterns: &[LogPattern]) -> Vec<String> {
        let mut alerts = Vec::new();

        for pattern in patterns {
            // Alert on high frequency errors
            if pattern.message_template.contains("ERROR")
                && pattern.count > self.error_threshold_per_min
            {
                alerts.push(format!(
                    "High error rate detected for pattern: {}",
                    pattern.message_template
                ));
            }

            // Alert on specific critical patterns
            if pattern.message_template.contains("panic")
                || pattern.message_template.contains("FATAL")
            {
                alerts.push(format!(
                    "Critical failure detected: {}",
                    pattern.message_template
                ));
            }
        }

        alerts
    }
}
