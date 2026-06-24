//! Status and helper functions.

use super::prelude::*;

pub(crate) async fn get_ready_replicas(client: &Client, node: &StellarNode) -> Result<i32> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    match node.spec.node_type {
        NodeType::Validator => {
            // Validators use StatefulSet
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(statefulset) => {
                    let ready_replicas = statefulset
                        .status
                        .as_ref()
                        .and_then(|s| s.ready_replicas)
                        .unwrap_or(0);
                    Ok(ready_replicas)
                }
                Err(e) => {
                    warn!("Failed to get StatefulSet {}/{}: {:?}", namespace, name, e);
                    Ok(0)
                }
            }
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            // RPC nodes use Deployment
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(deployment) => {
                    let ready_replicas = deployment
                        .status
                        .as_ref()
                        .and_then(|s| s.ready_replicas)
                        .unwrap_or(0);
                    Ok(ready_replicas)
                }
                Err(e) => {
                    warn!("Failed to get Deployment {}/{}: {:?}", namespace, name, e);
                    Ok(0)
                }
            }
        }
    }
}

/// Fetch the ready replicas for the canary deployment
#[allow(dead_code)]
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn get_canary_ready_replicas(client: &Client, node: &StellarNode) -> Result<i32> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = format!("{}-canary", node.name_any());

    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    match api.get(&name).await {
        Ok(deployment) => {
            let ready_replicas = deployment
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);
            Ok(ready_replicas)
        }
        Err(_) => Ok(0),
    }
}

/// Get the current version of the stable deployment
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn get_current_deployment_version(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<String>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let names = vec![node.name_any(), format!("{}-green", node.name_any())];

    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    for name in names {
        if let Ok(deployment) = api.get(&name).await {
            let version = deployment
                .spec
                .as_ref()
                .and_then(|s| s.template.spec.as_ref())
                .and_then(|ts| ts.containers.first())
                .and_then(|c| c.image.as_ref())
                .and_then(|img| img.split(':').next_back())
                .map(|v| v.to_string());
            if version.is_some() {
                return Ok(version);
            }
        }
    }

    Ok(None)
}

/// Check health of canary pods
#[allow(dead_code)]
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn check_canary_health(
    client: &Client,
    node: &StellarNode,
) -> Result<health::HealthCheckResult> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let canary_name = format!("{}-canary", node.name_any());

    // Create a temporary node with the canary name to use the existing health check logic
    let mut canary_node = node.clone();
    canary_node.metadata.name = Some(canary_name.clone());

    // 1. Basic pod readiness check
    let readiness = health::check_node_health(client, &canary_node, None).await?;
    if !readiness.healthy {
        return Ok(readiness);
    }

    // 2. HTTP error rate check against the canary service
    let max_error_rate = node
        .spec
        .strategy
        .canary()
        .map(|c| c.max_error_rate)
        .unwrap_or(0.05);

    match measure_canary_error_rate(client, node, &namespace).await {
        Ok(error_rate) => {
            if error_rate > max_error_rate {
                return Ok(health::HealthCheckResult::unhealthy(format!(
                    "Canary error rate {:.1}% exceeds threshold {:.1}%",
                    error_rate * 100.0,
                    max_error_rate * 100.0,
                )));
            }
            Ok(health::HealthCheckResult::synced(readiness.ledger_sequence))
        }
        Err(e) => {
            // If we can't measure error rate, fall back to readiness only
            warn!(
                "Could not measure canary error rate for {}/{}: {}. Falling back to readiness.",
                namespace,
                node.name_any(),
                e
            );
            Ok(readiness)
        }
    }
}

/// Measure the 4xx/5xx error rate on the canary service by sampling its /metrics or /health.
///
/// Queries the canary pod directly and counts non-2xx responses over a short window.
/// Returns a value in [0.0, 1.0].
pub(crate) async fn measure_canary_error_rate(
    client: &Client,
    node: &StellarNode,
    namespace: &str,
) -> Result<f64> {
    use k8s_openapi::api::core::v1::Pod;
    use std::time::Duration;

    let canary_name = format!("{}-canary", node.name_any());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let lp = kube::api::ListParams::default()
        .labels(&format!("app.kubernetes.io/instance={canary_name}"));

    let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;
    let pod = pods.items.iter().find(|p| {
        p.status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false)
    });

    let pod_ip = match pod.and_then(|p| p.status.as_ref()?.pod_ip.as_deref()) {
        Some(ip) => ip.to_string(),
        None => return Err(Error::ConfigError("No ready canary pod found".to_string())),
    };

    // Probe the Horizon /health endpoint multiple times to estimate error rate
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::ConfigError(format!("HTTP client error: {e}")))?;

    let url = format!("http://{pod_ip}:8000/health");
    let sample_count = 5u32;
    let mut errors = 0u32;

    for _ in 0..sample_count {
        match http_client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status >= 400 {
                    errors += 1;
                }
            }
            Err(_) => {
                errors += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    Ok(errors as f64 / sample_count as f64)
}

/// Update status for suspended nodes
#[allow(deprecated)]
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn update_suspended_status(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Set conditions for suspended state
    conditions::set_condition(
        &mut conditions,
        conditions::CONDITION_TYPE_READY,
        conditions::CONDITION_STATUS_FALSE,
        "NodeSuspended",
        "Node is offline - replicas scaled to 0. Service remains active for peer discovery.",
    );
    conditions::set_condition(
        &mut conditions,
        conditions::CONDITION_TYPE_AVAILABLE,
        conditions::CONDITION_STATUS_FALSE,
        "NodeSuspended",
        "Node is suspended and no replicas are available.",
    );
    conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
    conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);

    // Set observed generation on conditions
    if let Some(gen) = node.metadata.generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let status = StellarNodeStatus {
        message: Some("Node suspended - scaled to 0 replicas".to_string()),
        observed_generation: node.metadata.generation,
        replicas: 0,
        ready_replicas: 0,
        ledger_sequence: None,
        conditions,
        ..Default::default()
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Update the status subresource of a StellarNode using Kubernetes conditions pattern
pub(crate) fn apply_phase_conditions(
    conditions: &mut Vec<Condition>,
    phase: &str,
    message: Option<&str>,
) {
    match phase {
        "Ready" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_TRUE,
                "AllSubresourcesHealthy",
                message.unwrap_or("All sub-resources are healthy and operational"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_FALSE,
                "ReconcileComplete",
                "Reconciliation completed successfully",
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_FALSE,
                "NoIssues",
                "No degradation detected",
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_TRUE,
                "MinimumReplicasAvailable",
                "At least one replica is available and serving traffic",
            );
        }
        "Creating" | "Pending" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Creating",
                message.unwrap_or("Resources are being created"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Creating",
                message.unwrap_or("Creating resources"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Provisioning",
                "Resources are being created and are not yet available",
            );
        }
        "Syncing" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Syncing",
                message.unwrap_or("Node is syncing with the network"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Syncing",
                message.unwrap_or("Syncing data"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Syncing",
                "Node is syncing and not yet available for full traffic",
            );
        }
        "Running" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_TRUE,
                "ResourcesCreated",
                message.unwrap_or("Resources created successfully"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_FALSE,
                "Complete",
                "Resource creation complete",
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_TRUE,
                "MinimumReplicasAvailable",
                "Workload is running and available",
            );
        }
        "Degraded" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Degraded",
                message.unwrap_or("Node is experiencing issues"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "IssuesDetected",
                message.unwrap_or("Node is degraded"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Degraded",
                "Node is degraded and not considered available",
            );
        }
        "Failed" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Failed",
                message.unwrap_or("Node operation failed"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "Failed",
                message.unwrap_or("Operation failed"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Failed",
                "Node failed and is unavailable",
            );
        }
        "Remediating" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Remediating",
                message.unwrap_or("Auto-remediation in progress"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Remediating",
                message.unwrap_or("Remediation in progress"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "Remediating",
                "Node required remediation",
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Remediating",
                "Node is under remediation and not currently available",
            );
        }
        "Suspended" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Suspended",
                message.unwrap_or("Node is suspended"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Suspended",
                "Node is suspended and not available",
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Maintenance" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Maintenance",
                message.unwrap_or("Node is in maintenance mode"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_FALSE,
                "Maintenance",
                "Node is in maintenance mode and not available",
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        _ => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_UNKNOWN,
                "Unknown",
                message.unwrap_or("Status unknown"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_AVAILABLE,
                conditions::CONDITION_STATUS_UNKNOWN,
                "Unknown",
                message.unwrap_or("Availability unknown"),
            );
        }
    }
}

#[allow(deprecated)]
#[instrument(skip(client, node, message), fields(name = %node.name_any(), namespace = node.namespace(), phase))]
pub(crate) async fn update_status(
    client: &Client,
    node: &StellarNode,
    phase: &str,
    message: Option<String>,
    ready_replicas: i32,
    update_obs_gen: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let observed_generation = if update_obs_gen {
        node.metadata.generation
    } else {
        node.status
            .as_ref()
            .and_then(|status| status.observed_generation)
    };

    // Build conditions based on phase
    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    apply_phase_conditions(&mut conditions, phase, message.as_deref());

    // Set observed generation on all conditions
    if let Some(gen) = observed_generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let read_pool_endpoint = if node.spec.read_replica_config.is_some() {
        Some(crate::controller::read_pool::read_pool_endpoint(node))
    } else {
        None
    };

    let mut status_patch = serde_json::json!({
        "phase": phase,
        "observedGeneration": observed_generation,
        "replicas": if node.spec.suspended { 0 } else { node.spec.replicas },
        "readyReplicas": ready_replicas,
        "conditions": conditions,
        "readPoolEndpoint": read_pool_endpoint,
    });

    if let Some(msg) = message {
        status_patch["message"] = serde_json::Value::String(msg.to_string());
    }

    let patch = serde_json::json!({ "status": status_patch });
    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Update the status with archive health check results
/// Run the hourly archive integrity check for a validator node.
///
/// Fetches `stellar-history.json` from each configured archive, compares the reported
/// ledger sequence to the node's current ledger, and:
/// - Sets / clears the `ArchiveIntegrityDegraded` condition on the node's status.
/// - Updates the `stellar_archive_ledger_lag` Prometheus gauge so alert rules can fire.
///
/// The function is intentionally fire-and-forget on individual per-URL errors so that a
/// single unreachable archive does not block the rest of reconciliation.
#[instrument(skip(client, node, archive_urls), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn run_archive_integrity_check(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    archive_urls: &[String],
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let node_ledger = node
        .status
        .as_ref()
        .and_then(|s| s.ledger_sequence)
        .unwrap_or(0);

    // If the node has not yet reported a ledger we can't compute meaningful lag values.
    // Skip until a ledger becomes available.
    if node_ledger == 0 {
        debug!(
            "Skipping archive integrity check for {}/{}: node ledger not yet available",
            namespace, name
        );
        return Ok(());
    }

    info!(
        "Running periodic archive integrity check for {}/{} (node_ledger={})",
        namespace, name, node_ledger
    );

    let results = check_archive_integrity(archive_urls, node_ledger, None).await;

    // Determine the overall worst-case lag across all archives.
    let degraded_archives: Vec<_> = results.iter().filter(|r| !r.is_healthy()).collect();
    let any_degraded = !degraded_archives.is_empty();
    let max_lag = results.iter().filter_map(|r| r.lag).max().unwrap_or(0);

    // Update Prometheus metric with the maximum observed lag.
    #[cfg(feature = "metrics")]
    let hardware_generation = hardware_generation_for_metrics(client, node).await;
    #[cfg(feature = "metrics")]
    metrics::set_archive_ledger_lag(
        &namespace,
        &name,
        &node.spec.node_type.to_string(),
        node.spec.network_passphrase(),
        &hardware_generation,
        max_lag as i64,
    );

    // Patch the Degraded condition on the node status.
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let mut conds = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    if any_degraded {
        let messages: Vec<String> = degraded_archives.iter().map(|r| r.summary()).collect();
        let message = messages.join("; ");
        warn!(
            "Archive integrity degraded for {}/{}: {}",
            namespace, name, message
        );
        publish_stellar_event!(
            client,
            reporter,
            node,
            EventType::Warning,
            "ArchiveIntegrityDegraded",
            "ArchiveIntegrity",
            &format!("History archive(s) are lagging (max lag={max_lag}): {message}"),
        )
        .await?;
        conditions::set_condition(
            &mut conds,
            "ArchiveIntegrityDegraded",
            conditions::CONDITION_STATUS_TRUE,
            "ArchiveLagging",
            &format!(
                "Archive lag exceeds threshold of {ARCHIVE_LAG_THRESHOLD} ledgers. Max lag={max_lag}. {message}"
            ),
        );
    } else {
        // All archives healthy: clear (or keep cleared) the Degraded sub-condition.
        conditions::set_condition(
            &mut conds,
            "ArchiveIntegrityDegraded",
            conditions::CONDITION_STATUS_FALSE,
            "ArchiveInSync",
            &format!(
                "All {} archive(s) are within {} ledgers of the node",
                results.len(),
                ARCHIVE_LAG_THRESHOLD
            ),
        );
    }

    let patch = serde_json::json!({ "status": { "conditions": conds } });
    api.patch_status(
        &name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

#[instrument(skip(client, node, result), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn update_archive_health_status(
    client: &Client,
    node: &StellarNode,
    result: &ArchiveHealthResult,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Update ArchiveHealthCheck condition
    let archive_message = if result.any_healthy {
        result.summary()
    } else {
        format!("{}\n{}", result.summary(), result.error_details())
    };

    conditions::set_condition(
        &mut conditions,
        "ArchiveHealthCheck",
        if result.any_healthy {
            conditions::CONDITION_STATUS_TRUE
        } else {
            conditions::CONDITION_STATUS_FALSE
        },
        if result.any_healthy {
            "ArchiveHealthy"
        } else {
            "ArchiveUnreachable"
        },
        &archive_message,
    );

    // Set observed generation on conditions
    if let Some(gen) = node.metadata.generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let mut status_patch = serde_json::json!({
        "conditions": conditions,
        "phase": if result.any_healthy { "Creating" } else { "WaitingForArchive" },
    });

    // Don't update observed_generation if archive is unhealthy (to trigger retry)
    if result.any_healthy {
        status_patch["observedGeneration"] = serde_json::json!(node.metadata.generation);
    }

    let patch = serde_json::json!({ "status": status_patch });
    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Update the status subresource with health check results
#[allow(deprecated)]
#[instrument(skip(client, node, message, health), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn update_status_with_health(
    client: &Client,
    node: &StellarNode,
    _phase: &str,
    message: Option<String>,
    health: health::HealthCheckResult,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    // Build conditions based on health check
    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Ready condition based on health status
    if health.synced {
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_READY,
            conditions::CONDITION_STATUS_TRUE,
            "NodeSynced",
            "Node is fully synced and operational",
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_PROGRESSING,
            conditions::CONDITION_STATUS_FALSE,
            "SyncComplete",
            "Node sync completed",
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_AVAILABLE,
            conditions::CONDITION_STATUS_TRUE,
            "MinimumReplicasAvailable",
            "Node is healthy and available",
        );
        conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
    } else if health.healthy {
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_READY,
            conditions::CONDITION_STATUS_FALSE,
            "NodeSyncing",
            &health.message,
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_PROGRESSING,
            conditions::CONDITION_STATUS_TRUE,
            "Syncing",
            &health.message,
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_AVAILABLE,
            conditions::CONDITION_STATUS_TRUE,
            "MinimumReplicasAvailable",
            "Node is healthy but still syncing",
        );
        conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
    } else {
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_READY,
            conditions::CONDITION_STATUS_FALSE,
            "NodeNotHealthy",
            &health.message,
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_DEGRADED,
            conditions::CONDITION_STATUS_TRUE,
            "HealthCheckFailed",
            &health.message,
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_AVAILABLE,
            conditions::CONDITION_STATUS_FALSE,
            "HealthCheckFailed",
            "Node failed health checks and is unavailable",
        );
        conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
    }

    // Set observed generation on all conditions
    if let Some(gen) = node.metadata.generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let status = StellarNodeStatus {
        message,
        observed_generation: node.metadata.generation,
        replicas: if node.spec.suspended {
            0
        } else {
            node.spec.replicas
        },
        ready_replicas: if health.synced && !node.spec.suspended {
            node.spec.replicas
        } else {
            0
        },
        ledger_sequence: health.ledger_sequence,
        last_migrated_version: if health.synced && node.spec.node_type == NodeType::Horizon {
            Some(node.spec.version.clone())
        } else {
            node.status
                .as_ref()
                .and_then(|s| s.last_migrated_version.clone())
        },
        conditions,
        ..Default::default()
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Update the status subresource with canary information
#[allow(dead_code)]
pub(crate) async fn update_status_with_canary(
    client: &Client,
    node: &StellarNode,
    phase: &str,
    message: Option<&str>,
    ready_replicas: i32,
    canary_ready_replicas: i32,
    canary_version: Option<String>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    #[allow(deprecated)]
    let status = StellarNodeStatus {
        phase: phase.to_string(),
        message: message.map(String::from),
        observed_generation: node.metadata.generation,
        replicas: if node.spec.suspended {
            0
        } else {
            node.spec.replicas
        },
        ready_replicas,
        canary_ready_replicas,
        canary_version,
        ..Default::default()
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Run the archive checkpoint verification check
pub(crate) async fn run_archive_checkpoint_verification(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    urls: &[String],
    config: &crate::crd::ArchiveIntegrityConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    info!(
        "Running archive checkpoint integrity check for {}/{}",
        namespace, name
    );

    let mut results = Vec::new();
    for url in urls {
        match check_archive_integrity_random(
            url,
            config.check_percentage,
            config.max_checkpoints,
            Duration::from_secs(30),
        )
        .await
        {
            Ok(res) => results.push(res),
            Err(e) => {
                warn!("Archive integrity check failed for {}: {}", url, e);
                results.push(ArchiveIntegrityCheckResult {
                    url: url.clone(),
                    healthy: false,
                    checkpoints_verified: 0,
                    message: format!("Check failed: {e}"),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    let all_healthy = results.iter().all(|r| r.healthy);
    let summary = if all_healthy {
        format!(
            "Archive integrity verified: {} archives healthy",
            results.len()
        )
    } else {
        let failed = results.iter().filter(|r| !r.healthy).count();
        format!(
            "Archive integrity corruption detected: {}/{} archives corrupted",
            failed,
            results.len()
        )
    };

    // Update metrics
    #[cfg(feature = "metrics")]
    {
        let node_type = format!("{:?}", node.spec.node_type);
        let network = format!("{:?}", node.spec.network);
        let hardware = hardware_generation_for_metrics(client, node).await;
        metrics::set_archive_integrity_status(
            &namespace,
            &name,
            &node_type,
            &network,
            &hardware,
            all_healthy,
        );
    }

    // Update status conditions
    let mut status = node.status.clone().unwrap_or_default();
    if all_healthy {
        conditions::set_condition(
            &mut status.conditions,
            "ArchiveIntegrityCheck",
            conditions::CONDITION_STATUS_TRUE,
            "IntegrityVerified",
            &summary,
        );
        conditions::remove_condition(&mut status.conditions, "ArchiveIntegrityCorrupted");
    } else {
        conditions::set_condition(
            &mut status.conditions,
            "ArchiveIntegrityCheck",
            conditions::CONDITION_STATUS_FALSE,
            "IntegrityCheckFailed",
            &summary,
        );
        conditions::set_condition(
            &mut status.conditions,
            "ArchiveIntegrityCorrupted",
            conditions::CONDITION_STATUS_TRUE,
            "CorruptionDetected",
            &summary,
        );

        // Emit Fatal Event for corruption
        publish_stellar_event!(
            client,
            reporter,
            node,
            EventType::Warning,
            "ArchiveIntegrityCorruption",
            "ArchiveIntegrity",
            &format!(
                "FATAL: Corruption detected in history archives!\n\nDetails:\n{}",
                results
                    .iter()
                    .filter(|r| !r.healthy)
                    .map(|r| format!("- {}: {}", r.url, r.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        )
        .await?;
    }

    // Set observed generation
    if let Some(gen) = node.metadata.generation {
        for condition in &mut status.conditions {
            if condition.type_ == "ArchiveIntegrityCheck"
                || condition.type_ == "ArchiveIntegrityCorrupted"
            {
                condition.observed_generation = Some(gen);
            }
        }
    }

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Helper to parse duration string (e.g. "1h", "6h", "24h")
pub(crate) fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if let Some(h) = s.strip_suffix('h') {
        let hours = h
            .parse::<u64>()
            .map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(hours * 3600))
    } else if let Some(m) = s.strip_suffix('m') {
        let mins = m
            .parse::<u64>()
            .map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(mins * 60))
    } else if let Some(sec) = s.strip_suffix('s') {
        let secs = sec
            .parse::<u64>()
            .map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(secs))
    } else {
        Err(Error::ConfigError(format!(
            "Unsupported duration format: {s}"
        )))
    }
}

/// Helper to get the latest ledger from the Stellar network
pub(crate) async fn get_latest_network_ledger(network: &crate::crd::StellarNetwork) -> Result<u64> {
    let url = match network {
        crate::crd::StellarNetwork::Mainnet => "https://horizon.stellar.org",
        crate::crd::StellarNetwork::Testnet => "https://horizon-testnet.stellar.org",
        crate::crd::StellarNetwork::Futurenet => "https://horizon-futurenet.stellar.org",
        crate::crd::StellarNetwork::Custom(_) => {
            return Err(Error::ConfigError(
                "Custom network not supported for lag calculation yet".to_string(),
            ))
        }
    };

    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.map_err(Error::HttpError)?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let ledger = json["history_latest_ledger"].as_u64().ok_or_else(|| {
        Error::ConfigError("Failed to get latest ledger from horizon".to_string())
    })?;
    Ok(ledger)
}
/// Update the status with DR results
#[instrument(skip(client, node, dr_status), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn update_dr_status(
    client: &Client,
    node: &StellarNode,
    dr_status: DisasterRecoveryStatus,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let patch = serde_json::json!({
        "status": {
            "drStatus": dr_status
        }
    });

    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

pub(crate) async fn update_cross_cloud_failover_status(
    client: &Client,
    node: &StellarNode,
    status: crate::crd::CrossCloudFailoverStatus,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let patch = serde_json::json!({
        "status": {
            "crossCloudFailoverStatus": status
        }
    });

    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}
