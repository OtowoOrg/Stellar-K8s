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
//! Quarantine manager for offending metric series

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::cardinality::collector::MetricSeries;
use crate::error::{Error, Result};

/// Quarantine action type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum QuarantineAction {
    Quarantine,
    Release,
    Drop,
}

/// Quarantined series record
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantinedSeries {
    pub metric_name: String,
    pub labels: BTreeMap<String, String>,
    pub team: String,
    pub quarantined_at: DateTime<Utc>,
    pub reason: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub original_series: MetricSeries,
}

/// Quarantine manager for handling offending series
pub struct QuarantineManager {
    /// Quarantined series: metric_name -> label_hash -> QuarantinedSeries
    quarantined: Arc<RwLock<HashMap<String, HashMap<u64, QuarantinedSeries>>>>,
    /// Default quarantine duration
    default_duration: chrono::Duration,
    /// Kubernetes client for CR persistence
    client: Option<Client>,
    /// Namespace for quarantine CRs
    namespace: Option<String>,
    /// Prometheus metrics
    metrics: Arc<QuarantineMetrics>,
}

/// Prometheus metrics for quarantine
pub struct QuarantineMetrics {
    pub quarantined_total: Family<TeamMetricLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub released_total: Family<TeamMetricLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub dropped_total: Family<TeamMetricLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub currently_quarantined: Family<TeamMetricLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TeamMetricLabels {
    pub team: String,
    pub metric_name: String,
}

impl QuarantineMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            quarantined_total: Family::default(),
            released_total: Family::default(),
            dropped_total: Family::default(),
            currently_quarantined: Family::default(),
        };

        registry.register(
            "cardinality_quarantined_total",
            "Total series quarantined per team per metric",
            metrics.quarantined_total.clone(),
        ).unwrap();

        registry.register(
            "cardinality_released_total",
            "Total series released from quarantine per team per metric",
            metrics.released_total.clone(),
        ).unwrap();

        registry.register(
            "cardinality_dropped_total",
            "Total series dropped from quarantine per team per metric",
            metrics.dropped_total.clone(),
        ).unwrap();

        registry.register(
            "cardinality_currently_quarantined",
            "Currently quarantined series per team per metric",
            metrics.currently_quarantined.clone(),
        ).unwrap();

        metrics
    }
}

impl QuarantineManager {
    /// Create a new quarantine manager
    pub fn new(default_duration: chrono::Duration, registry: &mut Registry) -> Self {
        Self {
            quarantined: Arc::new(RwLock::new(HashMap::new())),
            default_duration,
            client: None,
            namespace: None,
            metrics: Arc::new(QuarantineMetrics::new(registry)),
        }
    }

    /// Create with Kubernetes persistence
    pub fn with_persistence(mut self, client: Client, namespace: String) -> Self {
        self.client = Some(client);
        self.namespace = Some(namespace);
        self
    }

    /// Quarantine a metric series
    pub async fn quarantine_series(
        &self,
        metric_name: &str,
        labels: BTreeMap<String, String>,
        team: String,
        reason: String,
    ) -> Result<()> {
        let label_hash = self.hash_labels(&labels);
        let now = Utc::now();
        let expires_at = now + self.default_duration;

        // Get original series info
        let original_series = MetricSeries {
            metric_name: metric_name.to_string(),
            labels: labels.clone(),
            team: team.clone(),
            first_seen: now,
            last_seen: now,
            sample_count: 0,
            is_quarantined: true,
            quarantine_reason: Some(reason.clone()),
        };

        let quarantined = QuarantinedSeries {
            metric_name: metric_name.to_string(),
            labels: labels.clone(),
            team: team.clone(),
            quarantined_at: now,
            reason: reason.clone(),
            expires_at: Some(expires_at),
            original_series,
        };

        // Store in memory
        {
            let mut quarantined_map = self.quarantined.write().await;
            let metric_map = quarantined_map.entry(metric_name.to_string()).or_default();
            metric_map.insert(label_hash, quarantined);
        }

        // Update metrics
        self.metrics.quarantined_total
            .get_or_create(&TeamMetricLabels {
                team: team.clone(),
                metric_name: metric_name.to_string(),
            })
            .inc();

        self.metrics.currently_quarantined
            .get_or_create(&TeamMetricLabels {
                team: team.clone(),
                metric_name: metric_name.to_string(),
            })
            .inc();

        // Persist to Kubernetes if configured
        if let (Some(client), Some(namespace)) = (&self.client, &self.namespace) {
            self.persist_quarantine(client, namespace, &quarantined).await?;
        }

        info!(
            "Quarantined series for team {} metric {}: {} (reason: {})",
            team, metric_name, label_hash, reason
        );

        Ok(())
    }

    /// Release a series from quarantine
    pub async fn release_series(&self, metric_name: &str, labels: &BTreeMap<String, String>) -> Result<bool> {
        let label_hash = self.hash_labels(labels);
        let team = self.get_team_for_series(metric_name, label_hash).await;

        let mut quarantined_map = self.quarantined.write().await;
        if let Some(metric_map) = quarantined_map.get_mut(metric_name) {
            if metric_map.remove(&label_hash).is_some() {
                if let Some(team) = team {
                    self.metrics.released_total
                        .get_or_create(&TeamMetricLabels {
                            team: team.clone(),
                            metric_name: metric_name.to_string(),
                        })
                        .inc();

                    self.metrics.currently_quarantined
                        .get_or_create(&TeamMetricLabels {
                            team,
                            metric_name: metric_name.to_string(),
                        })
                        .dec();

                    info!("Released series from quarantine: {}/{}", metric_name, label_hash);
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Drop (permanently remove) a quarantined series
    pub async fn drop_series(&self, metric_name: &str, labels: &BTreeMap<String, String>) -> Result<bool> {
        let label_hash = self.hash_labels(labels);
        let team = self.get_team_for_series(metric_name, label_hash).await;

        let mut quarantined_map = self.quarantined.write().await;
        if let Some(metric_map) = quarantined_map.get_mut(metric_name) {
            if metric_map.remove(&label_hash).is_some() {
                if let Some(team) = team {
                    self.metrics.dropped_total
                        .get_or_create(&TeamMetricLabels {
                            team: team.clone(),
                            metric_name: metric_name.to_string(),
                        })
                        .inc();

                    self.metrics.currently_quarantined
                        .get_or_create(&TeamMetricLabels {
                            team,
                            metric_name: metric_name.to_string(),
                        })
                        .dec();

                    warn!("Dropped quarantined series: {}/{}", metric_name, label_hash);
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Check if a series is quarantined
    pub async fn is_quarantined(&self, metric_name: &str, labels: &BTreeMap<String, String>) -> bool {
        let label_hash = self.hash_labels(labels);
        let quarantined = self.quarantined.read().await;
        quarantined.get(metric_name)
            .and_then(|m| m.get(&label_hash))
            .is_some()
    }

    /// Get all quarantined series for a team
    pub async fn get_quarantined_for_team(&self, team: &str) -> Vec<QuarantinedSeries> {
        let quarantined = self.quarantined.read().await;
        let mut result = Vec::new();
        for metric_map in quarantined.values() {
            for qs in metric_map.values() {
                if qs.team == team {
                    result.push(qs.clone());
                }
            }
        }
        result
    }

    /// Get all quarantined series
    pub async fn get_all_quarantined(&self) -> Vec<QuarantinedSeries> {
        let quarantined = self.quarantined.read().await;
        let mut result = Vec::new();
        for metric_map in quarantined.values() {
            for qs in metric_map.values() {
                result.push(qs.clone());
            }
        }
        result
    }

    /// Clean up expired quarantines
    pub async fn cleanup_expired(&self) -> usize {
        let now = Utc::now();
        let mut quarantined_map = self.quarantined.write().await;
        let mut cleaned = 0;

        for metric_map in quarantined_map.values_mut() {
            let initial_len = metric_map.len();
            metric_map.retain(|_, qs| {
                qs.expires_at.map(|e| e > now).unwrap_or(true)
            });
            cleaned += initial_len - metric_map.len();
        }

        // Update metrics for cleaned series
        if cleaned > 0 {
            for metric_map in quarantined_map.values() {
                for qs in metric_map.values() {
                    self.metrics.currently_quarantined
                        .get_or_create(&TeamMetricLabels {
                            team: qs.team.clone(),
                            metric_name: qs.metric_name.clone(),
                        })
                        .set(metric_map.len() as i64);
                }
            }
        }

        if cleaned > 0 {
            info!("Cleaned up {} expired quarantined series", cleaned);
        }

        cleaned
    }

    /// Get team for a series
    async fn get_team_for_series(&self, metric_name: &str, label_hash: u64) -> Option<String> {
        let quarantined = self.quarantined.read().await;
        quarantined.get(metric_name)
            .and_then(|m| m.get(&label_hash))
            .map(|qs| qs.team.clone())
    }

    /// Hash labels for lookup
    fn hash_labels(&self, labels: &BTreeMap<String, String>) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        for (k, v) in labels {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Persist quarantine to Kubernetes
    async fn persist_quarantine(&self, client: &Client, namespace: &str, qs: &QuarantinedSeries) -> Result<()> {
        // In a full implementation, this would create/update a Quarantine CR
        // For now, we'll just log
        debug!("Would persist quarantine for {}/{} to {}/{}", qs.metric_name, qs.team, namespace, qs.metric_name);
        Ok(())
    }

    /// Start background cleanup task
    pub async fn start_cleanup_task(self: Arc<Self>, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            self.cleanup_expired().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use chrono::Duration;

    #[tokio::test]
    async fn test_quarantine_and_release() {
        let mut registry = Registry::default();
        let manager = QuarantineManager::new(Duration::hours(1), &mut registry);
        
        let mut labels = BTreeMap::new();
        labels.insert("team".to_string(), "test-team".to_string());
        labels.insert("service".to_string(), "test-service".to_string());
        
        manager.quarantine_series("test_metric", labels.clone(), "test-team".to_string(), "Budget exceeded".to_string()).await.unwrap();
        
        let is_quarantined = manager.is_quarantined("test_metric", &labels).await;
        assert!(is_quarantined);
        
        let released = manager.release_series("test_metric", &labels).await.unwrap();
        assert!(released);
        
        let is_quarantined = manager.is_quarantined("test_metric", &labels).await;
        assert!(!is_quarantined);
    }

    #[tokio::test]
    async fn test_quarantine_drop() {
        let mut registry = Registry::default();
        let manager = QuarantineManager::new(Duration::hours(1), &mut registry);
        
        let mut labels = BTreeMap::new();
        labels.insert("team".to_string(), "test-team".to_string());
        
        manager.quarantine_series("test_metric", labels.clone(), "test-team".to_string(), "Budget exceeded".to_string()).await.unwrap();
        
        let dropped = manager.drop_series("test_metric", &labels).await.unwrap();
        assert!(dropped);
        
        let is_quarantined = manager.is_quarantined("test_metric", &labels).await;
        assert!(!is_quarantined);
    }
}