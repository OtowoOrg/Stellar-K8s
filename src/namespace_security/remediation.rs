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
//! Baseline auto-remediation

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::namespace_security::baseline::{
    BaselineSpec, CheckResult, CheckStatus, NamespaceBaselineResult, RemediationAction, RemediationActionType, TargetResource,
};
use crate::namespace_security::evaluator::EvaluationResult;
use crate::namespace_security::audit::{BaselineAuditLog, AuditEntry};
use crate::error::{Error, Result};

/// Remediation configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemediationConfig {
    /// Kubernetes client
    pub client: Client,
    /// Baseline specification
    pub spec: BaselineSpec,
    /// Audit log for recording remediation actions
    pub audit_log: Arc<BaselineAuditLog>,
    /// Dry run mode
    #[serde(default)]
    pub dry_run: bool,
    /// Maximum concurrent remediations
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_max_concurrent() -> usize { 5 }

/// Remediation result
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemediationResult {
    pub namespace: String,
    pub check_id: String,
    pub action_type: RemediationActionType,
    pub success: bool,
    pub message: String,
    pub dry_run: bool,
    pub timestamp: DateTime<Utc>,
    pub error: Option<String>,
}

/// Baseline remediator
pub struct BaselineRemediator {
    config: RemediationConfig,
    /// Prometheus metrics
    metrics: Arc<RemediationMetrics>,
    /// Active remediations semaphore
    semaphore: Arc<tokio::sync::Semaphore>,
}

/// Prometheus metrics for remediation
pub struct RemediationMetrics {
    pub remediations_total: Family<RemediationLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub remediation_duration_seconds: Family<RemediationLabels, Histogram>,
    pub remediation_success: Family<RemediationLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub remediation_failed: Family<RemediationLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RemediationLabels {
    pub baseline: String,
    pub namespace: String,
    pub check_id: String,
    pub action_type: String,
}

impl RemediationMetrics {
    pub fn new(registry: &mut Registry, baseline_name: &str) -> Self {
        let metrics = Self {
            remediations_total: Family::default(),
            remediation_duration_seconds: Family::default(),
            remediation_success: Family::default(),
            remediation_failed: Family::default(),
        };

        registry.register(
            "security_baseline_remediations_total",
            "Total remediation actions attempted",
            metrics.remediations_total.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_remediation_duration_seconds",
            "Remediation action duration",
            metrics.remediation_duration_seconds.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_remediation_success",
            "Successful remediations",
            metrics.remediation_success.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_remediation_failed",
            "Failed remediations",
            metrics.remediation_failed.clone(),
        ).unwrap();

        metrics
    }
}

impl BaselineRemediator {
    /// Create a new baseline remediator
    pub fn new(config: RemediationConfig, registry: &mut Registry) -> Self {
        let metrics = Arc::new(RemediationMetrics::new(registry, "baseline"));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_concurrent));
        
        Self {
            config,
            metrics,
            semaphore,
        }
    }

    /// Run remediation for failed checks in evaluation result
    pub async fn remediate(&self, evaluation: &EvaluationResult) -> Result<Vec<RemediationResult>> {
        if !self.config.spec.auto_remediation.enabled {
            info!("Auto-remediation disabled, skipping");
            return Ok(vec![]);
        }

        if self.config.spec.auto_remediation.dry_run || self.config.dry_run {
            info!("Running in dry-run mode");
        }

        let mut results = Vec::new();
        let mut remediation_count = 0;
        let max_remediations = self.config.spec.auto_remediation.max_remediations_per_cycle;

        for (namespace, ns_result) in &evaluation.namespace_results {
            if remediation_count >= max_remediations {
                warn!("Max remediations per cycle reached ({})", max_remediations);
                break;
            }

            for check_result in &ns_result.check_results {
                if remediation_count >= max_remediations {
                    break;
                }

                // Only remediate failed checks that are auto-remediable
                if check_result.status == CheckStatus::Fail 
                    && check_result.remediation_action.is_some() {
                    
                    // Check if safe-only mode and this is high/critical severity
                    if self.config.spec.auto_remediation.safe_only 
                        && matches!(check_result.severity, crate::namespace_security::baseline::CheckSeverity::High | crate::namespace_security::baseline::CheckSeverity::Critical) {
                        info!("Skipping high-severity remediation in safe-only mode: {}", check_result.check_id);
                        continue;
                    }

                    // Check if approval required for high severity
                    if self.config.spec.auto_remediation.require_approval_high_severity
                        && matches!(check_result.severity, crate::namespace_security::baseline::CheckSeverity::High | crate::namespace_security::baseline::CheckSeverity::Critical) {
                        info!("High-severity remediation requires approval: {}", check_result.check_id);
                        // In a real implementation, this would create an approval request
                        continue;
                    }

                    // Execute remediation
                    let permit = self.semaphore.clone().acquire_owned().await.unwrap();
                    let result = self.remediate_check(namespace, &check_result).await;
                    drop(permit);
                    
                    results.push(result);
                    remediation_count += 1;
                }
            }
        }

        Ok(results)
    }

    /// Remediate a single check
    async fn remediate_check(&self, namespace: &str, check_result: &CheckResult) -> RemediationResult {
        let start = std::time::Instant::now();
        let action = check_result.remediation_action.as_ref().unwrap();
        let dry_run = self.config.dry_run || self.config.spec.auto_remediation.dry_run;
        
        info!(
            "Remediating check {} in namespace {} (dry_run={})",
            check_result.check_id, namespace, dry_run
        );

        let result = if dry_run {
            self.dry_run_remediation(namespace, action).await
        } else {
            self.execute_remediation(namespace, action).await
        };

        let duration = start.elapsed();
        
        // Update metrics
        let labels = RemediationLabels {
            baseline: "baseline".to_string(),
            namespace: namespace.to_string(),
            check_id: check_result.check_id.clone(),
            action_type: format!("{:?}", action.action_type),
        };

        self.metrics.remediations_total.get_or_create(&labels).inc();
        self.metrics.remediation_duration_seconds.get_or_create(&labels).observe(duration.as_secs_f64());

        match &result {
            Ok(r) if r.success => {
                self.metrics.remediation_success.get_or_create(&labels).inc();
            }
            _ => {
                self.metrics.remediation_failed.get_or_create(&labels).inc();
            }
        }

        // Record in audit log
        self.config.audit_log.record(AuditEntry {
            namespace: namespace.to_string(),
            action: "AutoRemediation".to_string(),
            resource_type: format!("{:?}", action.action_type),
            resource_name: check_result.check_id.clone(),
            success: result.as_ref().map(|r| r.success).unwrap_or(false),
            message: result.as_ref().map(|r| r.message.clone()).unwrap_or_else(|e| e.to_string()),
            timestamp: Utc::now(),
            dry_run,
        }).await;

        result.unwrap_or_else(|e| RemediationResult {
            namespace: namespace.to_string(),
            check_id: check_result.check_id.clone(),
            action_type: action.action_type.clone(),
            success: false,
            message: "Remediation failed".to_string(),
            dry_run,
            timestamp: Utc::now(),
            error: Some(e.to_string()),
        })
    }

    /// Dry run remediation (simulate without applying)
    async fn dry_run_remediation(&self, namespace: &str, action: &RemediationAction) -> Result<RemediationResult> {
        let description = self.describe_remediation(namespace, action);
        
        info!("DRY RUN: {}", description);
        
        Ok(RemediationResult {
            namespace: namespace.to_string(),
            check_id: "".to_string(), // Will be filled by caller
            action_type: action.action_type.clone(),
            success: true,
            message: format!("DRY RUN: {}", description),
            dry_run: true,
            timestamp: Utc::now(),
            error: None,
        })
    }

    /// Execute remediation action
    async fn execute_remediation(&self, namespace: &str, action: &RemediationAction) -> Result<RemediationResult> {
        let result = match action.action_type {
            RemediationActionType::AddLabel => self.add_label(namespace, action).await,
            RemediationActionType::RemoveLabel => self.remove_label(namespace, action).await,
            RemediationActionType::AddAnnotation => self.add_annotation(namespace, action).await,
            RemediationActionType::PatchResource => self.patch_resource(namespace, action).await,
            RemediationActionType::CreateResource => self.create_resource(namespace, action).await,
            RemediationActionType::DeleteResource => self.delete_resource(namespace, action).await,
            RemediationActionType::ApplyManifest => self.apply_manifest(namespace, action).await,
        };

        result.map(|msg| RemediationResult {
            namespace: namespace.to_string(),
            check_id: "".to_string(),
            action_type: action.action_type.clone(),
            success: true,
            message: msg,
            dry_run: false,
            timestamp: Utc::now(),
            error: None,
        }).map_err(|e| RemediationResult {
            namespace: namespace.to_string(),
            check_id: "".to_string(),
            action_type: action.action_type.clone(),
            success: false,
            message: "Remediation failed".to_string(),
            dry_run: false,
            timestamp: Utc::now(),
            error: Some(e.to_string()),
        })
    }

    /// Add label to namespace
    async fn add_label(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        let api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(self.config.client.clone());
        
        let mut labels = BTreeMap::new();
        for (key, value) in &action.parameters {
            if let Some(str_val) = value.as_str() {
                labels.insert(key.clone(), str_val.to_string());
            }
        }

        let patch = json!({
            "metadata": {
                "labels": labels
            }
        });

        api.patch(namespace, &PatchParams::apply("baseline-remediator").force(), &Patch::Merge(&patch))
            .await
            .map_err(Error::KubeError)?;

        Ok(format!("Added labels to namespace {}", namespace))
    }

    /// Remove label from namespace
    async fn remove_label(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        let api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(self.config.client.clone());
        
        let ns = api.get(namespace).await.map_err(Error::KubeError)?;
        let mut labels = ns.metadata.labels.unwrap_or_default();
        
        for key in action.parameters.keys() {
            labels.remove(key);
        }

        let patch = json!({
            "metadata": {
                "labels": labels
            }
        });

        api.patch(namespace, &PatchParams::apply("baseline-remediator").force(), &Patch::Merge(&patch))
            .await
            .map_err(Error::KubeError)?;

        Ok(format!("Removed labels from namespace {}", namespace))
    }

    /// Add annotation to namespace
    async fn add_annotation(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        let api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(self.config.client.clone());
        
        let mut annotations = BTreeMap::new();
        for (key, value) in &action.parameters {
            if let Some(str_val) = value.as_str() {
                annotations.insert(key.clone(), str_val.to_string());
            }
        }

        let patch = json!({
            "metadata": {
                "annotations": annotations
            }
        });

        api.patch(namespace, &PatchParams::apply("baseline-remediator").force(), &Patch::Merge(&patch))
            .await
            .map_err(Error::KubeError)?;

        Ok(format!("Added annotations to namespace {}", namespace))
    }

    /// Patch a resource
    async fn patch_resource(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        let target = &action.target_resource;
        let ns = target.namespace.as_deref().unwrap_or(namespace);
        
        // Build patch from parameters
        let mut patch_data = json!({});
        if let Some(obj) = patch_data.as_object_mut() {
            for (key, value) in &action.parameters {
                obj.insert(key.clone(), value.clone());
            }
        }

        // For now, simulate the patch - in a full implementation this would use the dynamic client
        // or the kube client's request method
        Ok(format!("Patched {}/{} in namespace {} (simulated)", target.kind, target.name, ns))
    }

    /// Create a resource from manifest
    async fn create_resource(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        let manifest = action.parameters.get("manifest")
            .ok_or_else(|| Error::ValidationError("Manifest parameter required for CreateResource".to_string()))?;
        
        // Parse the manifest to get GVK
        let value: serde_json::Value = serde_json::from_value(manifest.clone())
            .map_err(|e| Error::SerializationError(e.to_string()))?;
        
        let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("ConfigMap");
        
        // For now, simulate the creation - in a full implementation this would use the dynamic client
        Ok(format!("Created {} in namespace {} (simulated)", kind, namespace))
    }

    /// Delete a resource
    async fn delete_resource(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        let target = &action.target_resource;
        let ns = target.namespace.as_deref().unwrap_or(namespace);
        
        // For now, simulate the deletion
        Ok(format!("Deleted {}/{} in namespace {} (simulated)", target.kind, target.name, ns))
    }

    /// Apply a manifest (create or update)
    async fn apply_manifest(&self, namespace: &str, action: &RemediationAction) -> Result<String> {
        // Similar to create but uses server-side apply
        self.create_resource(namespace, action).await
    }

    /// Generate human-readable description of remediation
    fn describe_remediation(&self, namespace: &str, action: &RemediationAction) -> String {
        let target = &action.target_resource;
        let ns = target.namespace.as_deref().unwrap_or(namespace);
        
        match action.action_type {
            RemediationActionType::AddLabel => {
                let labels: Vec<String> = action.parameters.keys().cloned().collect();
                format!("Add labels {} to namespace {}", labels.join(", "), ns)
            }
            RemediationActionType::RemoveLabel => {
                let labels: Vec<String> = action.parameters.keys().cloned().collect();
                format!("Remove labels {} from namespace {}", labels.join(", "), ns)
            }
            RemediationActionType::AddAnnotation => {
                let annotations: Vec<String> = action.parameters.keys().cloned().collect();
                format!("Add annotations {} to namespace {}", annotations.join(", "), ns)
            }
            RemediationActionType::PatchResource => {
                format!("Patch {}/{} in namespace {}", target.kind, target.name, ns)
            }
            RemediationActionType::CreateResource => {
                format!("Create {} in namespace {}", target.kind, ns)
            }
            RemediationActionType::DeleteResource => {
                format!("Delete {}/{} in namespace {}", target.kind, target.name, ns)
            }
            RemediationActionType::ApplyManifest => {
                format!("Apply manifest for {} in namespace {}", target.kind, ns)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namespace_security::baseline::{RemediationActionType, RemediationAction, TargetResource};

    #[test]
    fn test_remediation_action_serialization() {
        let action = RemediationAction {
            action_type: RemediationActionType::AddLabel,
            target_resource: TargetResource {
                api_version: "v1".to_string(),
                kind: "Namespace".to_string(),
                name: "test".to_string(),
                namespace: None,
            },
            parameters: BTreeMap::new(),
            description: "Test".to_string(),
        };
        
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("AddLabel"));
    }

    #[test]
    fn test_remediation_action_type_serialization() {
        let action_type = RemediationActionType::CreateResource;
        let json = serde_json::to_string(&action_type).unwrap();
        assert_eq!(json, "\"CreateResource\"");
    }
}
