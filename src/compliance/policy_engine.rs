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

/// Outcome of evaluating a single declarative control probe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeEvaluation {
    /// Identifier of the probe (typically the framework control ID).
    pub probe_id: String,
    /// Whether the probe expression evaluated to `true` for the given inputs.
    pub passed: bool,
    /// Human-readable explanation of the outcome.
    pub message: String,
    /// Timestamp at which the probe was evaluated.
    pub evaluated_at: DateTime<Utc>,
}

/// A declarative control probe (CEL/Kyverno-style) that can be evaluated
/// against cluster resources and metrics to collect compliance evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlProbe {
    /// Stable identifier, usually the framework control ID (e.g. `SOC2-CC6.1`).
    pub id: String,
    /// The policy expression to evaluate, using the same syntax as policies.
    pub expression: String,
    /// Optional JSON path into the metrics document used as the probe input.
    /// When `None`, the resource document is evaluated directly.
    pub metrics_path: Option<String>,
}

impl ControlProbe {
    pub fn new(id: impl Into<String>, expression: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            expression: expression.into(),
            metrics_path: None,
        }
    }

    pub fn with_metrics_path(mut self, path: impl Into<String>) -> Self {
        self.metrics_path = Some(path.into());
        self
    }
}

/// Stable API used by the evidence collector to evaluate declarative probes.
pub struct ProbeEvaluator;

impl ProbeEvaluator {
    /// Evaluate a probe against a resource document and an optional metrics
    /// document. The probe passes only if the expression holds for both the
    /// resource and (when configured) the referenced metrics value.
    pub fn evaluate(
        probe: &ControlProbe,
        resource: &serde_json::Value,
        metrics: Option<&serde_json::Value>,
    ) -> ProbeEvaluation {
        let resource_passed = evaluate_expression(&probe.expression, resource);

        let metrics_passed = match (&probe.metrics_path, metrics) {
            (Some(path), Some(metrics_doc)) => match json_path(metrics_doc, path) {
                Some(value) => evaluate_expression(&probe.expression, value),
                None => false,
            },
            (Some(_), None) => false,
            (None, _) => true,
        };

        let passed = resource_passed && metrics_passed;
        let message = if passed {
            format!("Probe '{}' passed", probe.id)
        } else if !resource_passed {
            format!(
                "Probe '{}' failed: expression '{}' not satisfied by resource",
                probe.id, probe.expression
            )
        } else {
            format!(
                "Probe '{}' failed: expression '{}' not satisfied by metrics at '{}'",
                probe.id,
                probe.expression,
                probe.metrics_path.as_deref().unwrap_or("")
            )
        };

        ProbeEvaluation {
            probe_id: probe.id.clone(),
            passed,
            message,
            evaluated_at: Utc::now(),
        }
    }

    /// Evaluate a batch of probes, returning one evaluation per probe.
    pub fn evaluate_all(
        probes: &[ControlProbe],
        resource: &serde_json::Value,
        metrics: Option<&serde_json::Value>,
    ) -> Vec<ProbeEvaluation> {
        probes
            .iter()
            .map(|probe| Self::evaluate(probe, resource, metrics))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn probe_passes_when_expression_holds() {
        let probe = ControlProbe::new("SOC2-CC6.1", "spec.tls.enabled == true");
        let resource = json!({ "spec": { "tls": { "enabled": true } } });
        let result = ProbeEvaluator::evaluate(&probe, &resource, None);
        assert!(result.passed);
        assert_eq!(result.probe_id, "SOC2-CC6.1");
    }

    #[test]
    fn probe_fails_when_expression_does_not_hold() {
        let probe = ControlProbe::new("SOC2-CC6.1", "spec.tls.enabled == true");
        let resource = json!({ "spec": { "tls": { "enabled": false } } });
        let result = ProbeEvaluator::evaluate(&probe, &resource, None);
        assert!(!result.passed);
        assert!(result.message.contains("failed"));
    }

    #[test]
    fn probe_with_metrics_path_requires_metrics_value() {
        let probe =
            ControlProbe::new("PCI-10.2", "value == true").with_metrics_path("audit.enabled");
        let resource = json!({ "value": true });
        let metrics = json!({ "audit": { "enabled": true } });
        assert!(ProbeEvaluator::evaluate(&probe, &resource, Some(&metrics)).passed);

        let failing_metrics = json!({ "audit": { "enabled": false } });
        assert!(!ProbeEvaluator::evaluate(&probe, &resource, Some(&failing_metrics)).passed);

        // Missing metrics document must fail closed.
        assert!(!ProbeEvaluator::evaluate(&probe, &resource, None).passed);
    }

    #[test]
    fn evaluate_all_returns_one_result_per_probe() {
        let probes = vec![
            ControlProbe::new("A", "spec.enabled == true"),
            ControlProbe::new("B", "spec.missing != null"),
        ];
        let resource = json!({ "spec": { "enabled": true } });
        let results = ProbeEvaluator::evaluate_all(&probes, &resource, None);
        assert_eq!(results.len(), 2);
        assert!(results[0].passed);
        assert!(!results[1].passed);
    }
}
