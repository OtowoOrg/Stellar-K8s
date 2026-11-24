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
//! Cardinality budget definitions and tracking

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Cardinality budget custom resource
#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "observability.stellar.org",
    version = "v1alpha1",
    kind = "CardinalityBudget",
    plural = "cardinalitybudgets",
    shortname = "cb",
    namespaced,
    status = "BudgetStatus",
    derive = "PartialEq",
    printcolumn = r#"{"name":"Team","type":"string","jsonPath":".spec.team"}"#,
    printcolumn = r#"{"name":"Limit","type":"integer","jsonPath":".spec.limit"}"#,
    printcolumn = r#"{"name":"Current","type":"integer","jsonPath":".status.currentCardinality"}"#,
    printcolumn = r#"{"name":"Utilization","type":"string","jsonPath":".status.utilizationPercent"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#
)]
pub struct BudgetSpec {
    /// Team identifier
    pub team: String,
    /// Maximum allowed cardinality (unique label combinations)
    pub limit: u64,
    /// Warning threshold percentage (0-100)
    #[serde(default = "default_warning_threshold")]
    pub warning_threshold: u8,
    /// Critical threshold percentage (0-100)
    #[serde(default = "default_critical_threshold")]
    pub critical_threshold: u8,
    /// Metric selectors this budget applies to
    #[serde(default)]
    pub metric_selectors: Vec<MetricSelector>,
    /// Label keys that count towards cardinality
    #[serde(default = "default_cardinality_labels")]
    pub cardinality_labels: Vec<String>,
    /// Action when budget exceeded
    #[serde(default)]
    pub enforcement_action: EnforcementAction,
    /// Quarantine duration for offending series
    #[serde(default = "default_quarantine_duration")]
    pub quarantine_duration: String,
}

fn default_warning_threshold() -> u8 { 70 }
fn default_critical_threshold() -> u8 { 90 }
fn default_cardinality_labels() -> Vec<String> {
    vec!["team".to_string(), "service".to_string(), "environment".to_string()]
}
fn default_quarantine_duration() -> String { "1h".to_string() }

/// Metric selector for budget scope
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricSelector {
    /// Metric name pattern (glob)
    pub metric_pattern: String,
    /// Label matchers
    #[serde(default)]
    pub label_matchers: BTreeMap<String, String>,
}

/// Enforcement action when budget exceeded
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum EnforcementAction {
    #[default]
    Warn,
    Quarantine,
    Drop,
}

/// Budget phase
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum BudgetPhase {
    #[default]
    Healthy,
    Warning,
    Critical,
    Exceeded,
}

/// Budget status
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatus {
    /// Current cardinality count
    pub current_cardinality: u64,
    /// Utilization percentage
    pub utilization_percent: String,
    /// Current phase
    pub phase: BudgetPhase,
    /// Number of quarantined series
    pub quarantined_series: u64,
    /// Number of dropped series
    pub dropped_series: u64,
    /// Last updated timestamp
    pub last_updated: DateTime<Utc>,
    /// Per-metric breakdown
    #[serde(default)]
    pub metric_breakdown: BTreeMap<String, MetricCardinalityStatus>,
    /// Conditions
    #[serde(default)]
    pub conditions: Vec<BudgetCondition>,
}

/// Per-metric cardinality status
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricCardinalityStatus {
    pub metric_name: String,
    pub current_cardinality: u64,
    pub limit: u64,
    pub quarantined_series: u64,
    pub dropped_series: u64,
}

/// Budget condition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BudgetCondition {
    pub type_: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<DateTime<Utc>>,
}

/// Team budget aggregate
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamBudget {
    pub team: String,
    pub total_limit: u64,
    pub total_current: u64,
    pub budgets: Vec<BudgetSummary>,
}

/// Budget summary for reporting
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSummary {
    pub name: String,
    pub namespace: String,
    pub limit: u64,
    pub current: u64,
    pub phase: BudgetPhase,
}

/// Cardinality budget aggregate for team-level tracking
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CardinalityBudgetAggregate {
    pub team: String,
    pub budgets: BTreeMap<String, BudgetSpec>,
    pub aggregate_limit: u64,
    pub aggregate_current: u64,
}

impl CardinalityBudgetAggregate {
    pub fn new(team: String, aggregate_limit: u64) -> Self {
        Self {
            team,
            budgets: BTreeMap::new(),
            aggregate_limit,
            aggregate_current: 0,
        }
    }

    pub fn add_budget(&mut self, name: String, spec: BudgetSpec) {
        self.budgets.insert(name, spec);
    }

    pub fn utilization(&self) -> f64 {
        if self.aggregate_limit == 0 {
            0.0
        } else {
            self.aggregate_current as f64 / self.aggregate_limit as f64 * 100.0
        }
    }
}

/// Duration serialization helper
pub mod duration_serde {
    use chrono::Duration;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}s", duration.num_seconds()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.ends_with('s') {
            let secs = s.trim_end_matches('s').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::seconds(secs))
        } else if s.ends_with('m') {
            let mins = s.trim_end_matches('m').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::minutes(mins))
        } else if s.ends_with('h') {
            let hours = s.trim_end_matches('h').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::hours(hours))
        } else if s.ends_with('d') {
            let days = s.trim_end_matches('d').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::days(days))
        } else {
            Err(serde::de::Error::custom("duration must end with s, m, h, or d"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_budget_spec_defaults() {
        let spec = BudgetSpec {
            team: "test-team".to_string(),
            limit: 10000,
            warning_threshold: default_warning_threshold(),
            critical_threshold: default_critical_threshold(),
            metric_selectors: vec![],
            cardinality_labels: default_cardinality_labels(),
            enforcement_action: EnforcementAction::default(),
            quarantine_duration: default_quarantine_duration(),
        };
        
        assert_eq!(spec.warning_threshold, 70);
        assert_eq!(spec.critical_threshold, 90);
        assert_eq!(spec.enforcement_action, EnforcementAction::Warn);
    }

    #[test]
    fn test_team_budget_utilization() {
        let mut budget = CardinalityBudgetAggregate::new("test-team".to_string(), 10000);
        budget.aggregate_current = 5000;
        assert_eq!(budget.utilization(), 50.0);
        
        budget.aggregate_current = 10000;
        assert_eq!(budget.utilization(), 100.0);
    }

    #[test]
    fn test_enforcement_action_serialization() {
        let action = EnforcementAction::Quarantine;
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, "\"Quarantine\"");
    }
}