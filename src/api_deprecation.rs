// Copyright 2026 Stellar-K8s Contributors
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
//! API Deprecation Enforcement with Automated Consumer Migration Reports.
//!
//! Supports staged enforcement (`Audit`, `Warn`, `Deny`) switchable without
//! restarting the admission webhook, plus per-namespace usage attribution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Staged enforcement mode matching `--enforcement-mode`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EnforcementMode {
    Audit,
    Warn,
    Deny,
}

/// Deprecated API rule with replacement target and sunset deadline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeprecatedApiRule {
    pub api_group: String,
    pub deprecated_version: String,
    pub kind: String,
    pub replacement_version: String,
    pub sunset_at: DateTime<Utc>,
}

impl DeprecatedApiRule {
    pub fn api_key(&self) -> String {
        format!("{}/{}/{}", self.api_group, self.deprecated_version, self.kind)
    }
}

/// Admission evaluation outcome for a resource request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeprecationDecision {
    pub allowed: bool,
    pub mode: EnforcementMode,
    pub warning: Option<String>,
    pub denial_reason: Option<String>,
}

/// Per-namespace consumer migration entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceMigrationEntry {
    pub namespace: String,
    pub deprecated_api: String,
    pub replacement_version: String,
    pub sunset_at: DateTime<Utc>,
    pub request_count: u64,
    pub last_seen_at: DateTime<Utc>,
}

/// Automated consumer migration report across all namespaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerMigrationReport {
    pub generated_at: DateTime<Utc>,
    pub enforcement_mode: EnforcementMode,
    pub total_deprecated_requests: u64,
    pub consumers: Vec<NamespaceMigrationEntry>,
}

/// Evaluates API deprecation rules and generates consumer migration reports.
pub struct ApiDeprecationEnforcer {
    mode: Arc<RwLock<EnforcementMode>>,
    rules: Arc<RwLock<HashMap<String, DeprecatedApiRule>>>,
    usage: Arc<RwLock<HashMap<(String, String), NamespaceMigrationEntry>>>,
}

impl ApiDeprecationEnforcer {
    pub fn new(initial_mode: EnforcementMode, rules: Vec<DeprecatedApiRule>) -> Self {
        let rule_map = rules.into_iter().map(|r| (r.api_key(), r)).collect();
        Self {
            mode: Arc::new(RwLock::new(initial_mode)),
            rules: Arc::new(RwLock::new(rule_map)),
            usage: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Dynamically flip enforcement mode without restarting the webhook binary.
    pub async fn set_enforcement_mode(&self, mode: EnforcementMode) {
        *self.mode.write().await = mode;
    }

    pub async fn enforcement_mode(&self) -> EnforcementMode {
        *self.mode.read().await
    }

    /// Evaluate an incoming API request and attribute usage to `namespace`.
    pub async fn evaluate(
        &self,
        namespace: &str,
        api_group: &str,
        version: &str,
        kind: &str,
    ) -> DeprecationDecision {
        let mode = *self.mode.read().await;
        let key = format!("{api_group}/{version}/{kind}");
        let rule = self.rules.read().await.get(&key).cloned();

        let Some(rule) = rule else {
            return DeprecationDecision {
                allowed: true,
                mode,
                warning: None,
                denial_reason: None,
            };
        };

        let mut usage = self.usage.write().await;
        let entry = usage
            .entry((namespace.to_string(), key.clone()))
            .or_insert_with(|| NamespaceMigrationEntry {
                namespace: namespace.to_string(),
                deprecated_api: key.clone(),
                replacement_version: rule.replacement_version.clone(),
                sunset_at: rule.sunset_at,
                request_count: 0,
                last_seen_at: Utc::now(),
            });
        entry.request_count += 1;
        entry.last_seen_at = Utc::now();

        let msg = format!(
            "API {key} is deprecated and sunsets at {}; migrate to {}",
            rule.sunset_at.to_rfc3339(),
            rule.replacement_version
        );

        match mode {
            EnforcementMode::Audit => DeprecationDecision {
                allowed: true,
                mode,
                warning: None,
                denial_reason: None,
            },
            EnforcementMode::Warn => DeprecationDecision {
                allowed: true,
                mode,
                warning: Some(msg),
                denial_reason: None,
            },
            EnforcementMode::Deny => DeprecationDecision {
                allowed: false,
                mode,
                warning: Some(msg.clone()),
                denial_reason: Some(msg),
            },
        }
    }

    /// Generate a consumer migration report covering 100% of observed deprecated APIs.
    pub async fn generate_migration_report(&self) -> ConsumerMigrationReport {
        let mode = *self.mode.read().await;
        let consumers: Vec<NamespaceMigrationEntry> =
            self.usage.read().await.values().cloned().collect();
        let total_deprecated_requests = consumers.iter().map(|c| c.request_count).sum();
        ConsumerMigrationReport {
            generated_at: Utc::now(),
            enforcement_mode: mode,
            total_deprecated_requests,
            consumers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn staged_enforcement_flips_without_restart_and_attributes_usage() {
        let rule = DeprecatedApiRule {
            api_group: "stellar.org".into(),
            deprecated_version: "v1alpha1".into(),
            kind: "StellarNode".into(),
            replacement_version: "v1beta1".into(),
            sunset_at: Utc::now() + Duration::days(30),
        };
        let enforcer = ApiDeprecationEnforcer::new(EnforcementMode::Warn, vec![rule]);

        let d1 = enforcer
            .evaluate("team-payments", "stellar.org", "v1alpha1", "StellarNode")
            .await;
        assert!(d1.allowed);
        assert!(d1.warning.is_some());

        enforcer.set_enforcement_mode(EnforcementMode::Deny).await;
        let d2 = enforcer
            .evaluate("team-payments", "stellar.org", "v1alpha1", "StellarNode")
            .await;
        assert!(!d2.allowed);
        assert!(d2.denial_reason.is_some());

        let report = enforcer.generate_migration_report().await;
        assert_eq!(report.total_deprecated_requests, 2);
        assert_eq!(report.consumers[0].namespace, "team-payments");
    }
}
