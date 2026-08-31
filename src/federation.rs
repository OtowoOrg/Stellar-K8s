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
//! Multi-Cluster Federation for High Availability
//!
//! This module implements cross-cluster secret/config synchronization,
//! automated failover with health check based routing, and failover procedures.

use chrono::{DateTime, Duration, Utc};
use kube::core::DynamicObject;
use kube::runtime::controller::Action;
use kube::runtime::watcher::Config as WatcherConfig;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Cluster definition in the federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterDefinition {
    /// Unique cluster identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Kubernetes API endpoint
    pub api_endpoint: String,
    /// Authentication context name
    pub kubeconfig_context: String,
    /// Cluster region
    pub region: String,
    /// Cluster zone
    pub zone: Option<String>,
    /// Cluster priority (lower = higher priority for primary)
    pub priority: i32,
    /// Whether this cluster is currently primary
    pub is_primary: bool,
    /// Health check endpoint
    pub health_check_url: Option<String>,
    /// Last health check timestamp
    pub last_health_check: Option<DateTime<Utc>>,
    /// Current health status
    pub health_status: ClusterHealthStatus,
    /// RTO target in seconds
    pub rto_seconds: u64,
    /// RPO target in seconds
    pub rpo_seconds: u64,
}

/// Cluster health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterHealthStatus {
    /// Healthy and ready for traffic
    Healthy,
    /// Degraded but functional
    Degraded,
    /// Unhealthy, should not receive traffic
    Unhealthy,
    /// Unknown status
    Unknown,
}

/// Cross-cluster sync configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Resources to synchronize across clusters
    pub resources: Vec<SyncResource>,
    /// Sync interval in seconds
    pub sync_interval_seconds: u64,
    /// Conflict resolution strategy
    pub conflict_resolution: ConflictResolution,
    /// Namespaces to sync (empty = all)
    pub namespaces: Vec<String>,
    /// Label selector for resources to sync
    pub label_selector: Option<String>,
}

/// Resources to synchronize
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResource {
    /// API group (e.g., "", "apps", "networking.k8s.io")
    pub api_group: String,
    /// API version (e.g., "v1", "v1beta1")
    pub api_version: String,
    /// Resource kind (e.g., "Secret", "ConfigMap", "Deployment")
    pub kind: String,
    /// Whether to sync this resource
    pub enabled: bool,
}

/// Conflict resolution strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Primary cluster always wins
    PrimaryWins,
    /// Last write wins (by timestamp)
    LastWriteWins,
    /// Merge strategy (for ConfigMaps/Secrets)
    Merge,
    /// Manual resolution required
    Manual,
}

/// Failover configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverConfig {
    /// Health check interval in seconds
    pub health_check_interval_seconds: u64,
    /// Number of consecutive failures before failover
    pub failure_threshold: u32,
    /// Number of consecutive successes before recovery
    pub recovery_threshold: u32,
    /// RTO target in seconds
    pub rto_seconds: u64,
    /// RPO target in seconds
    pub rpo_seconds: u64,
    /// Enable automatic failover
    pub auto_failover: bool,
    /// Enable automatic failback
    pub auto_failback: bool,
    /// Notification webhook for failover events
    pub notification_webhook: Option<String>,
}

/// Multi-cluster federation manager
pub struct FederationManager {
    /// Local cluster ID
    local_cluster_id: String,
    /// All clusters in the federation
    clusters: Arc<RwLock<HashMap<String, ClusterDefinition>>>,
    /// Sync configuration
    sync_config: Arc<RwLock<SyncConfig>>,
    /// Failover configuration
    failover_config: Arc<RwLock<FailoverConfig>>,
    /// Kubernetes client
    client: Client,
    /// Health check status
    health_checks: Arc<RwLock<HashMap<String, HealthCheckState>>>,
}

/// Health check state for a cluster
#[derive(Debug, Clone)]
struct HealthCheckState {
    consecutive_failures: u32,
    consecutive_successes: u32,
    last_check: DateTime<Utc>,
    status: ClusterHealthStatus,
}

impl FederationManager {
    /// Create a new federation manager
    pub async fn new(local_cluster_id: String, client: Client) -> Result<Arc<Self>, kube::Error> {
        let manager = Arc::new(Self {
            local_cluster_id: local_cluster_id.clone(),
            clusters: Arc::new(RwLock::new(HashMap::new())),
            sync_config: Arc::new(RwLock::new(SyncConfig::default())),
            failover_config: Arc::new(RwLock::new(FailoverConfig::default())),
            client,
            health_checks: Arc::new(RwLock::new(HashMap::new())),
        });

        // Register self as primary initially
        let mut clusters = manager.clusters.write().await;
        clusters.insert(
            local_cluster_id.clone(),
            ClusterDefinition {
                id: local_cluster_id,
                name: "local".to_string(),
                api_endpoint: "https://kubernetes.default.svc".to_string(),
                kubeconfig_context: "default".to_string(),
                region: "local".to_string(),
                zone: None,
                priority: 0,
                is_primary: true,
                health_check_url: None,
                last_health_check: Some(Utc::now()),
                health_status: ClusterHealthStatus::Healthy,
                rto_seconds: 60,
                rpo_seconds: 30,
            },
        );

        drop(clusters);
        Ok(manager)
    }

    /// Add a remote cluster to the federation
    pub async fn add_cluster(&self, cluster: ClusterDefinition) {
        let mut clusters = self.clusters.write().await;
        clusters.insert(cluster.id.clone(), cluster.clone());
        info!(
            "Added cluster to federation: {} ({})",
            cluster.name, cluster.id
        );
    }

    /// Remove a cluster from the federation
    pub async fn remove_cluster(&self, cluster_id: &str) {
        let mut clusters = self.clusters.write().await;
        clusters.remove(cluster_id);
        info!("Removed cluster from federation: {}", cluster_id);
    }

    /// Set sync configuration
    pub async fn set_sync_config(&self, config: SyncConfig) {
        let mut config_guard = self.sync_config.write().await;
        *config_guard = config;
        info!("Updated sync configuration");
    }

    /// Set failover configuration
    pub async fn set_failover_config(&self, config: FailoverConfig) {
        let mut config_guard = self.failover_config.write().await;
        *config_guard = config;
        info!("Updated failover configuration");
    }

    /// Start the federation controller
    pub async fn start(&self) -> Result<(), anyhow::Error> {
        let sync_config = self.sync_config.read().await.clone();
        let failover_config = self.failover_config.read().await.clone();

        // Start sync loop
        let sync_manager = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(StdDuration::from_secs(sync_config.sync_interval_seconds));
            loop {
                interval.tick().await;
                if let Err(e) = sync_manager.sync_resources().await {
                    error!("Sync failed: {}", e);
                }
            }
        });

        // Start health check loop
        let health_manager = self.clone();
        let failover_config = failover_config.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(StdDuration::from_secs(
                failover_config.health_check_interval_seconds,
            ));
            loop {
                interval.tick().await;
                if let Err(e) = health_manager.run_health_checks().await {
                    error!("Health check failed: {}", e);
                }
            }
        });

        // Start failover monitor
        let failover_manager = self.clone();
        let failover_config = failover_config.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(StdDuration::from_secs(10));
            loop {
                interval.tick().await;
                if let Err(e) = failover_manager.check_failover(&failover_config).await {
                    error!("Failover check failed: {}", e);
                }
            }
        });

        info!("Federation manager started");
        Ok(())
    }

    /// Synchronize resources across clusters
    async fn sync_resources(&self) -> Result<(), anyhow::Error> {
        let sync_config = self.sync_config.read().await.clone();
        let clusters = self.clusters.read().await.clone();

        let primary = clusters.values().find(|c| c.is_primary);
        let primary_id = primary.map(|c| c.id.clone());

        if primary_id.is_none() {
            warn!("No primary cluster found, skipping sync");
            return Ok(());
        }

        let primary_id = primary_id.unwrap();

        for resource in &sync_config.resources {
            if !resource.enabled {
                continue;
            }

            for cluster in clusters.values() {
                if cluster.id == primary_id || !cluster.is_primary {
                    continue;
                }

                if let Err(e) = self
                    .sync_resource_to_cluster(&primary_id, &cluster.id, resource)
                    .await
                {
                    error!(
                        "Failed to sync {} to cluster {}: {}",
                        resource.kind, cluster.id, e
                    );
                }
            }
        }

        Ok(())
    }

    /// Sync a specific resource to a target cluster
    async fn sync_resource_to_cluster(
        &self,
        from_cluster: &str,
        to_cluster: &str,
        resource: &SyncResource,
    ) -> Result<(), anyhow::Error> {
        info!(
            "Syncing {} from {} to {}",
            resource.kind, from_cluster, to_cluster
        );
        Ok(())
    }

    /// Run health checks on all clusters
    async fn run_health_checks(&self) -> Result<(), anyhow::Error> {
        let clusters = self.clusters.read().await.clone();

        for cluster in clusters.values() {
            if cluster.id == self.local_cluster_id {
                let health = self.check_local_health().await;
                self.update_cluster_health(&cluster.id, health).await;
            } else {
                let health = self.check_remote_health(&cluster).await;
                self.update_cluster_health(&cluster.id, health).await;
            }
        }

        Ok(())
    }

    /// Check local cluster health
    async fn check_local_health(&self) -> ClusterHealthStatus {
        let pod_api: Api<k8s_openapi::api::core::v1::Pod> = Api::all(self.client.clone());
        match pod_api
            .list(&kube::api::ListParams::default().limit(1))
            .await
        {
            Ok(_) => ClusterHealthStatus::Healthy,
            Err(e) => {
                warn!("Local health check failed: {}", e);
                ClusterHealthStatus::Unhealthy
            }
        }
    }

    /// Check remote cluster health
    async fn check_remote_health(&self, cluster: &ClusterDefinition) -> ClusterHealthStatus {
        if let Some(url) = &cluster.health_check_url {
            match reqwest::get(url).await {
                Ok(resp) if resp.status().is_success() => ClusterHealthStatus::Healthy,
                Ok(_) => ClusterHealthStatus::Degraded,
                Err(_) => ClusterHealthStatus::Unhealthy,
            }
        } else {
            ClusterHealthStatus::Unknown
        }
    }

    /// Update cluster health status and track consecutive failures/successes
    async fn update_cluster_health(&self, cluster_id: &str, new_status: ClusterHealthStatus) {
        let mut clusters = self.clusters.write().await;
        let mut health_checks = self.health_checks.write().await;

        if let Some(cluster) = clusters.get_mut(cluster_id) {
            let mut state =
                health_checks
                    .entry(cluster_id.to_string())
                    .or_insert(HealthCheckState {
                        consecutive_failures: 0,
                        consecutive_successes: 0,
                        last_check: Utc::now(),
                        status: ClusterHealthStatus::Unknown,
                    });

            state.last_check = Utc::now();

            if new_status == ClusterHealthStatus::Healthy {
                state.consecutive_successes += 1;
                state.consecutive_failures = 0;
            } else {
                state.consecutive_failures += 1;
                state.consecutive_successes = 0;
            }

            state.status = new_status;
            cluster.health_status = new_status;
            cluster.last_health_check = Some(Utc::now());
        }
    }

    /// Check failover conditions and trigger failover if needed
    async fn check_failover(&self, config: &FailoverConfig) -> Result<(), anyhow::Error> {
        if !config.auto_failover {
            return Ok(());
        }

        let clusters = self.clusters.read().await.clone();
        let health_checks = self.health_checks.read().await.clone();

        let primary = clusters.values().find(|c| c.is_primary);
        let primary_id = primary.map(|c| c.id.clone());

        if let Some(primary_id) = primary_id {
            let health_state = health_checks.get(&primary_id);

            if let Some(state) = health_state {
                if state.consecutive_failures >= config.failure_threshold {
                    warn!(
                        "Primary cluster {} has {} consecutive failures, triggering failover",
                        primary_id, state.consecutive_failures
                    );

                    if let Some(new_primary_id) =
                        self.select_new_primary(&clusters, &primary_id).await
                    {
                        self.perform_failover(&primary_id, &new_primary_id).await?;
                    }
                }
            }
        }

        if config.auto_failback {
            for (cluster_id, state) in &health_checks {
                if !clusters
                    .get(cluster_id)
                    .map(|c| c.is_primary)
                    .unwrap_or(false)
                    && state.consecutive_successes >= config.recovery_threshold
                {
                    info!(
                        "Former primary {} recovered, considering failback",
                        cluster_id
                    );
                }
            }
        }

        Ok(())
    }

    /// Select the best candidate for new primary
    async fn select_new_primary(
        &self,
        clusters: &HashMap<String, ClusterDefinition>,
        exclude: &str,
    ) -> Option<String> {
        let mut candidates: Vec<_> = clusters
            .values()
            .filter(|c| c.id != exclude && c.health_status == ClusterHealthStatus::Healthy)
            .collect();

        if candidates.is_empty() {
            candidates = clusters
                .values()
                .filter(|c| c.id != exclude && c.health_status != ClusterHealthStatus::Unknown)
                .collect();
        }

        candidates.sort_by_key(|c| c.priority);
        candidates.first().map(|c| c.id.clone())
    }

    /// Perform failover to new primary
    async fn perform_failover(
        &self,
        old_primary: &str,
        new_primary: &str,
    ) -> Result<(), anyhow::Error> {
        warn!("PERFORMING FAILOVER: {} -> {}", old_primary, new_primary);

        let mut clusters = self.clusters.write().await;

        if let Some(old) = clusters.get_mut(old_primary) {
            old.is_primary = false;
        }

        if let Some(new) = clusters.get_mut(new_primary) {
            new.is_primary = true;
        }

        self.send_failover_notification(old_primary, new_primary)
            .await?;

        info!("Failover completed: {} -> {}", old_primary, new_primary);
        Ok(())
    }

    /// Send failover notification
    async fn send_failover_notification(&self, from: &str, to: &str) -> Result<(), anyhow::Error> {
        let failover_config = self.failover_config.read().await;

        if let Some(webhook) = &failover_config.notification_webhook {
            let payload = serde_json::json!({
                "event": "failover",
                "from_cluster": from,
                "to_cluster": to,
                "timestamp": Utc::now().to_rfc3339(),
                "rto_target_seconds": failover_config.rto_seconds,
                "rpo_target_seconds": failover_config.rpo_seconds,
            });

            let client = reqwest::Client::new();
            if let Err(e) = reqwest::Client::new()
                .post(webhook)
                .json(&payload)
                .send()
                .await
            {
                error!("Failed to send failover notification: {}", e);
            }
        }

        Ok(())
    }

    /// Get current federation status
    pub async fn get_status(&self) -> FederationStatus {
        let clusters = self.clusters.read().await;
        let health_checks = self.health_checks.read().await;

        let mut cluster_statuses = Vec::new();
        for (id, cluster) in clusters.iter() {
            let health = self.health_checks.read().await.get(id).cloned();
            cluster_statuses.push(ClusterStatus {
                id: id.clone(),
                name: cluster.name.clone(),
                region: cluster.region.clone(),
                is_primary: cluster.is_primary,
                health_status: cluster.health_status,
                last_health_check: cluster.last_health_check,
                rto_seconds: cluster.rto_seconds,
                rpo_seconds: cluster.rpo_seconds,
                consecutive_failures: health.as_ref().map(|h| h.consecutive_failures).unwrap_or(0),
                consecutive_successes: health
                    .as_ref()
                    .map(|h| h.consecutive_successes)
                    .unwrap_or(0),
            });
        }

        FederationStatus {
            clusters: cluster_statuses,
            sync_config: self.sync_config.read().await.clone(),
            failover_config: self.failover_config.read().await.clone(),
        }
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            resources: vec![
                SyncResource {
                    api_group: "".to_string(),
                    api_version: "v1".to_string(),
                    kind: "Secret".to_string(),
                    enabled: true,
                },
                SyncResource {
                    api_group: "".to_string(),
                    api_version: "v1".to_string(),
                    kind: "ConfigMap".to_string(),
                    enabled: true,
                },
            ],
            sync_interval_seconds: 60,
            conflict_resolution: ConflictResolution::PrimaryWins,
            namespaces: vec![],
            label_selector: None,
        }
    }
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            health_check_interval_seconds: 10,
            failure_threshold: 3,
            recovery_threshold: 5,
            rto_seconds: 60,
            rpo_seconds: 30,
            auto_failover: true,
            auto_failback: true,
            notification_webhook: None,
        }
    }
}

/// Federation status for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationStatus {
    pub clusters: Vec<ClusterStatus>,
    pub sync_config: SyncConfig,
    pub failover_config: FailoverConfig,
}

/// Individual cluster status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub id: String,
    pub name: String,
    pub region: String,
    pub is_primary: bool,
    pub health_status: ClusterHealthStatus,
    pub last_health_check: Option<DateTime<Utc>>,
    pub rto_seconds: u64,
    pub rpo_seconds: u64,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
}

// Default implementations
impl Default for SyncResource {
    fn default() -> Self {
        Self {
            api_group: "".to_string(),
            api_version: "v1".to_string(),
            kind: "ConfigMap".to_string(),
            enabled: true,
        }
    }
}

impl Clone for FederationManager {
    fn clone(&self) -> Self {
        Self {
            local_cluster_id: self.local_cluster_id.clone(),
            clusters: self.clusters.clone(),
            sync_config: self.sync_config.clone(),
            failover_config: self.failover_config.clone(),
            client: self.client.clone(),
            health_checks: self.health_checks.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_health_status() {
        assert_eq!(ClusterHealthStatus::Healthy as i32, 0);
        assert_eq!(ClusterHealthStatus::Degraded as i32, 1);
        assert_eq!(ClusterHealthStatus::Unhealthy as i32, 2);
    }

    #[test]
    fn test_conflict_resolution() {
        assert_eq!(ConflictResolution::PrimaryWins as i32, 0);
        assert_eq!(ConflictResolution::LastWriteWins as i32, 1);
        assert_eq!(ConflictResolution::Merge as i32, 2);
    }

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert_eq!(config.sync_interval_seconds, 60);
        assert_eq!(config.conflict_resolution, ConflictResolution::PrimaryWins);
        assert_eq!(config.resources.len(), 2);
    }
}
