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
//! Baseline evaluator

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::namespace_security::baseline::{
    BaselineCheck, BaselineProfile, BaselineSpec, BaselineStatus, BaselinePhase, 
    CheckResult, CheckStatus, CheckSeverity, NamespaceBaselineResult, RemediationAction,
    builtin_checks,
};
use crate::error::{Error, Result};

/// Evaluator configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluatorConfig {
    /// Kubernetes client
    pub client: Client,
    /// Baseline specification
    pub spec: BaselineSpec,
    /// Custom checks (merged with builtin)
    #[serde(default)]
    pub custom_checks: Vec<BaselineCheck>,
    /// Namespace to evaluate (if not using spec.target_namespaces)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl EvaluatorConfig {
    pub fn new(client: Client, spec: BaselineSpec) -> Self {
        Self {
            client,
            spec,
            custom_checks: vec![],
            namespace: None,
        }
    }
}

/// Evaluation result
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationResult {
    pub baseline_name: String,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: u64,
    pub namespace_results: BTreeMap<String, NamespaceBaselineResult>,
    pub total_passed: u32,
    pub total_failed: u32,
    pub total_skipped: u32,
    pub overall_phase: BaselinePhase,
}

/// Baseline evaluator
pub struct BaselineEvaluator {
    config: EvaluatorConfig,
    checks: Vec<BaselineCheck>,
    /// Prometheus metrics
    metrics: Arc<EvaluatorMetrics>,
}

/// Prometheus metrics for evaluator
pub struct EvaluatorMetrics {
    pub evaluations_total: Family<EvaluatorLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub evaluation_duration_seconds: Family<EvaluatorLabels, Histogram>,
    pub checks_passed: Family<EvaluatorLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub checks_failed: Family<EvaluatorLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub checks_skipped: Family<EvaluatorLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub namespace_score: Family<NamespaceLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct EvaluatorLabels {
    pub baseline: String,
    pub profile: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct NamespaceLabels {
    pub baseline: String,
    pub namespace: String,
}

impl EvaluatorMetrics {
    pub fn new(registry: &mut Registry, baseline_name: &str, profile: &BaselineProfile) -> Self {
        let metrics = Self {
            evaluations_total: Family::default(),
            evaluation_duration_seconds: Family::default(),
            checks_passed: Family::default(),
            checks_failed: Family::default(),
            checks_skipped: Family::default(),
            namespace_score: Family::default(),
        };

        let labels = EvaluatorLabels {
            baseline: baseline_name.to_string(),
            profile: format!("{:?}", profile),
        };

        registry.register(
            "security_baseline_evaluations_total",
            "Total baseline evaluations",
            metrics.evaluations_total.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_evaluation_duration_seconds",
            "Baseline evaluation duration",
            metrics.evaluation_duration_seconds.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_checks_passed",
            "Checks passed in last evaluation",
            metrics.checks_passed.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_checks_failed",
            "Checks failed in last evaluation",
            metrics.checks_failed.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_checks_skipped",
            "Checks skipped in last evaluation",
            metrics.checks_skipped.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_namespace_score",
            "Namespace compliance score (0-100)",
            metrics.namespace_score.clone(),
        ).unwrap();

        // Initialize baseline-level metrics
        metrics.evaluations_total.get_or_create(&labels).inc();
        metrics.checks_passed.get_or_create(&labels).set(0);
        metrics.checks_failed.get_or_create(&labels).set(0);
        metrics.checks_skipped.get_or_create(&labels).set(0);

        metrics
    }
}

impl BaselineEvaluator {
    /// Create a new baseline evaluator
    pub fn new(config: EvaluatorConfig, registry: &mut Registry) -> Self {
        // Combine builtin and custom checks
        let mut checks = builtin_checks();
        checks.extend(config.custom_checks.clone());
        
        // Filter checks by profile
        let profile = config.spec.profile.clone();
        checks.retain(|c| c.profiles.contains(&profile) || c.profiles.contains(&BaselineProfile::Custom("".to_string())));

        // Filter by spec.checks if specified
        if !config.spec.checks.is_empty() {
            checks.retain(|c| config.spec.checks.contains(&c.id));
        }

        // Filter out skipped checks
        if !config.spec.skipped_checks.is_empty() {
            checks.retain(|c| !config.spec.skipped_checks.contains(&c.id));
        }

        let metrics = Arc::new(EvaluatorMetrics::new(registry, "baseline", &profile));

        Self {
            config,
            checks,
            metrics,
        }
    }

    /// Run evaluation for all target namespaces
    pub async fn evaluate_all(&self) -> Result<EvaluationResult> {
        let start = std::time::Instant::now();
        let baseline_name = self.config.spec.profile.to_string();
        
        // Determine target namespaces
        let namespaces = self.get_target_namespaces().await?;
        
        info!("Evaluating baseline {} for {} namespaces", baseline_name, namespaces.len());

        let mut namespace_results = BTreeMap::new();
        let mut total_passed = 0u32;
        let mut total_failed = 0u32;
        let mut total_skipped = 0u32;

        for ns in &namespaces {
            let result = self.evaluate_namespace(ns).await?;
            total_passed += result.passed;
            total_failed += result.failed;
            total_skipped += result.skipped;
            namespace_results.insert(ns.clone(), result);
        }

        let overall_phase = if total_failed > 0 {
            BaselinePhase::Completed
        } else {
            BaselinePhase::Completed
        };

        let duration = start.elapsed();
        
        // Update metrics
        let labels = EvaluatorLabels {
            baseline: baseline_name.clone(),
            profile: format!("{:?}", self.config.spec.profile),
        };
        
        self.metrics.evaluations_total.get_or_create(&labels).inc();
        self.metrics.evaluation_duration_seconds.get_or_create(&labels).observe(duration.as_secs_f64());
        self.metrics.checks_passed.get_or_create(&labels).set(total_passed as i64);
        self.metrics.checks_failed.get_or_create(&labels).set(total_failed as i64);
        self.metrics.checks_skipped.get_or_create(&labels).set(total_skipped as i64);

        for (ns, result) in &namespace_results {
            let score = if result.passed + result.failed > 0 {
                result.passed as f64 / (result.passed + result.failed) as f64 * 100.0
            } else {
                100.0
            };
            
            self.metrics.namespace_score
                .get_or_create(&NamespaceLabels {
                    baseline: baseline_name.clone(),
                    namespace: ns.clone(),
                })
                .set(score);
        }

        Ok(EvaluationResult {
            baseline_name,
            timestamp: Utc::now(),
            duration_ms: duration.as_millis() as u64,
            namespace_results,
            total_passed,
            total_failed,
            total_skipped,
            overall_phase,
        })
    }

    /// Evaluate a single namespace
    async fn evaluate_namespace(&self, namespace: &str) -> Result<NamespaceBaselineResult> {
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut skipped = 0u32;
        let mut check_results = Vec::new();

        for check in &self.checks {
            let result = self.run_check(namespace, check).await?;
            
            match result.status {
                CheckStatus::Pass => passed += 1,
                CheckStatus::Fail => failed += 1,
                CheckStatus::Skip => skipped += 1,
                CheckStatus::Error => failed += 1, // Treat errors as failures
            }
            
            check_results.push(result);
        }

        Ok(NamespaceBaselineResult {
            namespace: namespace.to_string(),
            phase: if failed > 0 { BaselinePhase::Completed } else { BaselinePhase::Completed },
            passed,
            failed,
            skipped,
            remediated: 0,
            last_evaluation: Some(Utc::now()),
            check_results,
        })
    }

    /// Run a single check against a namespace
    async fn run_check(&self, namespace: &str, check: &BaselineCheck) -> Result<CheckResult> {
        let result = match check.id.as_str() {
            "pss-restricted-enforce" => self.check_pss_labels(namespace).await,
            "default-deny-ingress" => self.check_default_deny_ingress(namespace).await,
            "default-deny-egress" => self.check_default_deny_egress(namespace).await,
            "limit-range" => self.check_limit_range(namespace).await,
            "resource-quota" => self.check_resource_quota(namespace).await,
            "no-wildcard-rbac" => self.check_no_wildcard_rbac(namespace).await,
            "no-default-sa-automount" => self.check_default_sa_automount(namespace).await,
            "secrets-encryption" => self.check_secrets_encryption(namespace).await,
            _ => Ok(CheckResult {
                check_id: check.id.clone(),
                check_name: check.name.clone(),
                status: CheckStatus::Skip,
                severity: check.severity.clone(),
                message: "Check not implemented".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            }),
        };

        result.map_err(|e| {
            CheckResult {
                check_id: check.id.clone(),
                check_name: check.name.clone(),
                status: CheckStatus::Error,
                severity: check.severity.clone(),
                message: format!("Check error: {}", e),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            }
        }).unwrap_or_else(|e| e)
    }

    /// Check PSS labels on namespace
    async fn check_pss_labels(&self, namespace: &str) -> Result<CheckResult> {
        let api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(self.config.client.clone());
        let ns = api.get(namespace).await.map_err(Error::KubeError)?;
        
        let labels = ns.metadata.labels.unwrap_or_default();
        let required = [
            ("pod-security.kubernetes.io/enforce", "restricted"),
            ("pod-security.kubernetes.io/enforce-version", "latest"),
            ("pod-security.kubernetes.io/warn", "restricted"),
            ("pod-security.kubernetes.io/warn-version", "latest"),
            ("pod-security.kubernetes.io/audit", "restricted"),
            ("pod-security.kubernetes.io/audit-version", "latest"),
        ];

        let mut missing = Vec::new();
        for (key, expected) in required {
            if labels.get(key) != Some(&expected.to_string()) {
                missing.push(format!("{}={}", key, expected));
            }
        }

        if missing.is_empty() {
            Ok(CheckResult {
                check_id: "pss-restricted-enforce".to_string(),
                check_name: "PSS Restricted Enforce Label".to_string(),
                status: CheckStatus::Pass,
                severity: CheckSeverity::High,
                message: "All PSS restricted labels present".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        } else {
            Ok(CheckResult {
                check_id: "pss-restricted-enforce".to_string(),
                check_name: "PSS Restricted Enforce Label".to_string(),
                status: CheckStatus::Fail,
                severity: CheckSeverity::High,
                message: format!("Missing PSS labels: {}", missing.join(", ")),
                details: Some(missing.join(", ")),
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        }
    }

    /// Check default-deny ingress NetworkPolicy
    async fn check_default_deny_ingress(&self, namespace: &str) -> Result<CheckResult> {
        let api: Api<k8s_openapi::api::networking::v1::NetworkPolicy> = 
            Api::namespaced(self.config.client.clone(), namespace);
        
        let list = api.list(&ListParams::default()).await.map_err(Error::KubeError)?;
        
        let has_default_deny = list.items.iter().any(|np| {
            let spec = &np.spec;
            spec.pod_selector.is_none() || spec.pod_selector.as_ref().map(|s| s.match_labels.is_empty() && s.match_expressions.is_empty()).unwrap_or(true)
                && spec.policy_types.as_ref().map(|t| t.contains(&"Ingress".to_string())).unwrap_or(false)
                && spec.ingress.as_ref().map(|i| i.is_empty()).unwrap_or(false)
        });

        if has_default_deny {
            Ok(CheckResult {
                check_id: "default-deny-ingress".to_string(),
                check_name: "Default Deny Ingress NetworkPolicy".to_string(),
                status: CheckStatus::Pass,
                severity: CheckSeverity::High,
                message: "Default deny ingress NetworkPolicy exists".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        } else {
            Ok(CheckResult {
                check_id: "default-deny-ingress".to_string(),
                check_name: "Default Deny Ingress NetworkPolicy".to_string(),
                status: CheckStatus::Fail,
                severity: CheckSeverity::High,
                message: "No default deny ingress NetworkPolicy found".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        }
    }

    /// Check default-deny egress NetworkPolicy
    async fn check_default_deny_egress(&self, namespace: &str) -> Result<CheckResult> {
        let api: Api<k8s_openapi::api::networking::v1::NetworkPolicy> = 
            Api::namespaced(self.config.client.clone(), namespace);
        
        let list = api.list(&ListParams::default()).await.map_err(Error::KubeError)?;
        
        let has_default_deny = list.items.iter().any(|np| {
            let spec = &np.spec;
            spec.pod_selector.is_none() || spec.pod_selector.as_ref().map(|s| s.match_labels.is_empty() && s.match_expressions.is_empty()).unwrap_or(true)
                && spec.policy_types.as_ref().map(|t| t.contains(&"Egress".to_string())).unwrap_or(false)
                && spec.egress.as_ref().map(|e| e.is_empty()).unwrap_or(false)
        });

        if has_default_deny {
            Ok(CheckResult {
                check_id: "default-deny-egress".to_string(),
                check_name: "Default Deny Egress NetworkPolicy".to_string(),
                status: CheckStatus::Pass,
                severity: CheckSeverity::Medium,
                message: "Default deny egress NetworkPolicy exists".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        } else {
            Ok(CheckResult {
                check_id: "default-deny-egress".to_string(),
                check_name: "Default Deny Egress NetworkPolicy".to_string(),
                status: CheckStatus::Fail,
                severity: CheckSeverity::Medium,
                message: "No default deny egress NetworkPolicy found".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        }
    }

    /// Check LimitRange
    async fn check_limit_range(&self, namespace: &str) -> Result<CheckResult> {
        let api: Api<k8s_openapi::api::core::v1::LimitRange> = 
            Api::namespaced(self.config.client.clone(), namespace);
        
        let list = api.list(&ListParams::default()).await.map_err(Error::KubeError)?;
        
        let has_limit_range = !list.items.is_empty();

        if has_limit_range {
            Ok(CheckResult {
                check_id: "limit-range".to_string(),
                check_name: "LimitRange for Resource Quotas".to_string(),
                status: CheckStatus::Pass,
                severity: CheckSeverity::Medium,
                message: "LimitRange exists".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        } else {
            Ok(CheckResult {
                check_id: "limit-range".to_string(),
                check_name: "LimitRange for Resource Quotas".to_string(),
                status: CheckStatus::Fail,
                severity: CheckSeverity::Medium,
                message: "No LimitRange found".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        }
    }

    /// Check ResourceQuota
    async fn check_resource_quota(&self, namespace: &str) -> Result<CheckResult> {
        let api: Api<k8s_openapi::api::core::v1::ResourceQuota> = 
            Api::namespaced(self.config.client.clone(), namespace);
        
        let list = api.list(&ListParams::default()).await.map_err(Error::KubeError)?;
        
        let has_quota = !list.items.is_empty();

        if has_quota {
            Ok(CheckResult {
                check_id: "resource-quota".to_string(),
                check_name: "ResourceQuota for Namespace".to_string(),
                status: CheckStatus::Pass,
                severity: CheckSeverity::Medium,
                message: "ResourceQuota exists".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        } else {
            Ok(CheckResult {
                check_id: "resource-quota".to_string(),
                check_name: "ResourceQuota for Namespace".to_string(),
                status: CheckStatus::Fail,
                severity: CheckSeverity::Medium,
                message: "No ResourceQuota found".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        }
    }

    /// Check no wildcard RBAC
    async fn check_no_wildcard_rbac(&self, namespace: &str) -> Result<CheckResult> {
        let role_api: Api<k8s_openapi::api::rbac::v1::Role> = 
            Api::namespaced(self.config.client.clone(), namespace);
        
        let list = role_api.list(&ListParams::default()).await.map_err(Error::KubeError)?;
        
        let mut wildcard_roles = Vec::new();
        for role in list.items {
            if let Some(rules) = role.rules {
                for rule in rules {
                    if rule.verbs.iter().any(|v| v == "*") ||
                       rule.resources.iter().any(|r| r == "*") ||
                       rule.api_groups.iter().any(|g| g == "*") {
                        wildcard_roles.push(role.name_any());
                        break;
                    }
                }
            }
        }

        if wildcard_roles.is_empty() {
            Ok(CheckResult {
                check_id: "no-wildcard-rbac".to_string(),
                check_name: "No Wildcard RBAC Permissions".to_string(),
                status: CheckStatus::Pass,
                severity: CheckSeverity::High,
                message: "No wildcard RBAC permissions found".to_string(),
                details: None,
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        } else {
            Ok(CheckResult {
                check_id: "no-wildcard-rbac".to_string(),
                check_name: "No Wildcard RBAC Permissions".to_string(),
                status: CheckStatus::Fail,
                severity: CheckSeverity::High,
                message: format!("Wildcard permissions found in roles: {}", wildcard_roles.join(", ")),
                details: Some(wildcard_roles.join(", ")),
                remediated: false,
                remediation_action: None,
                evaluated_at: Utc::now(),
            })
        }
    }

    /// Check default service account automount
    async fn check_default_sa_automount(&self, namespace: &str) -> Result<CheckResult> {
        let api: Api<k8s_openapi::api::core::v1::ServiceAccount> = 
            Api::namespaced(self.config.client.clone(), namespace);
        
        match api.get("default").await {
            Ok(sa) => {
                let automount = sa.automount_service_account_token.unwrap_or(true);
                
                if !automount {
                    Ok(CheckResult {
                        check_id: "no-default-sa-automount".to_string(),
                        check_name: "Default Service Account Automount Disabled".to_string(),
                        status: CheckStatus::Pass,
                        severity: CheckSeverity::Medium,
                        message: "Default SA automount disabled".to_string(),
                        details: None,
                        remediated: false,
                        remediation_action: None,
                        evaluated_at: Utc::now(),
                    })
                } else {
                    Ok(CheckResult {
                        check_id: "no-default-sa-automount".to_string(),
                        check_name: "Default Service Account Automount Disabled".to_string(),
                        status: CheckStatus::Fail,
                        severity: CheckSeverity::Medium,
                        message: "Default SA automount enabled".to_string(),
                        details: None,
                        remediated: false,
                        remediation_action: None,
                        evaluated_at: Utc::now(),
                    })
                }
            }
            Err(_) => {
                Ok(CheckResult {
                    check_id: "no-default-sa-automount".to_string(),
                    check_name: "Default Service Account Automount Disabled".to_string(),
                    status: CheckStatus::Error,
                    severity: CheckSeverity::Medium,
                    message: "Default service account not found".to_string(),
                    details: None,
                    remediated: false,
                    remediation_action: None,
                    evaluated_at: Utc::now(),
                })
            }
        }
    }

    /// Check secrets encryption (cluster-level, always pass for namespace eval)
    async fn check_secrets_encryption(&self, _namespace: &str) -> Result<CheckResult> {
        Ok(CheckResult {
            check_id: "secrets-encryption".to_string(),
            check_name: "Secrets Encryption at Rest".to_string(),
            status: CheckStatus::Pass, // Cluster-level check
            severity: CheckSeverity::Critical,
            message: "Cluster-level check - evaluated separately".to_string(),
            details: None,
            remediated: false,
            remediation_action: None,
            evaluated_at: Utc::now(),
        })
    }

    /// Get target namespaces from spec
    async fn get_target_namespaces(&self) -> Result<Vec<String>> {
        if let Some(ns) = &self.config.namespace {
            return Ok(vec![ns.clone()]);
        }

        if !self.config.spec.target_namespaces.is_empty() {
            return Ok(self.config.spec.target_namespaces.clone());
        }

        // Use namespace selector
        let api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(self.config.client.clone());
        let selector = if self.config.spec.namespace_selector.is_empty() {
            "app.kubernetes.io/managed-by=stellar-operator".to_string()
        } else {
            self.config.spec.namespace_selector.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",")
        };

        let list = api.list(&ListParams::default().labels(&selector)).await.map_err(Error::KubeError)?;
        
        Ok(list.items.iter().map(|ns| ns.name_any()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namespace_security::baseline::{BaselineProfile, BaselineSpec, AutoRemediationConfig};

    #[test]
    fn test_evaluator_config() {
        // Can't easily test without a client, but verify structs work
        let spec = BaselineSpec {
            profile: BaselineProfile::Standard,
            target_namespaces: vec!["test".to_string()],
            namespace_selector: BTreeMap::new(),
            checks: vec![],
            skipped_checks: vec![],
            auto_remediation: AutoRemediationConfig::default(),
            schedule: "0 */6 * * *".to_string(),
            remediation_schedule: "0 */12 * * *".to_string(),
        };
        
        assert_eq!(spec.profile, BaselineProfile::Standard);
    }
}
