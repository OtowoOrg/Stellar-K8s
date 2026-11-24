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
//! Change attribution for latency regressions

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use kube::{
    api::{Api, ListParams},
    Client,
};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::latency::deploy_marker::DeployMarker;
use crate::latency::sli_tracker::SLIDelta;

/// Attribution configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionConfig {
    /// Lookback window for finding related changes
    #[serde(default = "default_lookback_window")]
    pub lookback_window: String,
    /// Minimum correlation threshold
    #[serde(default = "default_correlation_threshold")]
    pub correlation_threshold: f64,
    /// Enable git commit analysis
    #[serde(default)]
    pub enable_git_analysis: bool,
    /// Git repository URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_repo_url: Option<String>,
    /// Enable Kubernetes event correlation
    #[serde(default)]
    pub enable_k8s_events: bool,
}

fn default_lookback_window() -> String { "24h".to_string() }
fn default_correlation_threshold() -> f64 { 0.7 }

impl Default for AttributionConfig {
    fn default() -> Self {
        Self {
            lookback_window: default_lookback_window(),
            correlation_threshold: default_correlation_threshold(),
            enable_git_analysis: false,
            git_repo_url: None,
            enable_k8s_events: true,
        }
    }
}

/// Latency attribution result
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyAttribution {
    pub sli_name: String,
    pub deploy_marker_id: String,
    pub timestamp: DateTime<Utc>,
    pub delta: f64,
    pub attributed_changes: Vec<AttributedChange>,
    pub correlation_score: f64,
    pub confidence: AttributionConfidence,
}

/// Attributed change
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributedChange {
    pub change_type: ChangeType,
    pub identifier: String,
    pub description: String,
    pub timestamp: DateTime<Utc>,
    pub correlation: f64,
    pub evidence: Vec<String>,
}

/// Change type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ChangeType {
    CodeDeployment,
    ConfigChange,
    InfrastructureChange,
    DependencyUpdate,
    DatabaseMigration,
    FeatureFlag,
    ScalingEvent,
    Unknown,
}

/// Attribution confidence level
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AttributionConfidence {
    High,
    Medium,
    Low,
    None,
}

/// Change attributor for linking latency regressions to deployments
pub struct ChangeAttributor {
    config: AttributionConfig,
    /// Kubernetes client for event correlation
    client: Option<Client>,
    /// Namespace for Kubernetes resources
    namespace: String,
    /// Prometheus metrics
    metrics: Arc<AttributionMetrics>,
    /// Attribution history
    history: Arc<RwLock<Vec<LatencyAttribution>>>,
}

/// Prometheus metrics for attribution
pub struct AttributionMetrics {
    pub attributions_total: Family<AttributionLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub attribution_confidence: Family<AttributionLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub changes_correlated: Family<AttributionLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AttributionLabels {
    pub sli_name: String,
    pub service: String,
    pub change_type: String,
}

impl AttributionMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            attributions_total: Family::default(),
            attribution_confidence: Family::default(),
            changes_correlated: Family::default(),
        };

        registry.register(
            "latency_attributions_total",
            "Total latency attributions performed",
            metrics.attributions_total.clone(),
        ).unwrap();

        registry.register(
            "latency_attribution_confidence",
            "Attribution confidence (0=none, 1=low, 2=medium, 3=high)",
            metrics.attribution_confidence.clone(),
        ).unwrap();

        registry.register(
            "latency_changes_correlated",
            "Number of changes correlated with latency regression",
            metrics.changes_correlated.clone(),
        ).unwrap();

        metrics
    }
}

impl ChangeAttributor {
    /// Create a new change attributor
    pub fn new(config: AttributionConfig, registry: &mut Registry) -> Self {
        Self {
            config,
            client: None,
            namespace: "default".to_string(),
            metrics: Arc::new(AttributionMetrics::new(registry)),
            history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create with Kubernetes client for event correlation
    pub fn with_kubernetes(mut self, client: Client, namespace: String) -> Self {
        self.client = Some(client);
        self.namespace = namespace;
        self
    }

    /// Attribute a latency regression to changes
    pub async fn attribute_regression(
        &self,
        sli_name: &str,
        service: &str,
        deploy_marker: &DeployMarker,
        delta: &SLIDelta,
    ) -> Result<LatencyAttribution> {
        info!(
            "Attributing regression for SLI {} deploy {} delta={:.2}%",
            sli_name, deploy_marker.id, delta.delta_percentage
        );

        let mut attributed_changes = Vec::new();

        // 1. Attribute to the deployment itself
        attributed_changes.push(AttributedChange {
            change_type: ChangeType::CodeDeployment,
            identifier: deploy_marker.id.clone(),
            description: format!("Deployment of {}@{}", deploy_marker.service, &deploy_marker.commit_sha[..8]),
            timestamp: deploy_marker.timestamp,
            correlation: 1.0, // Direct correlation
            evidence: vec![
                format!("Commit: {}", deploy_marker.commit_sha),
                format!("Version: {}", deploy_marker.version.as_deref().unwrap_or("unknown")),
                format!("Strategy: {:?}", deploy_marker.strategy),
            ],
        });

        // 2. Check for config changes in the same window
        if let Some(client) = &self.client {
            let config_changes = self.find_config_changes(client, service, deploy_marker.timestamp).await?;
            for change in config_changes {
                attributed_changes.push(change);
            }
        }

        // 3. Check for scaling events
        if let Some(client) = &self.client {
            let scaling_events = self.find_scaling_events(client, service, deploy_marker.timestamp).await?;
            for event in scaling_events {
                attributed_changes.push(event);
            }
        }

        // 4. Check for feature flag changes
        let feature_flags = self.find_feature_flag_changes(service, deploy_marker.timestamp).await?;
        for flag in feature_flags {
            attributed_changes.push(flag);
        }

        // Calculate overall correlation score
        let correlation_score = self.calculate_correlation_score(&attributed_changes);
        
        // Determine confidence
        let confidence = self.determine_confidence(correlation_score, &attributed_changes);

        let attribution = LatencyAttribution {
            sli_name: sli_name.to_string(),
            deploy_marker_id: deploy_marker.id.clone(),
            timestamp: Utc::now(),
            delta: delta.delta,
            attributed_changes: attributed_changes.clone(),
            correlation_score,
            confidence,
        };

        // Store in history
        {
            let mut history = self.history.write().await;
            history.push(attribution.clone());
            if history.len() > 1000 {
                history.remove(0);
            }
        }

        // Update metrics
        self.metrics.attributions_total
            .get_or_create(&AttributionLabels {
                sli_name: sli_name.to_string(),
                service: service.to_string(),
                change_type: "CodeDeployment".to_string(),
            })
            .inc();

        let confidence_value = match confidence {
            AttributionConfidence::High => 3,
            AttributionConfidence::Medium => 2,
            AttributionConfidence::Low => 1,
            AttributionConfidence::None => 0,
        };

        self.metrics.attribution_confidence
            .get_or_create(&AttributionLabels {
                sli_name: sli_name.to_string(),
                service: service.to_string(),
                change_type: "CodeDeployment".to_string(),
            })
            .set(confidence_value);

        self.metrics.changes_correlated
            .get_or_create(&AttributionLabels {
                sli_name: sli_name.to_string(),
                service: service.to_string(),
                change_type: "CodeDeployment".to_string(),
            })
            .set(attributed_changes.len() as i64);

        Ok(attribution)
    }

    /// Find config changes in Kubernetes
    async fn find_config_changes(
        &self,
        client: &Client,
        service: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<AttributedChange>> {
        let mut changes = Vec::new();
        
        // Check ConfigMaps
        let cm_api: Api<k8s_openapi::api::core::v1::ConfigMap> = Api::namespaced(client.clone(), &self.namespace);
        let label_selector = format!("app.kubernetes.io/name={}", service);
        
        if let Ok(list) = cm_api.list(&ListParams::default().labels(&label_selector)).await {
            for cm in list.items {
                if let Some(updated) = cm.metadata.creation_timestamp {
                    let created: DateTime<Utc> = updated.0.into();
                    if created >= since - self.config.lookback_window && created <= since + self.config.lookback_window {
                        changes.push(AttributedChange {
                            change_type: ChangeType::ConfigChange,
                            identifier: cm.name_any(),
                            description: format!("ConfigMap {} updated", cm.name_any()),
                            timestamp: created,
                            correlation: 0.8,
                            evidence: vec![
                                format!("ConfigMap: {}", cm.name_any()),
                                format!("Namespace: {}", cm.namespace().unwrap_or_default()),
                            ],
                        });
                    }
                }
            }
        }

        // Check Secrets
        let secret_api: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(client.clone(), &self.namespace);
        if let Ok(list) = secret_api.list(&ListParams::default().labels(&label_selector)).await {
            for secret in list.items {
                if let Some(updated) = secret.metadata.creation_timestamp {
                    let created: DateTime<Utc> = updated.0.into();
                    if created >= since - self.config.lookback_window && created <= since + self.config.lookback_window {
                        changes.push(AttributedChange {
                            change_type: ChangeType::ConfigChange,
                            identifier: secret.name_any(),
                            description: format!("Secret {} updated", secret.name_any()),
                            timestamp: created,
                            correlation: 0.7,
                            evidence: vec![
                                format!("Secret: {}", secret.name_any()),
                            ],
                        });
                    }
                }
            }
        }

        Ok(changes)
    }

    /// Find scaling events (HPA, VPA, manual replica changes)
    async fn find_scaling_events(
        &self,
        client: &Client,
        service: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<AttributedChange>> {
        let mut events = Vec::new();
        
        // Check Deployments for replica changes
        let deploy_api: Api<k8s_openapi::api::apps::v1::Deployment> = Api::namespaced(client.clone(), &self.namespace);
        let label_selector = format!("app.kubernetes.io/name={}", service);
        
        if let Ok(list) = deploy_api.list(&ListParams::default().labels(&label_selector)).await {
            for deploy in list.items {
                if let Some(replicas) = deploy.spec.as_ref().and_then(|s| s.replicas) {
                    if replicas > 1 {
                        events.push(AttributedChange {
                            change_type: ChangeType::ScalingEvent,
                            identifier: deploy.name_any(),
                            description: format!("Deployment {} scaled to {} replicas", deploy.name_any(), replicas),
                            timestamp: since, // Approximate
                            correlation: 0.6,
                            evidence: vec![
                                format!("Deployment: {}", deploy.name_any()),
                                format!("Replicas: {}", replicas),
                            ],
                        });
                    }
                }
            }
        }

        Ok(events)
    }

    /// Find feature flag changes
    async fn find_feature_flag_changes(
        &self,
        service: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<AttributedChange>> {
        // In a real implementation, this would query a feature flag service
        // For now, return empty
        Ok(vec![])
    }

    /// Calculate correlation score from attributed changes
    fn calculate_correlation_score(&self, changes: &[AttributedChange]) -> f64 {
        if changes.is_empty() {
            return 0.0;
        }
        
        // Weighted average of correlations
        let total_weight: f64 = changes.iter().map(|c| c.correlation).sum();
        total_weight / changes.len() as f64
    }

    /// Determine confidence level
    fn determine_confidence(&self, score: f64, changes: &[AttributedChange]) -> AttributionConfidence {
        if score >= 0.9 && changes.iter().any(|c| c.change_type == ChangeType::CodeDeployment) {
            AttributionConfidence::High
        } else if score >= 0.7 {
            AttributionConfidence::Medium
        } else if score >= 0.5 {
            AttributionConfidence::Low
        } else {
            AttributionConfidence::None
        }
    }

    /// Get attribution history
    pub async fn get_history(&self) -> Vec<LatencyAttribution> {
        self.history.read().await.clone()
    }

    /// Get attributions for a deploy marker
    pub async fn get_for_deploy(&self, deploy_marker_id: &str) -> Vec<LatencyAttribution> {
        self.history.read().await.iter()
            .filter(|a| a.deploy_marker_id == deploy_marker_id)
            .cloned()
            .collect()
    }

    /// Get attributions for an SLI
    pub async fn get_for_sli(&self, sli_name: &str) -> Vec<LatencyAttribution> {
        self.history.read().await.iter()
            .filter(|a| a.sli_name == sli_name)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribution_config_defaults() {
        let config = AttributionConfig::default();
        assert_eq!(config.lookback_window, Duration::hours(24));
        assert_eq!(config.correlation_threshold, 0.7);
    }

    #[test]
    fn test_change_type_serialization() {
        let ct = ChangeType::CodeDeployment;
        let json = serde_json::to_string(&ct).unwrap();
        assert_eq!(json, "\"CodeDeployment\"");
    }

    #[test]
    fn test_attribution_confidence_serialization() {
        let conf = AttributionConfidence::High;
        let json = serde_json::to_string(&conf).unwrap();
        assert_eq!(json, "\"High\"");
    }
}
