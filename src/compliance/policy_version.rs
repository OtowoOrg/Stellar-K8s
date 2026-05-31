//! Policy versioning and change tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::opa::Policy;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersion {
    pub policy_id: String,
    pub version: String,
    pub rego_expression: String,
    pub diff_summary: String,
    pub changed_by: String,
    pub changed_at: DateTime<Utc>,
}

pub struct PolicyVersionStore {
    history: HashMap<String, Vec<PolicyVersion>>,
}

impl PolicyVersionStore {
    pub fn new() -> Self {
        Self {
            history: HashMap::new(),
        }
    }

    pub fn commit_version(&mut self, policy: &Policy, changed_by: &str, diff_summary: &str) {
        let version = PolicyVersion {
            policy_id: policy.id.clone(),
            version: policy.version.clone(),
            rego_expression: policy.rego_expression.clone(),
            diff_summary: diff_summary.to_string(),
            changed_by: changed_by.to_string(),
            changed_at: Utc::now(),
        };
        self.history
            .entry(policy.id.clone())
            .or_default()
            .push(version);
    }

    pub fn get_history(&self, policy_id: &str) -> Vec<&PolicyVersion> {
        self.history
            .get(policy_id)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    pub fn diff_versions(&self, policy_id: &str, v1: &str, v2: &str) -> Option<String> {
        let history = self.history.get(policy_id)?;
        let ver1 = history.iter().find(|v| v.version == v1)?;
        let ver2 = history.iter().find(|v| v.version == v2)?;
        Some(format!(
            "Policy '{}' diff ({} -> {}):\n- Old: {}\n+ New: {}",
            policy_id, v1, v2, ver1.rego_expression, ver2.rego_expression
        ))
    }

    pub fn latest_version(&self, policy_id: &str) -> Option<&PolicyVersion> {
        self.history.get(policy_id)?.last()
    }
}

impl Default for PolicyVersionStore {
    fn default() -> Self {
        Self::new()
    }
}
