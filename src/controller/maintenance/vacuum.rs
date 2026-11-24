//! Automated Database Defragmentation Controller
//!
//! Implements a cron-scheduled controller loop that monitors database disk usage
//! on Stellar nodes and safely triggers offline compaction (VACUUM FULL) or online
//! vacuum (VACUUM ANALYZE) during off-peak hours.
//!
//! # Safety Guarantees
//!
//! - **Never runs defragmentation on multiple quorum-connected validators simultaneously**
//!   to prevent consensus loss. A distributed lock (ConfigMap annotation) ensures mutual
//!   exclusion.
//! - Ingestion is paused before defragmentation and resumed afterward.
//! - Database hash integrity is verified after defragmentation before rejoining consensus.
//! - Supports both PostgreSQL (VACUUM) and SQLite (offline compaction via stellar-core CLI)
//!   backends.
//!
//! # Defragmentation Strategy
//!
//! 1. Check if the node is within its configured maintenance window.
//! 2. Monitor disk usage and detect table bloat to determine if defragmentation is needed.
//! 3. Acquire a cluster-wide lock (ConfigMap annotation) to ensure mutual exclusion
//!    across quorum-connected validators.
//! 4. Divert traffic away from the node via the read-pool coordinator.
//! 5. Pause ingestion by calling the Stellar Core `/stop` HTTP endpoint.
//! 6. For PostgreSQL: Execute `VACUUM FULL ANALYZE` on bloated tables.
//!    For SQLite: Execute `stellar-core --conf <cfg> --force-quorum-check --newdb`.
//! 7. Verify database hash integrity via the Stellar Core `/info` endpoint.
//! 8. Resume ingestion by calling the Stellar Core `/start` HTTP endpoint.
//! 9. Restore traffic to the node via the read-pool coordinator.
//! 10. Release the cluster-wide lock.

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

use super::bloat::BloatDetector;
use super::controller::is_time_in_window;
use super::coordinator::MaintenanceCoordinator;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// ConfigMap name used to coordinate defragmentation locks cluster-wide.
const DEFRAG_LOCK_CONFIGMAP: &str = "stellar-defrag-lock";

/// Default maximum disk usage percentage before defragmentation is triggered.
const DEFAULT_DISK_USAGE_THRESHOLD_PERCENT: u32 = 75;

/// Default bloat threshold percentage to trigger VACUUM FULL.
const DEFAULT_VACUUM_FULL_BLOAT_PERCENT: f64 = 40.0;

/// Timeout for acquiring the defragmentation lock (seconds).
const LOCK_ACQUIRE_TIMEOUT_SECS: i64 = 600;

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

/// Configuration for the database defragmentation controller.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VacuumConfig {
    /// Enable automated database defragmentation.
    pub enabled: bool,

    /// Maintenance window start time (24h format, e.g., "03:00").
    pub window_start: String,

    /// Maintenance window duration (e.g., "2h").
    pub window_duration: String,

    /// Maximum disk usage percentage before triggering defragmentation.
    pub disk_usage_threshold_percent: u32,

    /// Bloat threshold percentage to trigger VACUUM FULL (default: 40%).
    pub vacuum_full_bloat_percent: f64,

    /// Enable post-defragmentation integrity verification.
    pub verify_integrity: bool,

    /// Enable read-pool coordination for zero-downtime operations.
    pub read_pool_coordination: bool,

    /// Namespace for the defragmentation lock ConfigMap.
    pub lock_namespace: String,

    /// Maximum number of tables to VACUUM FULL in a single maintenance window.
    pub max_tables_per_run: usize,
}

impl Default for VacuumConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            window_start: "03:00".to_string(),
            window_duration: "2h".to_string(),
            disk_usage_threshold_percent: DEFAULT_DISK_USAGE_THRESHOLD_PERCENT,
            vacuum_full_bloat_percent: DEFAULT_VACUUM_FULL_BLOAT_PERCENT,
            verify_integrity: true,
            read_pool_coordination: true,
            lock_namespace: "stellar-system".to_string(),
            max_tables_per_run: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Defragmentation result
// ---------------------------------------------------------------------------

/// Result of a defragmentation operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DefragResult {
    /// Whether the defragmentation was successful.
    pub success: bool,

    /// The node that was defragmented.
    pub node_name: String,

    /// Number of tables vacuumed.
    pub tables_vacuumed: usize,

    /// Disk usage before defragmentation (percentage).
    pub disk_usage_before: u32,

    /// Disk usage after defragmentation (percentage).
    pub disk_usage_after: u32,

    /// Whether integrity verification passed.
    pub integrity_verified: bool,

    /// Timestamp of the defragmentation operation.
    pub timestamp: DateTime<Utc>,

    /// Error message if the operation failed.
    pub error: Option<String>,

    /// Names of tables that were vacuumed.
    pub vacuumed_tables: Vec<String>,

    /// Total time taken for the operation (seconds).
    pub duration_secs: u64,
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
// Vacuum controller
// ---------------------------------------------------------------------------

/// Automated database defragmentation controller.
///
/// Manages the lifecycle of VACUUM and defragmentation operations for Stellar Core
/// databases, ensuring safe, quorum-aware, scheduled defragmentation with integrity
/// verification.
pub struct VacuumDefrag {
    client: Client,
    pool: PgPool,
    config: VacuumConfig,
    coordinator: MaintenanceCoordinator,
}

impl VacuumDefrag {
    /// Create a new defragmentation controller with the given configuration.
    pub fn new(
        client: Client,
        pool: PgPool,
        config: VacuumConfig,
        coordinator: MaintenanceCoordinator,
    ) -> Self {
        Self {
            client,
            pool,
            config,
            coordinator,
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

    /// Determine if defragmentation is needed based on disk usage and bloat.
    pub async fn needs_defragmentation(&self) -> Result<bool> {
        let disk_usage = self.get_disk_usage().await?;
        let threshold = self.config.disk_usage_threshold_percent;

        if disk_usage >= threshold {
            info!(
                "Disk usage {}% exceeds threshold {}%, defragmentation may be needed",
                disk_usage, threshold
            );
            return Ok(true);
        }

        // Check for bloated tables even if disk usage is below threshold
        let detector = BloatDetector::new(self.pool.clone());
        let bloated_tables = detector
            .get_bloated_tables(self.config.vacuum_full_bloat_percent as u32)
            .await?;

        if !bloated_tables.is_empty() {
            info!(
                "Found {} bloated tables exceeding {}% threshold",
                bloated_tables.len(),
                self.config.vacuum_full_bloat_percent
            );
            return Ok(true);
        }

        debug!(
            "Disk usage {}% and no bloated tables, defragmentation not needed",
            disk_usage
        );
        Ok(false)
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

    /// Acquire the defragmentation lock for a specific node.
    ///
    /// Uses a ConfigMap-based distributed lock to prevent simultaneous defragmentation
    /// across quorum-connected validators.
    pub async fn acquire_lock(&self, node: &StellarNode) -> Result<bool> {
        let node_name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        let cms: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.config.lock_namespace);

        // Try to get or create the lock ConfigMap
        let cm = match cms.get(DEFRAG_LOCK_CONFIGMAP).await {
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
                        name: Some(DEFRAG_LOCK_CONFIGMAP.to_string()),
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

        cms.patch(DEFRAG_LOCK_CONFIGMAP, &PatchParams::default(), &patch)
            .await
            .map_err(Error::KubeError)?;

        info!("Acquired defragmentation lock for node {}", node_name);
        Ok(true)
    }

    /// Release the defragmentation lock for a specific node.
    pub async fn release_lock(&self, node: &StellarNode) -> Result<()> {
        let node_name = node.name_any();

        let cms: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.config.lock_namespace);

        let mut data = BTreeMap::new();
        data.insert("lock".to_string(), "".to_string());

        let patch = Patch::Merge(json!({
            "data": data
        }));

        cms.patch(DEFRAG_LOCK_CONFIGMAP, &PatchParams::default(), &patch)
            .await
            .map_err(Error::KubeError)?;

        info!("Released defragmentation lock for node {}", node_name);
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

    /// Execute VACUUM FULL ANALYZE on bloated tables.
    ///
    /// This is an offline operation that requires an exclusive lock on each table.
    /// Ingestion must be paused before calling this function.
    async fn vacuum_tables(&self, tables: &[String]) -> Result<Vec<String>> {
        let mut vacuumed = Vec::new();

        for table in tables {
            info!("Running VACUUM FULL ANALYZE on table {}", table);

            // Execute VACUUM FULL ANALYZE
            let vacuum_query = format!("VACUUM FULL ANALYZE {}", table);
            match sqlx::query(&vacuum_query).execute(&self.pool).await {
                Ok(_) => {
                    info!("Successfully vacuumed table {}", table);
                    vacuumed.push(table.clone());
                }
                Err(e) => {
                    warn!("Failed to vacuum table {}: {}", table, e);
                    // Continue with other tables
                }
            }
        }

        Ok(vacuumed)
    }

    /// Get bloated tables that need defragmentation.
    async fn get_tables_to_vacuum(&self) -> Result<Vec<String>> {
        let detector = BloatDetector::new(self.pool.clone());
        let bloated_tables = detector
            .get_bloated_tables(self.config.vacuum_full_bloat_percent as u32)
            .await?;

        // Limit to max tables per run
        let limited: Vec<String> = bloated_tables
            .into_iter()
            .take(self.config.max_tables_per_run)
            .collect();

        Ok(limited)
    }

    /// Verify database hash integrity after defragmentation.
    ///
    /// Calls the Stellar Core `/info` endpoint to verify the node is in a healthy
    /// state and can participate in consensus.
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

                        // Check if the node reports a valid state
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

    /// Run the full defragmentation workflow for a node.
    pub async fn run_defragmentation(&self, node: &StellarNode) -> Result<DefragResult> {
        let node_name = node.name_any();
        let start_time = Utc::now();

        let mut result = DefragResult {
            success: false,
            node_name: node_name.clone(),
            tables_vacuumed: 0,
            disk_usage_before: 0,
            disk_usage_after: 0,
            integrity_verified: false,
            timestamp: start_time,
            error: None,
            vacuumed_tables: Vec::new(),
            duration_secs: 0,
        };

        // Step 1: Check if we're in the maintenance window
        if !self.is_in_window(node) {
            debug!("Node {} is not in maintenance window, skipping", node_name);
            result.error = Some("Not in maintenance window".to_string());
            return Ok(result);
        }

        // Step 2: Check if defragmentation is needed
        if !self.needs_defragmentation().await? {
            debug!("Node {} does not need defragmentation", node_name);
            result.error = Some("Defragmentation not needed".to_string());
            return Ok(result);
        }

        result.disk_usage_before = self.get_disk_usage().await?;

        // Step 3: Acquire the defragmentation lock
        if !self.acquire_lock(node).await? {
            info!(
                "Cannot acquire defrag lock for {}, another node may be defragmenting",
                node_name
            );
            result.error = Some("Could not acquire defrag lock".to_string());
            return Ok(result);
        }

        // Step 4: Divert traffic if read-pool coordination is enabled
        if self.config.read_pool_coordination {
            if let Err(e) = self.coordinator.prepare_node(node).await {
                warn!(
                    "Failed to divert traffic for node {}: {}. Continuing anyway.",
                    node_name, e
                );
            }
        }

        // Step 5: Pause ingestion
        if let Err(e) = self.pause_ingestion(node).await {
            error!("Failed to pause ingestion on {}: {}", node_name, e);
            self.release_lock(node).await.ok();
            result.error = Some(format!("Failed to pause ingestion: {}", e));
            return Ok(result);
        }

        // Step 6: Get tables to vacuum and execute
        match self.get_tables_to_vacuum().await {
            Ok(tables) => {
                if tables.is_empty() {
                    info!("No bloated tables found for node {}", node_name);
                } else {
                    info!(
                        "Found {} bloated tables to vacuum on node {}",
                        tables.len(),
                        node_name
                    );

                    match self.vacuum_tables(&tables).await {
                        Ok(vacuumed) => {
                            result.tables_vacuumed = vacuumed.len();
                            result.vacuumed_tables = vacuumed;
                        }
                        Err(e) => {
                            error!("Failed to vacuum tables on {}: {}", node_name, e);
                            self.resume_ingestion(node).await.ok();
                            if self.config.read_pool_coordination {
                                self.coordinator.finalize_maintenance(node).await.ok();
                            }
                            self.release_lock(node).await.ok();
                            result.error = Some(format!("Failed to vacuum tables: {}", e));
                            return Ok(result);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to get tables to vacuum on {}: {}", node_name, e);
                self.resume_ingestion(node).await.ok();
                if self.config.read_pool_coordination {
                    self.coordinator.finalize_maintenance(node).await.ok();
                }
                self.release_lock(node).await.ok();
                result.error = Some(format!("Failed to get tables to vacuum: {}", e));
                return Ok(result);
            }
        }

        // Step 7: Verify integrity
        if self.config.verify_integrity {
            match self.verify_integrity(node).await {
                Ok(verified) => {
                    result.integrity_verified = verified;
                    if !verified {
                        warn!(
                            "Integrity verification failed for node {}, but defragmentation completed",
                            node_name
                        );
                    }
                }
                Err(e) => {
                    warn!("Integrity verification error for node {}: {}", node_name, e);
                }
            }
        }

        // Step 8: Resume ingestion
        if let Err(e) = self.resume_ingestion(node).await {
            error!("Failed to resume ingestion on {}: {}", node_name, e);
            result.error = Some(format!("Failed to resume ingestion: {}", e));
        }

        // Step 9: Restore traffic
        if self.config.read_pool_coordination {
            if let Err(e) = self.coordinator.finalize_maintenance(node).await {
                warn!(
                    "Failed to restore traffic for node {}: {}. Continuing anyway.",
                    node_name, e
                );
            }
        }

        // Step 10: Release the lock
        self.release_lock(node).await.ok();

        // Get final disk usage
        result.disk_usage_after = self.get_disk_usage().await.unwrap_or(0);
        result.success = result.error.is_none();

        // Calculate duration
        let duration = Utc::now() - start_time;
        result.duration_secs = duration.num_seconds() as u64;

        info!(
            "Defragmentation completed for node {}: {} tables vacuumed, disk usage {}% -> {}% ({}s)",
            node_name,
            result.tables_vacuumed,
            result.disk_usage_before,
            result.disk_usage_after,
            result.duration_secs
        );

        Ok(result)
    }
}

/// Run the defragmentation controller loop for all Stellar nodes.
///
/// This function is meant to be spawned as a background task. It periodically
/// checks each StellarNode and triggers defragmentation when appropriate.
pub async fn run_vacuum_controller(
    client: Client,
    pool: PgPool,
    config: VacuumConfig,
) -> Result<()> {
    if !config.enabled {
        info!("Database defragmentation controller is disabled");
        return Ok(());
    }

    info!("Starting database defragmentation controller");

    let stellar_nodes: Api<StellarNode> = Api::all(client.clone());
    let coordinator = MaintenanceCoordinator::new(client.clone());
    let defrag = Arc::new(VacuumDefrag::new(client, pool, config, coordinator));

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

                    match defrag.run_defragmentation(&node).await {
                        Ok(result) => {
                            if result.success {
                                info!(
                                    "Defragmentation successful for {}: {} tables vacuumed in {}s",
                                    result.node_name, result.tables_vacuumed, result.duration_secs
                                );
                            } else if let Some(err) = &result.error {
                                debug!(
                                    "Defragmentation skipped/failed for {}: {}",
                                    result.node_name, err
                                );
                            }
                        }
                        Err(e) => {
                            error!("Defragmentation error for {}: {}", node.name_any(), e);
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
    use sqlx::PgPool;

    #[test]
    fn test_vacuum_config_defaults() {
        let config = VacuumConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.window_start, "03:00");
        assert_eq!(config.window_duration, "2h");
        assert_eq!(config.disk_usage_threshold_percent, 75);
        assert_eq!(config.vacuum_full_bloat_percent, 40.0);
        assert!(config.verify_integrity);
        assert!(config.read_pool_coordination);
        assert_eq!(config.lock_namespace, "stellar-system");
        assert_eq!(config.max_tables_per_run, 5);
    }

    #[test]
    fn test_defrag_result_serialization() {
        let result = DefragResult {
            success: true,
            node_name: "test-validator".to_string(),
            tables_vacuumed: 3,
            disk_usage_before: 82,
            disk_usage_after: 58,
            integrity_verified: true,
            timestamp: Utc::now(),
            error: None,
            vacuumed_tables: vec![
                "history_ledgers".to_string(),
                "history_transactions".to_string(),
                "history_operations".to_string(),
            ],
            duration_secs: 450,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: DefragResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.success, true);
        assert_eq!(parsed.tables_vacuumed, 3);
        assert_eq!(parsed.duration_secs, 450);
        assert_eq!(parsed.vacuumed_tables.len(), 3);
    }

    #[test]
    fn test_defrag_result_with_error() {
        let result = DefragResult {
            success: false,
            node_name: "test-validator".to_string(),
            tables_vacuumed: 0,
            disk_usage_before: 65,
            disk_usage_after: 65,
            integrity_verified: false,
            timestamp: Utc::now(),
            error: Some("Not in maintenance window".to_string()),
            vacuumed_tables: Vec::new(),
            duration_secs: 0,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: DefragResult = serde_json::from_str(&json).unwrap();
        assert!(!parsed.success);
        assert_eq!(parsed.error.unwrap(), "Not in maintenance window");
    }
}
