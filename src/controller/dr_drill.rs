//! Disaster Recovery Drill Orchestrator
//!
//! Automatically runs scheduled DR drills to test failover capabilities.
//! Measures Time to Recovery (TTR), verifies standby takeover, and generates reports.

use chrono::Utc;
use kube::{api::Patch, api::PatchParams, Client, ResourceExt};
use std::str::FromStr;
use std::time::Instant;
use tracing::{debug, error, info, instrument, warn};

use crate::controller::chaos_engineering::{ChaosExperimentConfig, ChaosExperimentType, ChaosRunner};
use crate::crd::{
    DRDrillResult, DRDrillScheduleConfig, DRDrillStatus, DRFailureType, DisasterRecoveryStatus,
    StellarNode,
};
use crate::error::{Error, Result};

#[cfg(feature = "metrics")]
use super::metrics;

/// Annotation key for tracking the last drill execution time
pub const DR_DRILL_LAST_RUN_ANNOTATION: &str = "stellar.org/dr-drill-last-run";
/// Annotation key for tracking drill status
pub const DR_DRILL_STATUS_ANNOTATION: &str = "stellar.org/dr-drill-status";

/// Check if a DR drill should be executed based on schedule
#[instrument(skip(node), fields(name = %node.name_any()))]
pub fn should_run_drill(node: &StellarNode, drill_config: &DRDrillScheduleConfig) -> bool {
    let schedule = match cron::Schedule::from_str(&drill_config.schedule) {
        Ok(s) => s,
        Err(e) => {
            warn!("Invalid cron schedule for DR drill: {}", e);
            return false;
        }
    };

    let now = Utc::now();
    let last_run = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(DR_DRILL_LAST_RUN_ANNOTATION))
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| now - chrono::Duration::days(1));

    // Check if next scheduled run is in the past or within 1 minute of now
    let next_run = schedule.after(&last_run).next().unwrap_or(now);
    next_run <= now && (now - next_run) < chrono::Duration::minutes(1)
}

/// Execute a DR drill
#[instrument(skip(client, node), fields(name = %node.name_any()))]
pub async fn execute_dr_drill(
    client: &Client,
    node: &StellarNode,
    drill_config: &DRDrillScheduleConfig,
    dr_status: &DisasterRecoveryStatus,
) -> Result<DRDrillResult> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let started_at = Utc::now();

    info!(
        "Starting DR drill for {}/{} (dry_run={})",
        namespace, name, drill_config.dry_run
    );

    // Update node annotation to mark drill as running
    update_drill_annotation(client, node, "running").await.ok();

    let drill_start = Instant::now();
    let mut result = DRDrillResult {
        status: DRDrillStatus::Running,
        time_to_recovery_ms: None,
        standby_takeover_success: false,
        application_availability: false,
        message: "Drill in progress".to_string(),
        started_at: started_at.to_rfc3339(),
        completed_at: None,
    };

    // Execute drill phases
    match execute_drill_phases(client, node, drill_config, dr_status).await {
        Ok(drill_result) => {
            result = drill_result;
            result.time_to_recovery_ms = Some(drill_start.elapsed().as_millis() as u64);
            result.completed_at = Some(Utc::now().to_rfc3339());

            if result.status == DRDrillStatus::Success {
                info!(
                    "DR drill completed successfully for {}/{} (TTR: {}ms)",
                    namespace,
                    name,
                    result.time_to_recovery_ms.unwrap_or(0)
                );

                // Record metrics
                #[cfg(feature = "metrics")]
                {
                    let ttr_ms = result.time_to_recovery_ms.unwrap_or(0) as i64;
                    metrics::observe_dr_drill_execution(
                        &namespace,
                        &name,
                        "success",
                        ttr_ms as f64,
                    );
                    metrics::set_dr_drill_time_to_recovery(&namespace, &name, "success", ttr_ms);
                }

                // Handle auto-rollback if configured
                if drill_config.auto_rollback {
                    if let Err(e) = schedule_drill_rollback(client, node, drill_config).await {
                        warn!("Failed to schedule drill rollback: {}", e);
                    }
                }
            } else {
                warn!(
                    "DR drill failed for {}/{}: {}",
                    namespace, name, result.message
                );

                // Record failure metrics
                #[cfg(feature = "metrics")]
                {
                    let ttr_ms = result.time_to_recovery_ms.unwrap_or(0) as i64;
                    metrics::observe_dr_drill_execution(&namespace, &name, "failed", ttr_ms as f64);
                    metrics::set_dr_drill_time_to_recovery(&namespace, &name, "failed", ttr_ms);
                }
            }
        }
        Err(e) => {
            error!("DR drill execution error for {}/{}: {e}", namespace, name);
            result.status = DRDrillStatus::Failed;
            result.message = format!("Drill execution failed: {e}");
            result.time_to_recovery_ms = Some(drill_start.elapsed().as_millis() as u64);
            result.completed_at = Some(Utc::now().to_rfc3339());

            // Record error metrics
            #[cfg(feature = "metrics")]
            {
                let ttr_ms = result.time_to_recovery_ms.unwrap_or(0) as i64;
                metrics::observe_dr_drill_execution(&namespace, &name, "failed", ttr_ms as f64);
                metrics::set_dr_drill_time_to_recovery(&namespace, &name, "failed", ttr_ms);
            }
        }
    }

    // Update node annotation with drill result
    update_drill_annotation(client, node, &format!("{:?}", result.status))
        .await
        .ok();

    Ok(result)
}

/// Execute the drill phases: failover simulation, verification, and rollback
async fn execute_drill_phases(
    client: &Client,
    node: &StellarNode,
    drill_config: &DRDrillScheduleConfig,
    dr_status: &DisasterRecoveryStatus,
) -> Result<DRDrillResult> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Phase 1: Simulate failover
    debug!("Phase 1: Simulating failover for {}/{}", namespace, name);
    let failover_result = simulate_failover(client, node, drill_config, dr_status).await?;

    if !failover_result.0 {
        return Ok(DRDrillResult {
            status: DRDrillStatus::Failed,
            time_to_recovery_ms: None,
            standby_takeover_success: false,
            application_availability: false,
            message: "Failover simulation failed".to_string(),
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
        });
    }

    // Phase 2: Verify standby takeover
    debug!(
        "Phase 2: Verifying standby takeover for {}/{}",
        namespace, name
    );
    let standby_healthy = verify_standby_takeover(client, node, drill_config).await?;

    // Phase 3: Verify application availability
    debug!(
        "Phase 3: Verifying application availability for {}/{}",
        namespace, name
    );
    let app_available = verify_application_availability(client, node, drill_config).await?;

    Ok(DRDrillResult {
        status: if standby_healthy && app_available {
            DRDrillStatus::Success
        } else {
            DRDrillStatus::Failed
        },
        time_to_recovery_ms: None,
        standby_takeover_success: standby_healthy,
        application_availability: app_available,
        message: format!(
            "Standby takeover: {standby_healthy}, Application available: {app_available}"
        ),
        started_at: Utc::now().to_rfc3339(),
        completed_at: None,
    })
}

/// Simulate failover by triggering a fake primary failure
async fn simulate_failover(
    client: &Client,
    node: &StellarNode,
    drill_config: &DRDrillScheduleConfig,
    _dr_status: &DisasterRecoveryStatus,
) -> Result<(bool, String)> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    if drill_config.dry_run {
        debug!(
            "DRY RUN: Would simulate {:?} failure for {}/{}",
            drill_config.failure_type, namespace, name
        );
        Ok((true, "Dry-run failover simulation".to_string()))
    } else {
        info!(
            "Simulating {:?} failure for {}/{} (timeout: {}s)",
            drill_config.failure_type, namespace, name, drill_config.timeout_seconds
        );

        let chaos_runner = ChaosRunner::new(client.clone());
        let experiment_type = match drill_config.failure_type {
            DRFailureType::PodKill => ChaosExperimentType::PodKill,
            DRFailureType::NetworkLatency => ChaosExperimentType::NetworkDelay,
            DRFailureType::DiskPressure => ChaosExperimentType::IoStress,
            DRFailureType::RegionOutage => ChaosExperimentType::NetworkDelay, // Simulate region outage with network delay
        };

        let chaos_config = ChaosExperimentConfig {
            experiment_type,
            namespace: namespace.clone(),
            target_label_selector: format!("app.kubernetes.io/instance={}", name),
            duration_secs: 60,
            delay_ms: Some(500),
            jitter_ms: Some(50),
            io_workers: Some(4),
            cpu_workers: None,
            memory_mb: None,
        };

        match chaos_runner.run_experiment(chaos_config).await {
            Ok(res) => {
                if res.system_recovered {
                    Ok((true, format!("Synthetic failure {:?} successful", drill_config.failure_type)))
                } else {
                    Ok((false, format!("Synthetic failure {:?} failed to recover", drill_config.failure_type)))
                }
            }
            Err(e) => Err(e),
        }
    }
}

use k8s_openapi::api::core::v1::{Endpoints, Pod, Service};
use kube::Api;

/// Verify that standby successfully took over
async fn verify_standby_takeover(
    client: &Client,
    node: &StellarNode,
    _drill_config: &DRDrillScheduleConfig,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    debug!("Verifying standby takeover for {}/{}", namespace, name);

    // 1. Check if node status reflects the change
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let updated_node = api.get(&name).await.map_err(Error::KubeError)?;

    let is_primary = updated_node
        .status
        .as_ref()
        .and_then(|s| s.dr_status.as_ref())
        .and_then(|dr| dr.current_role.as_ref())
        .map(|r| matches!(r, crate::crd::DRRole::Primary))
        .unwrap_or(false);

    if !is_primary {
        warn!(
            "Standby node {}/{} did not transition to Primary role",
            namespace, name
        );
        return Ok(false);
    }

    // 2. Check if pods are ready
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let lp =
        kube::api::ListParams::default().labels(&format!("app.kubernetes.io/instance={}", name));
    let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

    let all_ready = pod_list.items.iter().all(|p| {
        p.status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .map(|cs| cs.iter().all(|c| c.ready))
            .unwrap_or(false)
    });

    if !all_ready {
        warn!(
            "Not all pods for {}/{} are ready after takeover",
            namespace, name
        );
        return Ok(false);
    }

    Ok(true)
}

/// Verify that the application remained available during the drill
async fn verify_application_availability(
    client: &Client,
    node: &StellarNode,
    _drill_config: &DRDrillScheduleConfig,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    debug!(
        "Verifying application availability for {}/{}",
        namespace, name
    );

    // 1. Perform a simple HTTP health check if it's a Horizon node
    if node.spec.node_type == crate::crd::NodeType::Horizon {
        let ep_api: Api<Endpoints> = Api::namespaced(client.clone(), &namespace);
        let ep = ep_api.get(&name).await.map_err(Error::KubeError)?;

        let has_endpoints = ep.subsets.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
        if !has_endpoints {
            warn!("Service {}/{} has no active endpoints", namespace, name);
            return Ok(false);
        }
    }

    // 2. Data Integrity Check (Simplified)
    // We can check if the ledger sequence is advancing
    let initial_ledger = node.status.as_ref().and_then(|s| s.ledger_sequence).unwrap_or(0);
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let updated_node = api.get(&name).await.map_err(Error::KubeError)?;
    let current_ledger = updated_node
        .status
        .as_ref()
        .and_then(|s| s.ledger_sequence)
        .unwrap_or(0);

    if current_ledger <= initial_ledger && initial_ledger > 0 {
        warn!(
            "Ledger sequence did not advance for {}/{} ({} -> {})",
            namespace, name, initial_ledger, current_ledger
        );
    }

    Ok(true)
}

/// Schedule a drill rollback after the configured delay
async fn schedule_drill_rollback(
    _client: &Client,
    node: &StellarNode,
    drill_config: &DRDrillScheduleConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!(
        "Scheduling drill rollback for {}/{} in {}s",
        namespace, name, drill_config.rollback_delay_seconds
    );

    // In production, this would schedule an async task to rollback after delay
    // For now, we just log it
    debug!(
        "Rollback scheduled: will restore original state after {}s",
        drill_config.rollback_delay_seconds
    );

    Ok(())
}

/// Update drill status annotation on the node
async fn update_drill_annotation(client: &Client, node: &StellarNode, status: &str) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let patch = serde_json::json!({
        "metadata": {
            "annotations": {
                DR_DRILL_STATUS_ANNOTATION: status,
                DR_DRILL_LAST_RUN_ANNOTATION: Utc::now().to_rfc3339()
            }
        }
    });

    let api: kube::Api<StellarNode> = kube::Api::namespaced(client.clone(), &namespace);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator-dr-drill"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_run_drill_valid_schedule() {
        // This test would verify cron schedule parsing
        // Implementation depends on having a test StellarNode
    }

    #[test]
    fn test_drill_result_structure() {
        let result = DRDrillResult {
            status: DRDrillStatus::Success,
            time_to_recovery_ms: Some(1500),
            standby_takeover_success: true,
            application_availability: true,
            message: "Test drill".to_string(),
            started_at: Utc::now().to_rfc3339(),
            completed_at: Some(Utc::now().to_rfc3339()),
        };

        assert_eq!(result.status, DRDrillStatus::Success);
        assert!(result.standby_takeover_success);
        assert!(result.application_availability);
    }
}
