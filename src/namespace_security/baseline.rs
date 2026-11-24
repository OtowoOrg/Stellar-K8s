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
//! Security baseline definitions

use std::collections::BTreeMap;
use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::policy_drift::PolicyDriftCase;

/// Security baseline custom resource
#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "security.stellar.org",
    version = "v1alpha1",
    kind = "SecurityBaseline",
    plural = "securitybaselines",
    shortname = "sb",
    namespaced,
    status = "BaselineStatus",
    derive = "PartialEq",
    printcolumn = r#"{"name":"Profile","type":"string","jsonPath":".spec.profile"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Passed","type":"integer","jsonPath":".status.passedChecks"}"#,
    printcolumn = r#"{"name":"Failed","type":"integer","jsonPath":".status.failedChecks"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
pub struct BaselineSpec {
    /// Baseline profile name
    pub profile: BaselineProfile,
    /// Namespaces to evaluate (empty = all managed namespaces)
    #[serde(default)]
    pub target_namespaces: Vec<String>,
    /// Namespace label selector
    #[serde(default)]
    pub namespace_selector: BTreeMap<String, String>,
    /// Checks to run (empty = all for profile)
    #[serde(default)]
    pub checks: Vec<String>,
    /// Checks to skip
    #[serde(default)]
    pub skipped_checks: Vec<String>,
    /// Auto-remediation configuration
    #[serde(default)]
    pub auto_remediation: AutoRemediationConfig,
    /// Evaluation schedule (cron expression)
    #[serde(default = "default_schedule")]
    pub schedule: String,
    /// Remediation schedule (cron expression)
    #[serde(default = "default_remediation_schedule")]
    pub remediation_schedule: String,
}

fn default_schedule() -> String { "0 */6 * * *".to_string() } // Every 6 hours
fn default_remediation_schedule() -> String { "0 */12 * * *".to_string() } // Every 12 hours

/// Baseline profile
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum BaselineProfile {
    /// Minimal baseline - basic PSS restricted
    #[default]
    Minimal,
    /// Standard baseline - PSS restricted + network policies + resource limits
    Standard,
    /// Strict baseline - Standard + admission controls + encryption
    Strict,
    /// Custom profile with user-defined checks
    Custom(String),
}

/// Auto-remediation configuration
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoRemediationConfig {
    /// Enable auto-remediation
    #[serde(default)]
    pub enabled: bool,
    /// Only remediate safe checks (no risk of breaking workloads)
    #[serde(default = "default_true")]
    pub safe_only: bool,
    /// Maximum remediations per evaluation cycle
    #[serde(default = "default_max_remediations")]
    pub max_remediations_per_cycle: u32,
    /// Require approval for high-severity remediations
    #[serde(default = "default_true")]
    pub require_approval_high_severity: bool,
    /// Dry run mode
    #[serde(default)]
    pub dry_run: bool,
    /// Notification webhook
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_webhook: Option<String>,
}

fn default_true() -> bool { true }
fn default_max_remediations() -> u32 { 10 }

/// Baseline status
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BaselineStatus {
    /// Current evaluation phase
    pub phase: BaselinePhase,
    /// Last evaluation timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluation: Option<DateTime<Utc>>,
    /// Next scheduled evaluation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_evaluation: Option<DateTime<Utc>>,
    /// Last remediation timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_remediation: Option<DateTime<Utc>>,
    /// Number of passed checks
    pub passed_checks: u32,
    /// Number of failed checks
    pub failed_checks: u32,
    /// Number of skipped checks
    pub skipped_checks: u32,
    /// Number of auto-remediated checks
    pub remediated_checks: u32,
    /// Per-namespace results
    #[serde(default)]
    pub namespace_results: BTreeMap<String, NamespaceBaselineResult>,
    /// Remediation cases tracked from detection through verification
    #[serde(default)]
    pub drift_cases: BTreeMap<String, PolicyDriftCase>,
    /// Conditions
    #[serde(default)]
    pub conditions: Vec<BaselineCondition>,
}

/// Baseline evaluation phase
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum BaselinePhase {
    #[default]
    Pending,
    Evaluating,
    Completed,
    Remediating,
    Failed,
}

/// Per-namespace baseline result
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceBaselineResult {
    pub namespace: String,
    pub phase: BaselinePhase,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub remediated: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluation: Option<DateTime<Utc>>,
    #[serde(default)]
    pub check_results: Vec<CheckResult>,
}

/// Baseline check definition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BaselineCheck {
    /// Check identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description
    pub description: String,
    /// Severity if failed
    pub severity: CheckSeverity,
    /// Whether this check can be auto-remediated safely
    #[serde(default)]
    pub auto_remediable: bool,
    /// Remediation action (if auto_remediable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<RemediationAction>,
    /// Profiles this check belongs to
    pub profiles: Vec<BaselineProfile>,
    /// Kubernetes resource types this check applies to
    #[serde(default)]
    pub resource_types: Vec<String>,
}

/// Check severity
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum CheckSeverity {
    Info,
    #[default]
    Warning,
    High,
    Critical,
}

/// Check result
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub check_id: String,
    pub check_name: String,
    pub status: CheckStatus,
    pub severity: CheckSeverity,
    pub message: String,
    pub details: Option<String>,
    pub remediated: bool,
    pub remediation_action: Option<RemediationAction>,
    pub evaluated_at: DateTime<Utc>,
}

/// Check status
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum CheckStatus {
    #[default]
    Pass,
    Fail,
    Skip,
    Error,
}

/// Remediation action
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemediationAction {
    pub action_type: RemediationActionType,
    pub target_resource: TargetResource,
    pub parameters: BTreeMap<String, serde_json::Value>,
    pub description: String,
}

/// Remediation action type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RemediationActionType {
    AddLabel,
    RemoveLabel,
    AddAnnotation,
    PatchResource,
    CreateResource,
    DeleteResource,
    ApplyManifest,
}

/// Target resource for remediation
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetResource {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Baseline condition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BaselineCondition {
    pub type_: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<DateTime<Utc>>,
}

/// Built-in baseline checks
pub fn builtin_checks() -> Vec<BaselineCheck> {
    vec![
        // PSS Checks
        BaselineCheck {
            id: "pss-restricted-enforce".to_string(),
            name: "PSS Restricted Enforce Label".to_string(),
            description: "Namespace has pod-security.kubernetes.io/enforce=restricted label".to_string(),
            severity: CheckSeverity::High,
            auto_remediable: true,
            remediation: Some(RemediationAction {
                action_type: RemediationActionType::AddLabel,
                target_resource: TargetResource {
                    api_version: "v1".to_string(),
                    kind: "Namespace".to_string(),
                    name: "".to_string(), // Filled at runtime
                    namespace: None,
                },
                parameters: {
                    let mut params = BTreeMap::new();
                    params.insert("pod-security.kubernetes.io/enforce".to_string(), serde_json::json!("restricted"));
                    params.insert("pod-security.kubernetes.io/enforce-version".to_string(), serde_json::json!("latest"));
                    params.insert("pod-security.kubernetes.io/warn".to_string(), serde_json::json!("restricted"));
                    params.insert("pod-security.kubernetes.io/warn-version".to_string(), serde_json::json!("latest"));
                    params.insert("pod-security.kubernetes.io/audit".to_string(), serde_json::json!("restricted"));
                    params.insert("pod-security.kubernetes.io/audit-version".to_string(), serde_json::json!("latest"));
                    params
                },
                description: "Add PSS restricted labels to namespace".to_string(),
            }),
            profiles: vec![BaselineProfile::Minimal, BaselineProfile::Standard, BaselineProfile::Strict],
            resource_types: vec!["Namespace".to_string()],
        },
        // Network Policy Checks
        BaselineCheck {
            id: "default-deny-ingress".to_string(),
            name: "Default Deny Ingress NetworkPolicy".to_string(),
            description: "Namespace has a default-deny ingress NetworkPolicy".to_string(),
            severity: CheckSeverity::High,
            auto_remediable: true,
            remediation: Some(RemediationAction {
                action_type: RemediationActionType::CreateResource,
                target_resource: TargetResource {
                    api_version: "networking.k8s.io/v1".to_string(),
                    kind: "NetworkPolicy".to_string(),
                    name: "default-deny-ingress".to_string(),
                    namespace: None,
                },
                parameters: {
                    let mut params = BTreeMap::new();
                    params.insert("manifest".to_string(), serde_json::json!({
                        "apiVersion": "networking.k8s.io/v1",
                        "kind": "NetworkPolicy",
                        "metadata": {
                            "name": "default-deny-ingress",
                            "labels": {
                                "security.stellar.org/baseline": "true"
                            }
                        },
                        "spec": {
                            "podSelector": {},
                            "policyTypes": ["Ingress"],
                            "ingress": []
                        }
                    }));
                    params
                },
                description: "Create default-deny ingress NetworkPolicy".to_string(),
            }),
            profiles: vec![BaselineProfile::Standard, BaselineProfile::Strict],
            resource_types: vec!["NetworkPolicy".to_string()],
        },
        BaselineCheck {
            id: "default-deny-egress".to_string(),
            name: "Default Deny Egress NetworkPolicy".to_string(),
            description: "Namespace has a default-deny egress NetworkPolicy".to_string(),
            severity: CheckSeverity::Medium,
            auto_remediable: true,
            remediation: Some(RemediationAction {
                action_type: RemediationActionType::CreateResource,
                target_resource: TargetResource {
                    api_version: "networking.k8s.io/v1".to_string(),
                    kind: "NetworkPolicy".to_string(),
                    name: "default-deny-egress".to_string(),
                    namespace: None,
                },
                parameters: {
                    let mut params = BTreeMap::new();
                    params.insert("manifest".to_string(), serde_json::json!({
                        "apiVersion": "networking.k8s.io/v1",
                        "kind": "NetworkPolicy",
                        "metadata": {
                            "name": "default-deny-egress",
                            "labels": {
                                "security.stellar.org/baseline": "true"
                            }
                        },
                        "spec": {
                            "podSelector": {},
                            "policyTypes": ["Egress"],
                            "egress": []
                        }
                    }));
                    params
                },
                description: "Create default-deny egress NetworkPolicy".to_string(),
            }),
            profiles: vec![BaselineProfile::Strict],
            resource_types: vec!["NetworkPolicy".to_string()],
        },
        // Resource Limits Checks
        BaselineCheck {
            id: "limit-range".to_string(),
            name: "LimitRange for Resource Quotas".to_string(),
            description: "Namespace has a LimitRange defining default resource limits".to_string(),
            severity: CheckSeverity::Medium,
            auto_remediable: true,
            remediation: Some(RemediationAction {
                action_type: RemediationActionType::CreateResource,
                target_resource: TargetResource {
                    api_version: "v1".to_string(),
                    kind: "LimitRange".to_string(),
                    name: "default-limits".to_string(),
                    namespace: None,
                },
                parameters: {
                    let mut params = BTreeMap::new();
                    params.insert("manifest".to_string(), serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "LimitRange",
                        "metadata": {
                            "name": "default-limits",
                            "labels": {
                                "security.stellar.org/baseline": "true"
                            }
                        },
                        "spec": {
                            "limits": [
                                {
                                    "type": "Container",
                                    "default": {
                                        "cpu": "500m",
                                        "memory": "1Gi"
                                    },
                                    "defaultRequest": {
                                        "cpu": "100m",
                                        "memory": "256Mi"
                                    },
                                    "max": {
                                        "cpu": "4",
                                        "memory": "8Gi"
                                    },
                                    "min": {
                                        "cpu": "10m",
                                        "memory": "10Mi"
                                    }
                                }
                            ]
                        }
                    }));
                    params
                },
                description: "Create default LimitRange for containers".to_string(),
            }),
            profiles: vec![BaselineProfile::Standard, BaselineProfile::Strict],
            resource_types: vec!["LimitRange".to_string()],
        },
        // Resource Quota Checks
        BaselineCheck {
            id: "resource-quota".to_string(),
            name: "ResourceQuota for Namespace".to_string(),
            description: "Namespace has a ResourceQuota limiting resource consumption".to_string(),
            severity: CheckSeverity::Medium,
            auto_remediable: true,
            remediation: Some(RemediationAction {
                action_type: RemediationActionType::CreateResource,
                target_resource: TargetResource {
                    api_version: "v1".to_string(),
                    kind: "ResourceQuota".to_string(),
                    name: "default-quota".to_string(),
                    namespace: None,
                },
                parameters: {
                    let mut params = BTreeMap::new();
                    params.insert("manifest".to_string(), serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "ResourceQuota",
                        "metadata": {
                            "name": "default-quota",
                            "labels": {
                                "security.stellar.org/baseline": "true"
                            }
                        },
                        "spec": {
                            "hard": {
                                "cpu": "16",
                                "memory": "32Gi",
                                "pods": "50",
                                "services": "20",
                                "secrets": "50",
                                "persistentvolumeclaims": "10"
                            }
                        }
                    }));
                    params
                },
                description: "Create default ResourceQuota".to_string(),
            }),
            profiles: vec![BaselineProfile::Standard, BaselineProfile::Strict],
            resource_types: vec!["ResourceQuota".to_string()],
        },
        // RBAC Checks
        BaselineCheck {
            id: "no-wildcard-rbac".to_string(),
            name: "No Wildcard RBAC Permissions".to_string(),
            description: "No Role/ClusterRole with wildcard (*) permissions in namespace".to_string(),
            severity: CheckSeverity::High,
            auto_remediable: false,
            remediation: None,
            profiles: vec![BaselineProfile::Standard, BaselineProfile::Strict],
            resource_types: vec!["Role".to_string(), "RoleBinding".to_string()],
        },
        // Service Account Checks
        BaselineCheck {
            id: "no-default-sa-automount".to_string(),
            name: "Default Service Account Automount Disabled".to_string(),
            description: "Default service account has automountServiceAccountToken=false".to_string(),
            severity: CheckSeverity::Medium,
            auto_remediable: true,
            remediation: Some(RemediationAction {
                action_type: RemediationActionType::PatchResource,
                target_resource: TargetResource {
                    api_version: "v1".to_string(),
                    kind: "ServiceAccount".to_string(),
                    name: "default".to_string(),
                    namespace: None,
                },
                parameters: {
                    let mut params = BTreeMap::new();
                    params.insert("automountServiceAccountToken".to_string(), serde_json::json!(false));
                    params
                },
                description: "Disable automount for default service account".to_string(),
            }),
            profiles: vec![BaselineProfile::Standard, BaselineProfile::Strict],
            resource_types: vec!["ServiceAccount".to_string()],
        },
        // Encryption Checks
        BaselineCheck {
            id: "secrets-encryption".to_string(),
            name: "Secrets Encryption at Rest".to_string(),
            description: "Secrets are encrypted at rest (requires cluster-level configuration)".to_string(),
            severity: CheckSeverity::Critical,
            auto_remediable: false,
            remediation: None,
            profiles: vec![BaselineProfile::Strict],
            resource_types: vec!["Secret".to_string()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_baseline_profile_serialization() {
        let profile = BaselineProfile::Standard;
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(json, "\"Standard\"");
        
        let custom = BaselineProfile::Custom("my-profile".to_string());
        let json = serde_json::to_string(&custom).unwrap();
        assert!(json.contains("my-profile"));
    }

    #[test]
 fn test_builtin_checks() {
        let checks = builtin_checks();
        assert!(!checks.is_empty());
        
        // Verify all checks have required fields
        for check in &checks {
            assert!(!check.id.is_empty());
            assert!(!check.name.is_empty());
            assert!(!check.profiles.is_empty());
        }
        
        // Check specific profiles
        let minimal_checks: Vec<_> = checks.iter().filter(|c| c.profiles.contains(&BaselineProfile::Minimal)).collect();
        assert!(!minimal_checks.is_empty());
        
        let strict_checks: Vec<_> = checks.iter().filter(|c| c.profiles.contains(&BaselineProfile::Strict)).collect();
        assert!(strict_checks.len() > minimal_checks.len());
    }

    #[test]
    fn test_check_severity_serialization() {
        let severity = CheckSeverity::High;
        let json = serde_json::to_string(&severity).unwrap();
        assert_eq!(json, "\"High\"");
    }
}
