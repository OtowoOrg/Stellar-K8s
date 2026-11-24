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
//! Deploy marker emission for latency tracking

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
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::{Error, Result};

/// Configuration for deploy marker emission
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployMarkerConfig {
    /// Kubernetes client for CR operations
    pub client: Option<Client>,
    /// Namespace for deploy markers
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Emit to Prometheus metrics
    #[serde(default)]
    pub emit_prometheus: bool,
    /// Emit to Kafka topic
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kafka_topic: Option<String>,
    /// Emit to NATS subject
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats_subject: Option<String>,
    /// Default labels to include
    #[serde(default)]
    pub default_labels: BTreeMap<String, String>,
}

fn default_namespace() -> String { "observability".to_string() }

impl Default for DeployMarkerConfig {
    fn default() -> Self {
        Self {
            client: None,
            namespace: default_namespace(),
            emit_prometheus: true,
            kafka_topic: None,
            nats_subject: None,
            default_labels: BTreeMap::new(),
        }
    }
}

/// Deploy marker representing a code deployment
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployMarker {
    /// Unique marker ID
    pub id: String,
    /// Deployment timestamp
    pub timestamp: DateTime<Utc>,
    /// Service/component name
    pub service: String,
    /// Environment (production, staging, etc.)
    pub environment: String,
    /// Git commit SHA
    pub commit_sha: String,
    /// Git ref (branch/tag)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Deployment version/tag
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Deployment strategy
    #[serde(default)]
    pub strategy: DeploymentStrategy,
    /// Deployer identity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployed_by: Option<String>,
    /// Related change/ticket IDs
    #[serde(default)]
    pub change_ids: Vec<String>,
    /// Custom metadata
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Pre-deploy baseline SLI values
    #[serde(default)]
    pub pre_deploy_baselines: BTreeMap<String, f64>,
}

/// Deployment strategy
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum DeploymentStrategy {
    #[default]
    Rolling,
    BlueGreen,
    Canary,
    Recreate,
    Custom(String),
}

/// Deploy marker emitter
pub struct DeployMarkerEmitter {
    config: DeployMarkerConfig,
    /// Prometheus metrics
    metrics: Option<Arc<DeployMarkerMetrics>>,
    /// Emitted markers history
    history: Arc<RwLock<Vec<DeployMarker>>>,
    max_history: usize,
}

/// Prometheus metrics for deploy markers
pub struct DeployMarkerMetrics {
    pub deployments_total: Family<DeployLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub deployment_duration_seconds: Family<DeployLabels, Histogram>,
    pub active_deployments: Family<DeployLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct DeployLabels {
    pub service: String,
    pub environment: String,
    pub strategy: String,
}

impl DeployMarkerMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            deployments_total: Family::default(),
            deployment_duration_seconds: Family::default(),
            active_deployments: Family::default(),
        };

        registry.register(
            "deploy_markers_total",
            "Total deploy markers emitted",
            metrics.deployments_total.clone(),
        ).unwrap();

        registry.register(
            "deploy_marker_duration_seconds",
            "Deployment duration in seconds",
            metrics.deployment_duration_seconds.clone(),
        ).unwrap();

        registry.register(
            "deploy_markers_active",
            "Currently active deployments",
            metrics.active_deployments.clone(),
        ).unwrap();

        metrics
    }
}

impl DeployMarkerEmitter {
    /// Create a new deploy marker emitter
    pub fn new(config: DeployMarkerConfig, registry: Option<&mut Registry>) -> Self {
        let metrics = registry.map(|r| Arc::new(DeployMarkerMetrics::new(r)));
        Self {
            config,
            metrics,
            history: Arc::new(RwLock::new(Vec::new())),
            max_history: 1000,
        }
    }

    /// Emit a deploy marker
    pub async fn emit(&self, mut marker: DeployMarker) -> Result<DeployMarker> {
        // Generate ID if not provided
        if marker.id.is_empty() {
            marker.id = Uuid::new_v4().to_string();
        }
        
        // Set timestamp if not provided
        if marker.timestamp == DateTime::UNIX_EPOCH {
            marker.timestamp = Utc::now();
        }

        // Apply default labels
        for (k, v) in &self.config.default_labels {
            marker.metadata.entry(k.clone()).or_insert(v.clone());
        }

        info!(
            "Emitting deploy marker: id={} service={} version={} strategy={:?}",
            marker.id, marker.service, marker.version.as_deref().unwrap_or("unknown"), marker.strategy
        );

        // Emit to Prometheus
        if self.config.emit_prometheus {
            if let Some(metrics) = &self.metrics {
                metrics.deployments_total
                    .get_or_create(&DeployLabels {
                        service: marker.service.clone(),
                        environment: marker.environment.clone(),
                        strategy: format!("{:?}", marker.strategy),
                    })
                    .inc();
                
                metrics.active_deployments
                    .get_or_create(&DeployLabels {
                        service: marker.service.clone(),
                        environment: marker.environment.clone(),
                        strategy: format!("{:?}", marker.strategy),
                    })
                    .inc();
            }
        }

        // Store in history
        {
            let mut history = self.history.write().await;
            history.push(marker.clone());
            if history.len() > self.max_history {
                history.remove(0);
            }
        }

        // Persist to Kubernetes if client available
        if let Some(client) = &self.config.client {
            self.persist_to_kubernetes(client, &marker).await?;
        }

        // Emit to Kafka if configured
        if let Some(topic) = &self.config.kafka_topic {
            self.emit_to_kafka(topic, &marker).await?;
        }

        // Emit to NATS if configured
        if let Some(subject) = &self.config.nats_subject {
            self.emit_to_nats(subject, &marker).await?;
        }

        Ok(marker)
    }

    /// Emit a deployment start marker
    pub async fn emit_start(&self, service: &str, environment: &str, commit_sha: &str) -> Result<DeployMarker> {
        let marker = DeployMarker {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            service: service.to_string(),
            environment: environment.to_string(),
            commit_sha: commit_sha.to_string(),
            git_ref: None,
            version: None,
            strategy: DeploymentStrategy::Rolling,
            deployed_by: None,
            change_ids: vec![],
            metadata: BTreeMap::new(),
            pre_deploy_baselines: BTreeMap::new(),
        };
        self.emit(marker).await
    }

    /// Emit a deployment completion marker with duration
    pub async fn emit_complete(
        &self,
        marker: &DeployMarker,
        duration: std::time::Duration,
        success: bool,
    ) -> Result<()> {
        // Update Prometheus metrics
        if let Some(metrics) = &self.metrics {
            metrics.deployment_duration_seconds
                .get_or_create(&DeployLabels {
                    service: marker.service.clone(),
                    environment: marker.environment.clone(),
                    strategy: format!("{:?}", marker.strategy),
                })
                .observe(duration.as_secs_f64());

            metrics.active_deployments
                .get_or_create(&DeployLabels {
                    service: marker.service.clone(),
                    environment: marker.environment.clone(),
                    strategy: format!("{:?}", marker.strategy),
                })
                .dec();
        }

        // Could emit a completion event here
        info!(
            "Deployment completed: service={} duration={:.2}s success={}",
            marker.service, duration.as_secs_f64(), success
        );

        Ok(())
    }

    /// Get recent deploy markers
    pub async fn get_recent(&self, limit: usize) -> Vec<DeployMarker> {
        let history = self.history.read().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Get deploy markers for a service
    pub async fn get_for_service(&self, service: &str) -> Vec<DeployMarker> {
        let history = self.history.read().await;
        history.iter()
            .filter(|m| m.service == service)
            .cloned()
            .collect()
    }

    /// Get deploy markers in time range
    pub async fn get_in_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<DeployMarker> {
        let history = self.history.read().await;
        history.iter()
            .filter(|m| m.timestamp >= start && m.timestamp <= end)
            .cloned()
            .collect()
    }

    /// Persist deploy marker to Kubernetes as annotation on a ConfigMap
    async fn persist_to_kubernetes(&self, client: &Client, marker: &DeployMarker) -> Result<()> {
        let api: Api<k8s_openapi::api::core::v1::ConfigMap> = 
            Api::namespaced(client.clone(), &self.config.namespace);
        
        let cm_name = format!("deploy-marker-{}", marker.id);
        let data = serde_json::to_string(marker).map_err(Error::SerializationError)?;
        
        let cm = k8s_openapi::api::core::v1::ConfigMap {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(cm_name.clone()),
                namespace: Some(self.config.namespace.clone()),
                labels: Some(BTreeMap::from([
                    ("observability.stellar.org/type".to_string(), "deploy-marker".to_string()),
                    ("observability.stellar.org/service".to_string(), marker.service.clone()),
                    ("observability.stellar.org/environment".to_string(), marker.environment.clone()),
                ])),
                annotations: Some(BTreeMap::from([
                    ("observability.stellar.org/commit-sha".to_string(), marker.commit_sha.clone()),
                    ("observability.stellar.org/timestamp".to_string(), marker.timestamp.to_rfc3339()),
                ])),
                ..Default::default()
            },
            data: Some(BTreeMap::from([
                ("marker.json".to_string(), data),
            ])),
            ..Default::default()
        };

        api.create(&kube::api::PostParams::default(), &cm).await
            .or_else(|e| {
                if e.to_string().contains("already exists") {
                    let patch = serde_json::json!({
                        "data": { "marker.json": data }
                    });
                    api.patch(&cm_name, &PatchParams::apply("deploy-marker"), &Patch::Merge(&patch)).await
                } else {
                    Err(e)
                }
            })
            .map_err(Error::KubeError)?;

        Ok(())
    }

    /// Emit to Kafka (placeholder)
    async fn emit_to_kafka(&self, topic: &str, marker: &DeployMarker) -> Result<()> {
        // In a real implementation, use rdkafka
        debug!("Would emit deploy marker to Kafka topic {}: {}", topic, marker.id);
        Ok(())
    }

    /// Emit to NATS (placeholder)
    async fn emit_to_nats(&self, subject: &str, marker: &DeployMarker) -> Result<()> {
        // In a real implementation, use async-nats
        debug!("Would emit deploy marker to NATS subject {}: {}", subject, marker.id);
        Ok(())
    }
}

/// Helper to create deploy marker from deployment annotation
pub fn create_marker_from_annotation(
    annotations: &BTreeMap<String, String>,
) -> Option<DeployMarker> {
    let commit_sha = annotations.get("deployment.stellar.org/commit-sha")?;
    let service = annotations.get("deployment.stellar.org/service")?;
    let environment = annotations.get("deployment.stellar.org/environment")?;
    
    Some(DeployMarker {
        id: String::new(),
        timestamp: Utc::now(),
        service: service.clone(),
        environment: environment.clone(),
        commit_sha: commit_sha.clone(),
        git_ref: annotations.get("deployment.stellar.org/git-ref").cloned(),
        version: annotations.get("deployment.stellar.org/version").cloned(),
        strategy: annotations.get("deployment.stellar.org/strategy")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default(),
        deployed_by: annotations.get("deployment.stellar.org/deployed-by").cloned(),
        change_ids: annotations.get("deployment.stellar.org/change-ids")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default(),
        metadata: BTreeMap::new(),
        pre_deploy_baselines: BTreeMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_deploy_marker_creation() {
        let marker = DeployMarker {
            id: "test-123".to_string(),
            timestamp: Utc::now(),
            service: "api-gateway".to_string(),
            environment: "production".to_string(),
            commit_sha: "abc123".to_string(),
            git_ref: Some("main".to_string()),
            version: Some("v1.2.3".to_string()),
            strategy: DeploymentStrategy::Canary,
            deployed_by: Some("ci-bot".to_string()),
            change_ids: vec!["PR-456".to_string()],
            metadata: BTreeMap::new(),
            pre_deploy_baselines: BTreeMap::new(),
        };
        
        let json = serde_json::to_string(&marker).unwrap();
        assert!(json.contains("api-gateway"));
        assert!(json.contains("abc123"));
    }

    #[test]
    fn test_deployment_strategy_serialization() {
        let strategy = DeploymentStrategy::Canary;
        let json = serde_json::to_string(&strategy).unwrap();
        assert_eq!(json, "\"Canary\"");
        
        let custom = DeploymentStrategy::Custom("my-strategy".to_string());
        let json = serde_json::to_string(&custom).unwrap();
        assert!(json.contains("my-strategy"));
    }
}