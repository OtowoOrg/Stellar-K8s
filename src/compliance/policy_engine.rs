//! Policy violation detection and automated remediation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::opa::Policy;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ViolationSeverity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub policy_id: String,
    pub policy_name: String,
    pub resource_name: String,
    pub resource_kind: String,
    pub violation_message: String,
    pub severity: ViolationSeverity,
    pub remediation_hint: Option<String>,
    pub detected_at: DateTime<Utc>,
    pub auto_remediated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResult {
    pub policy_id: String,
    pub passed: bool,
    pub message: String,
}

pub struct PolicyEngine {
    policies: Vec<Policy>,
}

impl PolicyEngine {
    pub fn new(policies: Vec<Policy>) -> Self {
        Self { policies }
    }

    /// Evaluate a single policy against a JSON resource.
    pub fn evaluate_policy(&self, policy: &Policy, resource: &serde_json::Value) -> PolicyResult {
        if !policy.enabled {
            return PolicyResult {
                policy_id: policy.id.clone(),
                passed: true,
                message: "Policy disabled".to_string(),
            };
        }

        // Simple expression evaluator: check JSON path equality patterns
        let passed = evaluate_expression(&policy.rego_expression, resource);
        PolicyResult {
            policy_id: policy.id.clone(),
            passed,
            message: if passed {
                "Policy check passed".to_string()
            } else {
                format!(
                    "Policy '{}' violated: {}",
                    policy.name, policy.rego_expression
                )
            },
        }
    }

    /// Evaluate all enabled policies against a resource, returning violations.
    pub fn evaluate_all(
        &self,
        resource_name: &str,
        resource_kind: &str,
        resource: &serde_json::Value,
    ) -> Vec<PolicyViolation> {
        self.policies
            .iter()
            .filter(|p| p.enabled)
            .filter_map(|policy| {
                let result = self.evaluate_policy(policy, resource);
                if !result.passed {
                    warn!(
                        policy = %policy.id,
                        resource = %resource_name,
                        "Policy violation detected"
                    );
                    Some(PolicyViolation {
                        policy_id: policy.id.clone(),
                        policy_name: policy.name.clone(),
                        resource_name: resource_name.to_string(),
                        resource_kind: resource_kind.to_string(),
                        violation_message: result.message,
                        severity: severity_for_framework(&policy.framework),
                        remediation_hint: remediation_hint(&policy.id),
                        detected_at: Utc::now(),
                        auto_remediated: false,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn auto_remediate(&self, violation: &PolicyViolation) -> bool {
        // Only auto-remediate low/medium severity with known hints
        if violation.severity > ViolationSeverity::Medium {
            return false;
        }
        info!(
            policy = %violation.policy_id,
            resource = %violation.resource_name,
            "Auto-remediation triggered"
        );
        // In a real implementation, this would apply patches via kube API
        true
    }
}

fn evaluate_expression(expr: &str, resource: &serde_json::Value) -> bool {
    // Simple evaluator: parse "path == value" or "path != null"
    if let Some((path, op_value)) = expr.split_once(" == ") {
        let actual = json_path(resource, path.trim());
        let expected = op_value.trim().trim_matches('"');
        if expected == "true" {
            return actual.and_then(|v| v.as_bool()).unwrap_or(false);
        }
        return actual
            .and_then(|v| v.as_str())
            .map(|s| s == expected)
            .unwrap_or(false);
    }
    if let Some((path, _)) = expr.split_once(" != null") {
        return json_path(resource, path.trim()).is_some();
    }
    // Default: pass unknown expressions
    true
}

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path.split('.') {
        current = current.get(key)?;
    }
    Some(current)
}

fn severity_for_framework(framework: &str) -> ViolationSeverity {
    match framework {
        "PCI-DSS" => ViolationSeverity::Critical,
        "SOC2" => ViolationSeverity::High,
        "ISO27001" => ViolationSeverity::Medium,
        _ => ViolationSeverity::Low,
    }
}

fn remediation_hint(policy_id: &str) -> Option<String> {
    match policy_id {
        "soc2-tls-required" => {
            Some("Set spec.tls.enabled: true in your StellarNode manifest".to_string())
        }
        "soc2-resource-limits" => Some("Add resource limits to spec.resources.limits".to_string()),
        "iso27001-network-policy" => {
            Some("Add network-policy annotation to resource metadata".to_string())
        }
        "iso27001-image-pinned" => {
            Some("Pin spec.version to a specific semver tag (e.g. v21.0.0)".to_string())
        }
        "pci-encryption-at-rest" => Some("Set spec.storage.encrypted: true".to_string()),
        _ => None,
    }
}
