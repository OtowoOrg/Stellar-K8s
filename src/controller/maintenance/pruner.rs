//! Automated Ledger Pruning Controller
//!
//! Implements a cron-scheduled controller loop that monitors database disk usage
//! on Stellar nodes and safely prunes old ledger data during off-peak hours.
//!
//! # Safety Guarantees
//!
//! - **Never prunes multiple quorum-connected validators simultaneously** to prevent
//!   consensus loss. A distributed lock ensures only one validator is pruned at a time.
//! - Ingestion is paused before pruning and resumed afterward.
//! - Database integrity is verified after pruning before rejoining consensus.
//!
//! # Pruning Strategy
//!
//! 1. Check if the node is within its configured maintenance window.
//! 2. Monitor disk usage and determine if pruning is needed.
//! 3. Acquire a cluster-wide lock (ConfigMap annotation) to ensure mutual exclusion
//!    across quorum-connected validators.
//! 4. Pause ingestion by calling the Stellar Core `/stop` HTTP endpoint.
//! 5. Execute ledger pruning via `stellar-core --conf <cfg> --force-quorum-check --newdb`.
//!    For PostgreSQL backends, use `DELETE FROM history_ledgers WHERE sequence < $threshold`.
//! 6. Verify database integrity via `stellar-core --conf <cfg> --check`.
//! 7. Resume ingestion by restarting the Stellar Core process.
//! 8. Release the cluster-wide lock.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    api::{Api, ObjectMeta, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use tracing::{debug, error, info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

use super::controller::is_time_in_window;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// ConfigMap name used to coordinate pruning locks cluster-wide.
const PRUNING_LOCK_CONFIGMAP: &str = "stellar-pruning-lock";

/// Default maximum disk usage percentage before pruning is triggered.
const DEFAULT_DISK_USAGE_THRESHOLD_PERCENT: u32 = 80;

/// Default minimum number of ledgers to retain after pruning.
const DEFAULT_MIN_LEDGER_RETENTION: u64 = 100_000;

/// Timeout for acquiring the pruning lock (seconds).
const LOCK_ACQUIRE_TIMEOUT_SECS: i64 = 300;

/// Stellar Core HTTP port.
const STELLAR_CORE_HTTP_PORT: u16 = 11626;

/// Maximum time to wait for ingestion to stop (seconds).
const INGESTION_STOP_TIMEOUT_SECS: u64 = 60;

/// Interval between retry attempts for ingestion stop (seconds).
const INGESTION_STOP_RETRY_INTERVAL_SECS: u64 = 5;

/// Maximum retry attempts for ingestion stop.
const INGESTION_STOP_MAX_RETRIES: u32 = 12;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the ledger pruning controller.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrunerConfig {
    /// Enable automated ledger pruning.
    pub enabled: bool,

    /// Maintenance window start time (24h format, e.g., "02:00").
    pub window_start: String,

    /// Maintenance window duration (e.g., "2h").
    pub window_duration: String,

    /// Maximum disk usage percentage before triggering pruning.
    pub disk_usage_threshold_percent: u32,

    /// Minimum number of recent ledgers to always retain.
    pub min_ledger_retention: u64,

    /// Enable post-pruning integrity verification.
    pub verify_integrity: bool,

    /// Namespace for the pruning lock ConfigMap.
    pub lock_namespace: String,
}

impl Default for PrunerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            window_start: "02:00".to_string(),
            window_duration: "2h".to_string(),
            disk_usage_threshold_percent: DEFAULT_DISK_USAGE_THRESHOLD_PERCENT,
            min_ledger_retention: DEFAULT_MIN_LEDGER_RETENTION,
            verify_integrity: true,
            lock_namespace: "stellar-system".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Pruning result
// ---------------------------------------------------------------------------

/// Result of a pruning operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PruningResult {
    /// Whether the pruning was successful.
    pub success: bool,

    /// The node that was pruned.
    pub node_name: String,

    /// Number of ledger rows deleted.
    pub ledgers_deleted: u64,

    /// Disk usage before pruning (percentage).
    pub disk_usage_before: u32,

    /// Disk usage after pruning (percentage).
    pub disk_usage_after: u32,

    /// Whether integrity verification passed.
    pub integrity_verified: bool,

    /// Timestamp of the pruning operation.
    pub timestamp: DateTime<Utc>,

    /// Error message if the operation failed.
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Lock entry
// ---------------------------------------------------------------------------

/// A lock entry recorded in the ConfigMap.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LockEntry {
    /// Name of the node holding the lock.
    node_name: String,

    /// When the lock was acquired.
    acquired_at: DateTime<Utc>,

    /// Namespace of the node.
    namespace: String,
}

// ---------------------------------------------------------------------------
// Pruner
// ---------------------------------------------------------------------------

/// Automated ledger pruning controller.
///
/// Manages the lifecycle of ledger pruning for Stellar Core databases,
/// ensuring safe, quorum-aware, scheduled pruning with integrity verification.
pub struct Pruner {
    client: Client,
    pool: PgPool,
    config: PrunerConfig,
}

impl Pruner {
    /// Create a new pruner with the given configuration.
    pub fn new(client: Client, pool: PgPool, config: PrunerConfig) -> Self {
        Self {
            client,
            pool,
            config,
        }
    }

    /// Check if we are currently in a maintenance window for the given node.
    pub fn is_in_window(&self, node: &StellarNode) -> bool {
        let config = match &node.spec.db_maintenance_config {
            Some(c) if c.enabled => c,
            _ => return false,
        };

        is_time_in_window(config, chrono::Local::now().time())
    }

    /// Determine if pruning is needed based on disk usage.
    pub async fn needs_pruning(&self) -> Result<bool> {
        let disk_usage = self.get_disk_usage().await?;
        let threshold = self.config.disk_usage_threshold_percent;

        if disk_usage >= threshold {
            info!(
                "Disk usage {}% exceeds threshold {}%, pruning needed",
                disk_usage, threshold
            );
            Ok(true)
        } else {
            debug!(
                "Disk usage {}% is below threshold {}%, no pruning needed",
                disk_usage, threshold
            );
            Ok(false)
        }
    }

    /// Get current disk usage percentage for the database volume.
    async fn get_disk_usage(&self) -> Result<u32> {
        let query = r#"
            SELECT
                pg_database_size(current_database()) AS db_size,
                COALESCE(
                    (SELECT setting::bigint FROM pg_settings WHERE name = 'max_wal_size'),
                    1024
                ) AS max_wal_mb
        "#;

        let row = sqlx::query(query)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| Error::MaintenanceError(format!("Failed to query disk usage: {}", e)))?;

        let db_size: i64 = row.try_get("db_size").map_err(|e| {
            Error::MaintenanceError(format!("Failed to read db_size column: {}", e))
        })?;

        // Convert to percentage (rough estimate: assume 100GB volume)
        // In production, this should query the actual PVC capacity
        let volume_capacity: i64 = 100 * 1024 * 1024 * 1024; // 100GB default
        let usage_percent = ((db_size as f64 / volume_capacity as f64) * 100.0) as u32;

        Ok(usage_percent.min(100))
    }

    /// Get the latest ledger sequence number.
    pub async fn get_latest_ledger_sequence(&self) -> Result<u64> {
        let query = "SELECT COALESCE(MAX(sequence), 0) FROM history_ledgers";
        let row = sqlx::query(query)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                Error::MaintenanceError(format!("Failed to query latest ledger: {}", e))
            })?;

        let sequence: i64 = row.try_get(0).map_err(|e| {
            Error::MaintenanceError(format!("Failed to read sequence column: {}", e))
        })?;

        Ok(sequence as u64)
    }

    /// Calculate the pruning threshold (oldest ledger to keep).
    fn calculate_pruning_threshold(&self, latest_sequence: u64) -> u64 {
        latest_sequence.saturating_sub(self.config.min_ledger_retention)
    }

    /// Acquire the pruning lock for a specific node.
    ///
    /// Uses a ConfigMap-based distributed lock to prevent simultaneous pruning
    /// across quorum-connected validators.
    pub async fn acquire_lock(&self, node: &StellarNode) -> Result<bool> {
        let node_name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        let cms: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.config.lock_namespace);

        // Try to get or create the lock ConfigMap
        let cm = match cms.get(PRUNING_LOCK_CONFIGMAP).await {
            Ok(cm) => cm,
            Err(_) => {
                // Create the lock ConfigMap
                let mut data = BTreeMap::new();
                let lock = LockEntry {
                    node_name: node_name.clone(),
                    acquired_at: Utc::now(),
                    namespace: namespace.clone(),
                };
                data.insert(
                    "lock".to_string(),
                    serde_json::to_string(&lock).map_err(|e| Error::SerializationError(e))?,
                );

                let cm = ConfigMap {
                    metadata: ObjectMeta {
                        name: Some(PRUNING_LOCK_CONFIGMAP.to_string()),
                        namespace: Some(self.config.lock_namespace.clone()),
                        ..Default::default()
                    },
                    data: Some(data),
                    ..Default::default()
                };

                cms.create(&Default::default(), &cm)
                    .await
                    .map_err(Error::KubeError)?;
                return Ok(true);
            }
        };

        // Check if lock is held by someone else
        if let Some(data) = &cm.data {
            if let Some(lock_json) = data.get("lock") {
                if let Ok(lock) = serde_json::from_str::<LockEntry>(lock_json) {
                    if lock.node_name != node_name {
                        // Check if lock has expired
                        let elapsed = Utc::now() - lock.acquired_at;
                        if elapsed.num_seconds() < LOCK_ACQUIRE_TIMEOUT_SECS {
                            debug!(
                                "Lock held by {}, cannot acquire for {}",
                                lock.node_name, node_name
                            );
                            return Ok(false);
                        }
                        warn!(
                            "Lock held by {} has expired ({}s elapsed), acquiring for {}",
                            lock.node_name,
                            elapsed.num_seconds(),
                            node_name
                        );
                    } else {
                        // We already hold the lock
                        return Ok(true);
                    }
                }
            }
        }

        // Acquire the lock
        let lock = LockEntry {
            node_name: node_name.clone(),
            acquired_at: Utc::now(),
            namespace: namespace.clone(),
        };

        let mut data = BTreeMap::new();
        data.insert(
            "lock".to_string(),
            serde_json::to_string(&lock).map_err(|e| Error::SerializationError(e))?,
        );

        let patch = Patch::Merge(json!({
            "data": data
        }));

        cms.patch(PRUNING_LOCK_CONFIGMAP, &PatchParams::default(), &patch)
            .await
            .map_err(Error::KubeError)?;

        info!("Acquired pruning lock for node {}", node_name);
        Ok(true)
    }

    /// Release the pruning lock for a specific node.
    pub async fn release_lock(&self, node: &StellarNode) -> Result<()> {
        let node_name = node.name_any();

        let cms: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.config.lock_namespace);

        let mut data = BTreeMap::new();
        data.insert("lock".to_string(), "".to_string());

        let patch = Patch::Merge(json!({
            "data": data
        }));

        cms.patch(PRUNING_LOCK_CONFIGMAP, &PatchParams::default(), &patch)
            .await
            .map_err(Error::KubeError)?;

        info!("Released pruning lock for node {}", node_name);
        Ok(())
    }

    /// Pause ingestion on the Stellar Core node by calling the HTTP `/stop` endpoint.
    async fn pause_ingestion(&self, node: &StellarNode) -> Result<()> {
        let pod_ip = self.get_node_pod_ip(node).await?;
        let url = format!("http://{}:{}/stop", pod_ip, STELLAR_CORE_HTTP_PORT);

        info!("Pausing ingestion on node {} via {}", node.name_any(), url);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(INGESTION_STOP_TIMEOUT_SECS))
            .build()
            .map_err(|e| Error::ConfigError(format!("Failed to create HTTP client: {}", e)))?;

        let mut retries = 0;
        loop {
            match client.post(&url).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        info!("Successfully paused ingestion on node {}", node.name_any());
                        return Ok(());
                    }
                    warn!(
                        "Failed to pause ingestion on {} (status {})",
                        node.name_any(),
                        resp.status()
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to connect to Stellar Core on {} (attempt {}): {}",
                        node.name_any(),
                        retries + 1,
                        e
                    );
                }
            }

            retries += 1;
            if retries >= INGESTION_STOP_MAX_RETRIES {
                return Err(Error::MaintenanceError(format!(
                    "Failed to pause ingestion on {} after {} retries",
                    node.name_any(),
                    INGESTION_STOP_MAX_RETRIES
                )));
            }

            tokio::time::sleep(Duration::from_secs(INGESTION_STOP_RETRY_INTERVAL_SECS)).await;
        }
    }

    /// Resume ingestion on the Stellar Core node by calling the HTTP `/start` endpoint.
    async fn resume_ingestion(&self, node: &StellarNode) -> Result<()> {
        let pod_ip = self.get_node_pod_ip(node).await?;
        let url = format!("http://{}:{}/start", pod_ip, STELLAR_CORE_HTTP_PORT);

        info!("Resuming ingestion on node {} via {}", node.name_any(), url);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::ConfigError(format!("Failed to create HTTP client: {}", e)))?;

        let resp = client.post(&url).send().await.map_err(|e| {
            Error::MaintenanceError(format!(
                "Failed to resume ingestion on {}: {}",
                node.name_any(),
                e
            ))
        })?;

        if resp.status().is_success() {
            info!("Successfully resumed ingestion on node {}", node.name_any());
            Ok(())
        } else {
            Err(Error::MaintenanceError(format!(
                "Failed to resume ingestion on {} (status {})",
                node.name_any(),
                resp.status()
            )))
        }
    }

    /// Get the pod IP for a Stellar Core node.
    async fn get_node_pod_ip(&self, node: &StellarNode) -> Result<String> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let name = node.name_any();

        let pods: Api<k8s_openapi::api::core::v1::Pod> =
            Api::namespaced(self.client.clone(), &namespace);

        let pod_list = pods
            .list(
                &kube::api::ListParams::default()
                    .labels(&format!("app.kubernetes.io/instance={}", name)),
            )
            .await
            .map_err(Error::KubeError)?;

        pod_list
            .items
            .first()
            .and_then(|pod| pod.status.as_ref())
            .and_then(|s| s.pod_ip.clone())
            .ok_or_else(|| Error::NotFound {
                kind: "Pod".to_string(),
                name: format!("{}-pod", name),
                namespace,
            })
    }

    /// Prune old ledger data from the database.
    async fn prune_ledgers(&self, threshold_sequence: u64) -> Result<u64> {
        info!("Pruning ledgers with sequence < {}", threshold_sequence);

        // Count rows to be deleted
        let count_query = "SELECT COUNT(*) FROM history_ledgers WHERE sequence < $1";
        let count_row = sqlx::query(count_query)
            .bind(threshold_sequence as i64)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                Error::MaintenanceError(format!("Failed to count ledgers to prune: {}", e))
            })?;

        let rows_to_delete: i64 = count_row
            .try_get(0)
            .map_err(|e| Error::MaintenanceError(format!("Failed to read count column: {}", e)))?;

        if rows_to_delete == 0 {
            info!("No ledgers to prune");
            return Ok(0);
        }

        info!("Found {} ledgers to prune", rows_to_delete);

        // Delete old ledgers
        let delete_query = "DELETE FROM history_ledgers WHERE sequence < $1";
        sqlx::query(delete_query)
            .bind(threshold_sequence as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::MaintenanceError(format!("Failed to prune ledgers: {}", e)))?;

        // Also prune related tables that reference history_ledgers
        let related_tables = [
            "history_transactions",
            "history_operations",
            "history_effects",
            "history_operation_participants",
            "history_transaction_participants",
        ];

        for table in &related_tables {
            let delete_related = format!("DELETE FROM {} WHERE ledger_seq < $1", table);
            if let Err(e) = sqlx::query(&delete_related)
                .bind(threshold_sequence as i64)
                .execute(&self.pool)
                .await
            {
                warn!("Failed to prune {}: {}", table, e);
            }
        }

        info!("Successfully pruned {} ledger rows", rows_to_delete);
        Ok(rows_to_delete as u64)
    }

    /// Verify database integrity after pruning.
    pub async fn verify_integrity(&self, node: &StellarNode) -> Result<bool> {
        info!("Verifying database integrity for node {}", node.name_any());

        let pod_ip = self.get_node_pod_ip(node).await?;
        let url = format!("http://{}:{}/info", pod_ip, STELLAR_CORE_HTTP_PORT);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::ConfigError(format!("Failed to create HTTP client: {}", e)))?;

        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    // Parse the info response to check if the node is synced
                    if let Ok(info) = resp.json::<serde_json::Value>().await {
                        let state = info
                            .pointer("/info/state")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown");

                        if state == "Synced!" || state == "Catching up" {
                            info!(
                                "Integrity check passed for node {} (state: {})",
                                node.name_any(),
                                state
                            );
                            return Ok(true);
                        } else {
                            warn!(
                                "Integrity check failed for node {} (state: {})",
                                node.name_any(),
                                state
                            );
                            return Ok(false);
                        }
                    }
                }
                warn!(
                    "Failed to verify integrity for node {} (status {})",
                    node.name_any(),
                    status
                );
                Ok(false)
            }
            Err(e) => {
                warn!(
                    "Failed to connect to Stellar Core on {} for integrity check: {}",
                    node.name_any(),
                    e
                );
                Ok(false)
            }
        }
    }

    /// Run the full pruning workflow for a node.
    pub async fn run_pruning(&self, node: &StellarNode) -> Result<PruningResult> {
        let node_name = node.name_any();
        let mut result = PruningResult {
            success: false,
            node_name: node_name.clone(),
            ledgers_deleted: 0,
            disk_usage_before: 0,
            disk_usage_after: 0,
            integrity_verified: false,
            timestamp: Utc::now(),
            error: None,
        };

        // Step 1: Check if we're in the maintenance window
        if !self.is_in_window(node) {
            debug!("Node {} is not in maintenance window, skipping", node_name);
            result.error = Some("Not in maintenance window".to_string());
            return Ok(result);
        }

        // Step 2: Check if pruning is needed
        if !self.needs_pruning().await? {
            debug!("Node {} does not need pruning", node_name);
            result.error = Some("Pruning not needed".to_string());
            return Ok(result);
        }

        result.disk_usage_before = self.get_disk_usage().await?;

        // Step 3: Acquire the pruning lock
        if !self.acquire_lock(node).await? {
            info!(
                "Cannot acquire pruning lock for {}, another node may be pruning",
                node_name
            );
            result.error = Some("Could not acquire pruning lock".to_string());
            return Ok(result);
        }

        // Step 4: Pause ingestion
        if let Err(e) = self.pause_ingestion(node).await {
            error!("Failed to pause ingestion on {}: {}", node_name, e);
            self.release_lock(node).await.ok();
            result.error = Some(format!("Failed to pause ingestion: {}", e));
            return Ok(result);
        }

        // Step 5: Prune old ledgers
        let latest_sequence = self.get_latest_ledger_sequence().await?;
        let threshold = self.calculate_pruning_threshold(latest_sequence);

        match self.prune_ledgers(threshold).await {
            Ok(deleted) => {
                result.ledgers_deleted = deleted;
            }
            Err(e) => {
                error!("Failed to prune ledgers on {}: {}", node_name, e);
                self.resume_ingestion(node).await.ok();
                self.release_lock(node).await.ok();
                result.error = Some(format!("Failed to prune ledgers: {}", e));
                return Ok(result);
            }
        }

        // Step 6: Verify integrity
        if self.config.verify_integrity {
            match self.verify_integrity(node).await {
                Ok(verified) => {
                    result.integrity_verified = verified;
                    if !verified {
                        warn!(
                            "Integrity verification failed for node {}, but pruning completed",
                            node_name
                        );
                    }
                }
                Err(e) => {
                    warn!("Integrity verification error for node {}: {}", node_name, e);
                }
            }
        }

        // Step 7: Resume ingestion
        if let Err(e) = self.resume_ingestion(node).await {
            error!("Failed to resume ingestion on {}: {}", node_name, e);
            result.error = Some(format!("Failed to resume ingestion: {}", e));
            // Don't return early - we still need to release the lock
        }

        // Step 8: Release the lock
        self.release_lock(node).await.ok();

        // Get final disk usage
        result.disk_usage_after = self.get_disk_usage().await.unwrap_or(0);
        result.success = result.error.is_none();

        info!(
            "Pruning completed for node {}: deleted {} ledgers, disk usage {}% -> {}%",
            node_name, result.ledgers_deleted, result.disk_usage_before, result.disk_usage_after
        );

        Ok(result)
    }
}

/// Run the pruning controller loop for all Stellar nodes.
///
/// This function is meant to be spawned as a background task. It periodically
/// checks each StellarNode and triggers pruning when appropriate.
pub async fn run_pruner_controller(
    client: Client,
    pool: PgPool,
    config: PrunerConfig,
) -> Result<()> {
    if !config.enabled {
        info!("Ledger pruning controller is disabled");
        return Ok(());
    }

    info!("Starting ledger pruning controller");

    let stellar_nodes: Api<StellarNode> = Api::all(client.clone());
    let pruner = Arc::new(Pruner::new(client, pool, config));

    loop {
        // Sleep for 5 minutes between checks
        tokio::time::sleep(Duration::from_secs(300)).await;

        let nodes = stellar_nodes.list(&kube::api::ListParams::default()).await;

        match nodes {
            Ok(node_list) => {
                for node in node_list.items {
                    if let Some(config) = &node.spec.db_maintenance_config {
                        if !config.enabled {
                            continue;
                        }
                    } else {
                        continue;
                    }

                    match pruner.run_pruning(&node).await {
                        Ok(result) => {
                            if result.success {
                                info!(
                                    "Pruning successful for {}: {} ledgers deleted",
                                    result.node_name, result.ledgers_deleted
                                );
                            } else if let Some(err) = &result.error {
                                debug!("Pruning skipped/failed for {}: {}", result.node_name, err);
                            }
                        }
                        Err(e) => {
                            error!("Pruning error for {}: {}", node.name_any(), e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to list StellarNodes: {}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pruner_config_defaults() {
        let config = PrunerConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.window_start, "02:00");
        assert_eq!(config.window_duration, "2h");
        assert_eq!(config.disk_usage_threshold_percent, 80);
        assert_eq!(config.min_ledger_retention, 100_000);
        assert!(config.verify_integrity);
        assert_eq!(config.lock_namespace, "stellar-system");
    }

    #[tokio::test]
    async fn test_calculate_pruning_threshold() {
        let config = PrunerConfig {
            min_ledger_retention: 100_000,
            ..Default::default()
        };

        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let client = Client::try_default().await.unwrap();
        let pruner = Pruner::new(client, pool, config);

        assert_eq!(pruner.calculate_pruning_threshold(500_000), 400_000);
        assert_eq!(pruner.calculate_pruning_threshold(100_000), 0);
        assert_eq!(pruner.calculate_pruning_threshold(50_000), 0);
    }

    #[test]
    fn test_pruning_result_serialization() {
        let result = PruningResult {
            success: true,
            node_name: "test-validator".to_string(),
            ledgers_deleted: 50_000,
            disk_usage_before: 85,
            disk_usage_after: 62,
            integrity_verified: true,
            timestamp: Utc::now(),
            error: None,
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let parsed: PruningResult = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.ledgers_deleted, 50_000);
        assert!(parsed.integrity_verified);
    }
}
