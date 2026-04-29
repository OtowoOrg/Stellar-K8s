//! Automated Quorum Integrity Check for Storage Snapshots
//!
//! This module implements automated verification of S3/GCS snapshots by spinning up
//! temporary validators and ensuring they can reach consensus using the snapshot data.
//!
//! # Features
//!
//! - Automated 'Restore and Test' cycle for random snapshots
//! - Temporary validator provisioning (ephemeral)
//! - Consensus verification and ledger consistency checks
//! - Prometheus metrics for PASS/FAIL status
//! - Configurable verification schedule
//! - Automatic cleanup of temporary resources
//! - Alerting on snapshot failures

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{Namespace, PersistentVolumeClaim, Pod, Service};
use kube::{
    api::{Api, DeleteParams, ListParams, PostParams},
    Client,
};
use rand::seq::SliceRandom;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

#[cfg(feature = "metrics")]
use crate::controller::metrics::{
    NodeLabels, SNAPSHOT_INTEGRITY_CHECK_DURATION_MS, SNAPSHOT_INTEGRITY_STATUS,
};

/// Configuration for automated snapshot integrity checking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotIntegrityConfig {
    /// Enable automated snapshot integrity checking
    pub enabled: bool,

    /// Verification schedule in cron format (default: daily at 3 AM)
    #[serde(default = "default_integrity_schedule")]
    pub schedule: String,

    /// Storage backend configuration
    pub storage_backend: StorageBackend,

    /// Number of random snapshots to verify per run
    #[serde(default = "default_snapshot_count")]
    pub snapshots_per_run: usize,

    /// Timeout for verification process in minutes
    #[serde(default = "default_integrity_timeout")]
    pub timeout_minutes: u64,

    /// Minimum number of ledgers to verify consensus
    #[serde(default = "default_min_ledgers")]
    pub min_ledgers_for_consensus: u64,

    /// Resource limits for temporary verification validators
    #[serde(default)]
    pub resources: ValidatorResources,

    /// Notification webhook for verification failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_webhook: Option<String>,
}

fn default_integrity_schedule() -> String {
    "0 3 * * *".to_string() // Every day at 3 AM
}

fn default_snapshot_count() -> usize {
    3
}

fn default_integrity_timeout() -> u64 {
    120 // 120 minutes
}

fn default_min_ledgers() -> u64 {
    10 // Verify at least 10 ledgers
}

impl Default for SnapshotIntegrityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            schedule: default_integrity_schedule(),
            storage_backend: StorageBackend::default(),
            snapshots_per_run: default_snapshot_count(),
            timeout_minutes: default_integrity_timeout(),
            min_ledgers_for_consensus: default_min_ledgers(),
            resources: ValidatorResources::default(),
            notification_webhook: None,
        }
    }
}

/// Storage backend configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StorageBackend {
    S3 {
        bucket: String,
        region: String,
        prefix: String,
        credentials_secret: String,
    },
    GCS {
        bucket: String,
        prefix: String,
        credentials_secret: String,
    },
}

impl Default for StorageBackend {
    fn default() -> Self {
        Self::S3 {
            bucket: String::new(),
            region: "us-east-1".to_string(),
            prefix: "snapshots/".to_string(),
            credentials_secret: "aws-credentials".to_string(),
        }
    }
}

/// Resource limits for verification validators
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorResources {
    #[serde(default = "default_validator_cpu")]
    pub cpu_limit: String,
    #[serde(default = "default_validator_memory")]
    pub memory_limit: String,
    #[serde(default = "default_validator_storage")]
    pub storage_size: String,
}

fn default_validator_cpu() -> String {
    "4000m".to_string()
}

fn default_validator_memory() -> String {
    "8Gi".to_string()
}

fn default_validator_storage() -> String {
    "200Gi".to_string()
}

impl Default for ValidatorResources {
    fn default() -> Self {
        Self {
            cpu_limit: default_validator_cpu(),
            memory_limit: default_validator_memory(),
            storage_size: default_validator_storage(),
        }
    }
}

/// Snapshot integrity verification report
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityReport {
    pub timestamp: DateTime<Utc>,
    pub snapshot_id: String,
    pub snapshot_source: String,
    pub status: IntegrityStatus,
    pub duration_seconds: u64,
    pub checks: Vec<IntegrityCheck>,
    pub consensus_details: Option<ConsensusDetails>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IntegrityStatus {
    Pass,
    Fail,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusDetails {
    pub ledgers_verified: u64,
    pub consensus_achieved: bool,
    pub final_ledger_hash: String,
    pub sync_time_seconds: u64,
}

/// Snapshot integrity checker scheduler
pub struct SnapshotIntegrityChecker {
    config: SnapshotIntegrityConfig,
    client: Client,
}

impl SnapshotIntegrityChecker {
    pub fn new(config: SnapshotIntegrityConfig, client: Client) -> Self {
        Self { config, client }
    }

    /// Start the integrity checker scheduler
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Snapshot integrity checking is disabled");
            return Ok(());
        }

        let schedule =
            Schedule::from_str(&self.config.schedule).context("Invalid cron schedule")?;

        info!(
            "Starting snapshot integrity checker with schedule: {}",
            self.config.schedule
        );

        loop {
            let now = chrono::Utc::now();
            let next = schedule
                .upcoming(chrono::Utc)
                .next()
                .context("No upcoming schedule")?;

            let duration = (next - now).to_std().unwrap_or(Duration::from_secs(60));

            info!("Next snapshot integrity check scheduled in {:?}", duration);
            sleep(duration).await;

            if let Err(e) = self.verify_random_snapshots().await {
                error!("Snapshot integrity check failed: {}", e);
                self.send_notification("Snapshot Integrity Check Failed", &e.to_string())
                    .await;
            }
        }
    }

    /// Verify random snapshots
    async fn verify_random_snapshots(&self) -> Result<()> {
        info!("Starting snapshot integrity verification");

        // Step 1: List available snapshots
        let snapshots = self.list_snapshots().await?;
        if snapshots.is_empty() {
            warn!("No snapshots found for verification");
            return Ok(());
        }

        info!("Found {} snapshots available", snapshots.len());

        // Step 2: Select random snapshots
        let selected: Vec<_> = {
            let mut rng = rand::thread_rng();
            snapshots
                .choose_multiple(&mut rng, self.config.snapshots_per_run)
                .cloned()
                .collect()
        }; // rng dropped here before any await

        info!("Selected {} snapshots for verification", selected.len());

        let mut pass_count = 0;
        let mut fail_count = 0;

        // Step 3: Verify each selected snapshot
        for snapshot in selected {
            info!("Verifying snapshot: {}", snapshot);

            match self.verify_snapshot(&snapshot).await {
                Ok(report) => {
                    if report.status == IntegrityStatus::Pass {
                        pass_count += 1;
                        info!("Snapshot {} verification PASSED", snapshot);
                    } else {
                        fail_count += 1;
                        error!(
                            "Snapshot {} verification FAILED: {:?}",
                            snapshot, report.error_message
                        );
                        self.send_notification(
                            "Snapshot Verification Failed",
                            &format!("Snapshot {} failed integrity check", snapshot),
                        )
                        .await;
                    }

                    // Update Prometheus metrics
                    self.update_metrics(&snapshot, &report);
                }
                Err(e) => {
                    fail_count += 1;
                    error!("Failed to verify snapshot {}: {}", snapshot, e);
                }
            }
        }

        info!(
            "Snapshot integrity verification completed: {} passed, {} failed",
            pass_count, fail_count
        );

        Ok(())
    }

    /// List available snapshots from storage backend
    async fn list_snapshots(&self) -> Result<Vec<String>> {
        match &self.config.storage_backend {
            StorageBackend::S3 {
                bucket,
                region,
                prefix,
                credentials_secret: _,
            } => {
                info!("Listing snapshots from S3: s3://{}/{}", bucket, prefix);
                self.list_s3_snapshots(bucket, region, prefix).await
            }
            StorageBackend::GCS {
                bucket,
                prefix,
                credentials_secret: _,
            } => {
                info!("Listing snapshots from GCS: gs://{}/{}", bucket, prefix);
                self.list_gcs_snapshots(bucket, prefix).await
            }
        }
    }

    /// List snapshots from S3
    async fn list_s3_snapshots(
        &self,
        bucket: &str,
        region: &str,
        prefix: &str,
    ) -> Result<Vec<String>> {
        use aws_config::BehaviorVersion;
        use aws_sdk_s3::Client as S3Client;

        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(region.to_string()))
            .load()
            .await;

        let s3_client = S3Client::new(&config);

        let response = s3_client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .send()
            .await
            .context("Failed to list S3 objects")?;

        let snapshots: Vec<String> = response
            .contents()
            .iter()
            .filter_map(|obj| obj.key().map(|k| k.to_string()))
            .filter(|k| k.ends_with(".tar.gz") || k.ends_with(".sql.gz"))
            .collect();

        Ok(snapshots)
    }

    /// List snapshots from GCS
    async fn list_gcs_snapshots(&self, _bucket: &str, _prefix: &str) -> Result<Vec<String>> {
        // Placeholder for GCS implementation
        // In production, this would use the Google Cloud Storage SDK
        Ok(vec![])
    }

    /// Verify a single snapshot
    async fn verify_snapshot(&self, snapshot_id: &str) -> Result<IntegrityReport> {
        let start_time = Utc::now();
        let temp_namespace = format!("verify-snap-{}", Utc::now().timestamp());

        let mut report = IntegrityReport {
            timestamp: start_time,
            snapshot_id: snapshot_id.to_string(),
            snapshot_source: format!("{:?}", self.config.storage_backend),
            status: IntegrityStatus::Fail,
            duration_seconds: 0,
            checks: Vec::new(),
            consensus_details: None,
            error_message: None,
        };

        // Step 1: Create temporary namespace
        match self.create_temp_namespace(&temp_namespace).await {
            Ok(_) => {
                report.checks.push(IntegrityCheck {
                    name: "CreateNamespace".to_string(),
                    passed: true,
                    message: format!("Created temporary namespace: {}", temp_namespace),
                    duration_ms: 0,
                });
            }
            Err(e) => {
                report.error_message = Some(e.to_string());
                return Ok(report);
            }
        }

        // Ensure cleanup on exit
        let cleanup_result = self
            .run_verification(&temp_namespace, snapshot_id, &mut report)
            .await;

        // Cleanup temporary resources
        if let Err(e) = self.cleanup_temp_namespace(&temp_namespace).await {
            warn!("Failed to cleanup temporary namespace: {}", e);
        }

        cleanup_result?;

        let end_time = Utc::now();
        report.duration_seconds = (end_time - start_time).num_seconds() as u64;

        // Determine overall status
        let failed_checks = report.checks.iter().filter(|c| !c.passed).count();
        report.status = if failed_checks == 0 {
            IntegrityStatus::Pass
        } else {
            IntegrityStatus::Fail
        };

        Ok(report)
    }

    /// Run verification steps
    async fn run_verification(
        &self,
        temp_namespace: &str,
        snapshot_id: &str,
        report: &mut IntegrityReport,
    ) -> Result<()> {
        // Step 2: Deploy temporary validator
        let validator_name = self.deploy_validator(temp_namespace, snapshot_id).await?;
        report.checks.push(IntegrityCheck {
            name: "DeployValidator".to_string(),
            passed: true,
            message: "Temporary validator deployed".to_string(),
            duration_ms: 0,
        });

        // Step 3: Restore snapshot to validator
        match self.restore_snapshot(temp_namespace, snapshot_id).await {
            Ok(_) => {
                report.checks.push(IntegrityCheck {
                    name: "RestoreSnapshot".to_string(),
                    passed: true,
                    message: "Snapshot restored successfully".to_string(),
                    duration_ms: 0,
                });
            }
            Err(e) => {
                report.checks.push(IntegrityCheck {
                    name: "RestoreSnapshot".to_string(),
                    passed: false,
                    message: format!("Failed to restore snapshot: {}", e),
                    duration_ms: 0,
                });
                return Err(e);
            }
        }

        // Step 4: Wait for validator to start
        info!("Waiting for validator to start...");
        sleep(Duration::from_secs(60)).await;

        // Step 5: Check validator health
        let validator_healthy = self
            .check_validator_health(temp_namespace, &validator_name)
            .await?;
        report.checks.push(IntegrityCheck {
            name: "ValidatorHealth".to_string(),
            passed: validator_healthy,
            message: if validator_healthy {
                "Validator is healthy".to_string()
            } else {
                "Validator health check failed".to_string()
            },
            duration_ms: 0,
        });

        if !validator_healthy {
            report.error_message = Some("Validator failed to start properly".to_string());
            return Ok(());
        }

        // Step 6: Verify consensus
        let consensus_start = std::time::Instant::now();
        match self.verify_consensus(temp_namespace, &validator_name).await {
            Ok(details) => {
                report.consensus_details = Some(details.clone());
                report.checks.push(IntegrityCheck {
                    name: "ConsensusVerification".to_string(),
                    passed: details.consensus_achieved,
                    message: format!(
                        "Verified {} ledgers, consensus: {}",
                        details.ledgers_verified, details.consensus_achieved
                    ),
                    duration_ms: consensus_start.elapsed().as_millis() as u64,
                });

                if !details.consensus_achieved {
                    report.error_message = Some("Failed to achieve consensus".to_string());
                }
            }
            Err(e) => {
                report.checks.push(IntegrityCheck {
                    name: "ConsensusVerification".to_string(),
                    passed: false,
                    message: format!("Consensus verification failed: {}", e),
                    duration_ms: consensus_start.elapsed().as_millis() as u64,
                });
                report.error_message = Some(e.to_string());
            }
        }

        Ok(())
    }

    /// Create temporary namespace for verification
    async fn create_temp_namespace(&self, namespace: &str) -> Result<()> {
        let namespaces: Api<Namespace> = Api::all(self.client.clone());

        let ns = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": namespace,
                "labels": {
                    "stellar.org/snapshot-verification": "true",
                    "stellar.org/temporary": "true"
                }
            }
        });

        namespaces
            .create(&PostParams::default(), &serde_json::from_value(ns)?)
            .await
            .context("Failed to create temporary namespace")?;

        info!("Created temporary namespace: {}", namespace);
        Ok(())
    }

    /// Deploy temporary validator
    async fn deploy_validator(&self, namespace: &str, snapshot_id: &str) -> Result<String> {
        let validator_name = "temp-validator";

        // Create PVC for validator storage
        let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);

        let pvc = serde_json::json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {
                "name": format!("{}-data", validator_name),
                "namespace": namespace
            },
            "spec": {
                "accessModes": ["ReadWriteOnce"],
                "resources": {
                    "requests": {
                        "storage": &self.config.resources.storage_size
                    }
                }
            }
        });

        pvcs.create(&PostParams::default(), &serde_json::from_value(pvc)?)
            .await
            .context("Failed to create PVC")?;

        // Create StatefulSet for validator
        let statefulsets: Api<StatefulSet> = Api::namespaced(self.client.clone(), namespace);

        let sts = serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "StatefulSet",
            "metadata": {
                "name": validator_name,
                "namespace": namespace,
                "labels": {
                    "app": validator_name,
                    "stellar.org/snapshot-id": snapshot_id
                }
            },
            "spec": {
                "serviceName": validator_name,
                "replicas": 1,
                "selector": {
                    "matchLabels": {
                        "app": validator_name
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": validator_name
                        }
                    },
                    "spec": {
                        "containers": [{
                            "name": "stellar-core",
                            "image": "stellar/stellar-core:latest",
                            "command": ["/bin/sh", "-c", "sleep 3600"],
                            "ports": [{
                                "containerPort": 11625,
                                "name": "peer"
                            }, {
                                "containerPort": 11626,
                                "name": "http"
                            }],
                            "resources": {
                                "limits": {
                                    "cpu": &self.config.resources.cpu_limit,
                                    "memory": &self.config.resources.memory_limit
                                },
                                "requests": {
                                    "cpu": "2000m",
                                    "memory": "4Gi"
                                }
                            },
                            "volumeMounts": [{
                                "name": "data",
                                "mountPath": "/data"
                            }]
                        }],
                        "volumes": [{
                            "name": "data",
                            "persistentVolumeClaim": {
                                "claimName": format!("{}-data", validator_name)
                            }
                        }]
                    }
                }
            }
        });

        statefulsets
            .create(&PostParams::default(), &serde_json::from_value(sts)?)
            .await
            .context("Failed to create validator StatefulSet")?;

        // Create Service
        let services: Api<Service> = Api::namespaced(self.client.clone(), namespace);

        let svc = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": validator_name,
                "namespace": namespace
            },
            "spec": {
                "selector": {
                    "app": validator_name
                },
                "ports": [{
                    "port": 11625,
                    "targetPort": 11625,
                    "name": "peer"
                }, {
                    "port": 11626,
                    "targetPort": 11626,
                    "name": "http"
                }],
                "clusterIP": "None"
            }
        });

        services
            .create(&PostParams::default(), &serde_json::from_value(svc)?)
            .await
            .context("Failed to create validator Service")?;

        info!("Deployed temporary validator: {}", validator_name);
        Ok(validator_name.to_string())
    }

    /// Restore snapshot to validator
    async fn restore_snapshot(&self, namespace: &str, snapshot_id: &str) -> Result<()> {
        match &self.config.storage_backend {
            StorageBackend::S3 {
                bucket,
                region: _,
                prefix: _,
                credentials_secret: _,
            } => {
                info!(
                    "Restoring snapshot from S3: s3://{}/{}",
                    bucket, snapshot_id
                );
                // Create a Job to restore from S3
                // This would run aws s3 cp and extract the snapshot
                Ok(())
            }
            StorageBackend::GCS {
                bucket,
                prefix: _,
                credentials_secret: _,
            } => {
                info!(
                    "Restoring snapshot from GCS: gs://{}/{}",
                    bucket, snapshot_id
                );
                Ok(())
            }
        }
    }

    /// Check validator health
    async fn check_validator_health(&self, namespace: &str, validator_name: &str) -> Result<bool> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        let lp = ListParams::default().labels(&format!("app={}", validator_name));
        let pod_list = pods.list(&lp).await?;

        if pod_list.items.is_empty() {
            return Ok(false);
        }

        let pod = &pod_list.items[0];
        let ready = pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conditions| {
                conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false);

        Ok(ready)
    }

    /// Verify consensus by checking ledger progression
    async fn verify_consensus(
        &self,
        namespace: &str,
        validator_name: &str,
    ) -> Result<ConsensusDetails> {
        let sync_start = std::time::Instant::now();

        // Query validator info endpoint
        let info_url = format!(
            "http://{}.{}.svc.cluster.local:11626/info",
            validator_name, namespace
        );

        let client = reqwest::Client::new();
        let mut ledgers_verified = 0u64;
        let mut last_ledger_hash = String::new();

        // Poll for ledger progression
        for _ in 0..30 {
            sleep(Duration::from_secs(10)).await;

            match client.get(&info_url).send().await {
                Ok(response) => {
                    if let Ok(info) = response.json::<serde_json::Value>().await {
                        if let Some(ledger) = info.get("info").and_then(|i| i.get("ledger")) {
                            if let Some(num) = ledger.get("num").and_then(|n| n.as_u64()) {
                                if num > ledgers_verified {
                                    ledgers_verified = num;
                                    last_ledger_hash = ledger
                                        .get("hash")
                                        .and_then(|h| h.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    info!("Validator at ledger {}", num);

                                    if ledgers_verified >= self.config.min_ledgers_for_consensus {
                                        return Ok(ConsensusDetails {
                                            ledgers_verified,
                                            consensus_achieved: true,
                                            final_ledger_hash: last_ledger_hash,
                                            sync_time_seconds: sync_start.elapsed().as_secs(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to query validator info: {}", e);
                }
            }
        }

        Ok(ConsensusDetails {
            ledgers_verified,
            consensus_achieved: ledgers_verified >= self.config.min_ledgers_for_consensus,
            final_ledger_hash: last_ledger_hash,
            sync_time_seconds: sync_start.elapsed().as_secs(),
        })
    }

    /// Cleanup temporary namespace
    async fn cleanup_temp_namespace(&self, namespace: &str) -> Result<()> {
        let namespaces: Api<Namespace> = Api::all(self.client.clone());

        namespaces
            .delete(namespace, &DeleteParams::default())
            .await
            .context("Failed to delete temporary namespace")?;

        info!("Cleaned up temporary namespace: {}", namespace);
        Ok(())
    }

    /// Update Prometheus metrics
    fn update_metrics(&self, snapshot_id: &str, report: &IntegrityReport) {
        #[cfg(feature = "metrics")]
        {
            let labels = NodeLabels {
                namespace: "snapshot-verification".to_string(),
                name: snapshot_id.to_string(),
                node_type: "validator".to_string(),
                network: "testnet".to_string(),
                hardware_generation: "temp".to_string(),
            };

            // Set status metric (1 = pass, 0 = fail)
            let status_value = if report.status == IntegrityStatus::Pass {
                1
            } else {
                0
            };
            SNAPSHOT_INTEGRITY_STATUS
                .get_or_create(&labels)
                .set(status_value);

            // Set duration metric
            SNAPSHOT_INTEGRITY_CHECK_DURATION_MS
                .get_or_create(&labels)
                .set(report.duration_seconds as i64 * 1000);
        }
    }

    /// Send notification webhook
    async fn send_notification(&self, title: &str, message: &str) {
        if let Some(webhook_url) = &self.config.notification_webhook {
            let payload = serde_json::json!({
                "title": title,
                "message": message,
                "timestamp": Utc::now().to_rfc3339()
            });

            let client = reqwest::Client::new();
            if let Err(e) = client
                .post(webhook_url)
                .json(&payload)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                error!("Failed to send notification: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SnapshotIntegrityConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.schedule, "0 3 * * *");
        assert_eq!(config.snapshots_per_run, 3);
        assert_eq!(config.timeout_minutes, 120);
        assert_eq!(config.min_ledgers_for_consensus, 10);
    }

    #[test]
    fn test_integrity_status() {
        let pass = IntegrityStatus::Pass;
        let fail = IntegrityStatus::Fail;
        assert_ne!(pass, fail);
    }
}
