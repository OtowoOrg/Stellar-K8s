//! Controller for ReadOnlyPool resources
//!
//! Manages auto-scaling pools of read-only Stellar nodes with weighted load balancing
//! and shard balancing capabilities.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Pod, Service};
use kube::{
    api::{Api, Patch, PatchParams},
    client::Client,
    runtime::{
        controller::{Action, Controller},
        watcher::Config,
    },
    ResourceExt,
};
use tracing::{error, info, instrument, warn};

use crate::crd::{
    Condition, ReadOnlyPool, ReadOnlyPoolStatus, ReplicaWeight, ShardAssignment,
    ShardStrategy,
};
use crate::error::{Error, Result};

use super::conditions;
use super::read_only_pool_resources;

/// Shared state for the ReadOnlyPool controller
pub struct ReadOnlyPoolControllerState {
    /// Kubernetes client for API interactions
    pub client: Client,
}

/// Main entry point to start the ReadOnlyPool controller
pub async fn run_read_only_pool_controller(
    state: Arc<ReadOnlyPoolControllerState>,
) -> Result<()> {
    let client = state.client.clone();
    let pools: Api<ReadOnlyPool> = Api::all(client.clone());

    info!("Starting ReadOnlyPool controller");

    // Verify CRD exists
    match pools.list(&Default::default()).await {
        Ok(_) => info!("ReadOnlyPool CRD is available"),
        Err(e) => {
            error!("ReadOnlyPool CRD not found: {:?}", e);
            return Err(Error::ConfigError(
                "ReadOnlyPool CRD not installed".to_string(),
            ));
        }
    }

    Controller::new(pools, Config::default())
        .owns::<Deployment>(Api::all(client.clone()), Config::default())
        .owns::<Service>(Api::all(client.clone()), Config::default())
        .owns::<Pod>(Api::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile_read_only_pool, error_policy, state)
        .for_each(|res| async move {
            match res {
                Ok(obj) => info!("Reconciled ReadOnlyPool: {:?}", obj),
                Err(e) => error!("Reconcile error: {:?}", e),
            }
        })
        .await;

    Ok(())
}

/// Main reconciliation function for ReadOnlyPool
#[instrument(skip(ctx), fields(name = %obj.name_any(), namespace = obj.namespace()))]
async fn reconcile_read_only_pool(
    obj: Arc<ReadOnlyPool>,
    ctx: Arc<ReadOnlyPoolControllerState>,
) -> Result<Action> {
    let client = ctx.client.clone();
    let namespace = obj.namespace().unwrap_or_else(|| "default".to_string());
    let name = obj.name_any();

    info!(
        "Reconciling ReadOnlyPool {}/{}",
        namespace, name
    );

    // Validate spec
    if let Err(e) = obj.spec.validate() {
        warn!("Validation failed for {}/{}: {}", namespace, name, e);
        update_status(&client, &obj, |status| {
            status.conditions = vec![Condition {
                type_: "Ready".to_string(),
                status: "False".to_string(),
                last_transition_time: chrono::Utc::now().to_rfc3339(),
                reason: "ValidationFailed".to_string(),
                message: e.clone(),
                observed_generation: None,
            }];
        })
        .await?;
        return Err(Error::ValidationError(e));
    }

    // 1. Ensure ConfigMap exists
    read_only_pool_resources::ensure_config_map(&client, &obj).await?;

    // 2. Ensure Deployment exists
    ensure_deployment(&client, &obj).await?;

    // 3. Ensure Service exists
    ensure_service(&client, &obj).await?;

    // 4. Check health of all pods and collect metrics
    let pod_health = check_pool_health(&client, &obj).await?;

    // 5. Calculate load balancing weights
    let replica_weights = if obj.spec.load_balancing.enabled {
        calculate_load_balancing_weights(&obj, &pod_health).await?
    } else {
        vec![]
    };

    // 6. Calculate shard assignments
    let shard_assignments = if obj.spec.shard_balancing.enabled {
        calculate_shard_assignments(&obj, &pod_health).await?
    } else {
        vec![]
    };

    // 7. Update Service with weighted endpoints (if load balancing enabled)
    if obj.spec.load_balancing.enabled {
        update_service_weights(&client, &obj, &replica_weights).await?;
    }

    // 8. Update pod annotations with shard assignments
    if obj.spec.shard_balancing.enabled {
        update_pod_shard_assignments(&client, &obj, &shard_assignments).await?;
    }

    // 9. Auto-scale based on metrics
    let target_replicas = calculate_target_replicas(&obj, &pod_health).await?;
    if target_replicas != pod_health.current_replicas {
        info!(
            "Scaling pool {}/{} from {} to {} replicas",
            namespace, name, pod_health.current_replicas, target_replicas
        );
        scale_deployment(&client, &obj, target_replicas).await?;
    }

    // 10. Update status
    update_pool_status(&client, &obj, &pod_health, &replica_weights, &shard_assignments).await?;

    // Requeue based on update interval
    let requeue_duration = if obj.spec.load_balancing.enabled {
        Duration::from_secs(obj.spec.load_balancing.update_interval_seconds)
    } else {
        Duration::from_secs(60)
    };

    Ok(Action::requeue(requeue_duration))
}

/// Health information for the pool
#[derive(Debug, Clone)]
struct PoolHealth {
    current_replicas: i32,
    ready_replicas: i32,
    fresh_replicas: i32,
    lagging_replicas: i32,
    replica_health: Vec<ReplicaHealth>,
    average_ledger_sequence: Option<u64>,
    network_latest_ledger: Option<u64>,
    average_lag: Option<i64>,
}

/// Health information for a single replica
#[derive(Debug, Clone)]
struct ReplicaHealth {
    pod_name: String,
    ready: bool,
    ledger_sequence: Option<u64>,
    lag: Option<i64>,
    is_fresh: bool,
}

/// Check health of all pods in the pool
async fn check_pool_health(
    client: &Client,
    pool: &ReadOnlyPool,
) -> Result<PoolHealth> {
    let namespace = pool.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    // Get all pods for this pool
    let label_selector = format!("app.kubernetes.io/instance={}", pool.name_any());
    let pods = pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await?;

    let mut replica_health = Vec::new();
    let mut ledger_sequences = Vec::new();
    let mut lags = Vec::new();

    // Get network latest ledger
    let network_latest = get_network_latest_ledger(&pool.spec.network).await.ok();

    for pod in pods.items {
        let pod_name = pod.name_any();
        let ready = pod
            .status
            .as_ref()
            .and_then(|s| {
                s.conditions
                    .as_ref()
                    .and_then(|conditions| {
                        conditions
                            .iter()
                            .find(|c| c.type_ == "Ready")
                            .map(|c| c.status == "True")
                    })
            })
            .unwrap_or(false);

        // Try to get ledger sequence from pod annotations or metrics
        let ledger_sequence = get_pod_ledger_sequence(client, &namespace, &pod_name).await.ok();
        let lag = ledger_sequence
            .and_then(|seq| network_latest.map(|latest| (latest as i64) - (seq as i64)));

        if let Some(seq) = ledger_sequence {
            ledger_sequences.push(seq);
        }
        if let Some(lag_val) = lag {
            lags.push(lag_val);
        }

        let lag_threshold = pool.spec.load_balancing.lag_threshold;
        let is_fresh = lag
            .map(|l| l >= 0 && (l as u64) <= lag_threshold)
            .unwrap_or(false);

        replica_health.push(ReplicaHealth {
            pod_name,
            ready,
            ledger_sequence,
            lag,
            is_fresh,
        });
    }

    let current_replicas = replica_health.len() as i32;
    let ready_replicas = replica_health.iter().filter(|r| r.ready).count() as i32;
    let fresh_replicas = replica_health.iter().filter(|r| r.is_fresh).count() as i32;
    let lagging_replicas = replica_health.iter().filter(|r| !r.is_fresh).count() as i32;

    let average_ledger_sequence = if !ledger_sequences.is_empty() {
        Some(
            (ledger_sequences.iter().sum::<u64>() as f64
                / ledger_sequences.len() as f64) as u64,
        )
    } else {
        None
    };

    let average_lag = if !lags.is_empty() {
        Some(lags.iter().sum::<i64>() / lags.len() as i64)
    } else {
        None
    };

    Ok(PoolHealth {
        current_replicas,
        ready_replicas,
        fresh_replicas,
        lagging_replicas,
        replica_health,
        average_ledger_sequence,
        network_latest_ledger: network_latest,
        average_lag,
    })
}

/// Get ledger sequence for a pod
async fn get_pod_ledger_sequence(
    client: &Client,
    namespace: &str,
    pod_name: &str,
) -> Result<u64> {
    // Try to query the pod's metrics endpoint or annotation
    // For now, we'll use a placeholder - in production this would query
    // the Stellar Core metrics endpoint
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = pod_api.get(pod_name).await?;

    // Check annotation first
    if let Some(annotations) = &pod.metadata.annotations {
        if let Some(seq_str) = annotations.get("stellar.org/ledger-sequence") {
            if let Ok(seq) = seq_str.parse::<u64>() {
                return Ok(seq);
            }
        }
    }

    // TODO: Query metrics endpoint
    // For now, return error to indicate we couldn't determine it
    Err(Error::ConfigError(
        "Could not determine ledger sequence".to_string(),
    ))
}

/// Get the latest ledger from the network
async fn get_network_latest_ledger(network: &crate::crd::StellarNetwork) -> Result<u64> {
    let url = match network {
        crate::crd::StellarNetwork::Mainnet => "https://horizon.stellar.org",
        crate::crd::StellarNetwork::Testnet => "https://horizon-testnet.stellar.org",
        crate::crd::StellarNetwork::Futurenet => "https://horizon-futurenet.stellar.org",
        crate::crd::StellarNetwork::Custom(_) => {
            return Err(Error::ConfigError(
                "Custom network not supported for ledger lookup".to_string(),
            ))
        }
    };

    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.map_err(Error::HttpError)?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let ledger = json["history_latest_ledger"]
        .as_u64()
        .ok_or_else(|| Error::ConfigError("Failed to get latest ledger from horizon".to_string()))?;
    Ok(ledger)
}

/// Calculate load balancing weights for replicas
#[allow(clippy::unnecessary_wraps)]
async fn calculate_load_balancing_weights(
    pool: &ReadOnlyPool,
    health: &PoolHealth,
) -> Result<Vec<ReplicaWeight>> {
    let mut weights = Vec::new();
    let fresh_weight = pool.spec.load_balancing.fresh_node_weight;
    let lagging_weight = pool.spec.load_balancing.lagging_node_weight;

    for replica in &health.replica_health {
        let weight = if replica.is_fresh {
            fresh_weight
        } else {
            lagging_weight
        };

        weights.push(ReplicaWeight {
            replica_name: replica.pod_name.clone(),
            weight,
            ledger_sequence: replica.ledger_sequence,
            lag: replica.lag,
            is_fresh: replica.is_fresh,
            last_updated: chrono::Utc::now().to_rfc3339(),
        });
    }

    Ok(weights)
}

/// Calculate shard assignments for replicas
#[allow(clippy::unnecessary_wraps)]
async fn calculate_shard_assignments(
    pool: &ReadOnlyPool,
    health: &PoolHealth,
) -> Result<Vec<ShardAssignment>> {
    let mut assignments = Vec::new();
    let shard_count = pool.spec.shard_balancing.shard_count;
    let archive_urls = &pool.spec.history_archive_urls;

    if archive_urls.is_empty() {
        return Ok(assignments);
    }

    match pool.spec.shard_balancing.strategy {
        ShardStrategy::RoundRobin => {
            for (idx, replica) in health.replica_health.iter().enumerate() {
                let shard_id = (idx % shard_count as usize) as i32;
                let archive_url = archive_urls[shard_id as usize % archive_urls.len()].clone();

                assignments.push(ShardAssignment {
                    replica_name: replica.pod_name.clone(),
                    shard_id,
                    archive_url,
                    ledger_range: None, // Round-robin doesn't use ledger ranges
                });
            }
        }
        ShardStrategy::HashBased => {
            // Use consistent hashing based on pod name
            for replica in &health.replica_health {
                let hash = simple_hash(&replica.pod_name);
                let shard_id = (hash % shard_count as u64) as i32;
                let archive_url = archive_urls[shard_id as usize % archive_urls.len()].clone();

                assignments.push(ShardAssignment {
                    replica_name: replica.pod_name.clone(),
                    shard_id,
                    archive_url,
                    ledger_range: None,
                });
            }
        }
        ShardStrategy::Manual => {
            // Manual assignments are set via annotations, just read them
            // This would be implemented by reading pod annotations
            // For now, leave assignments empty - they will be set manually
            let _ = assignments;
        }
    }

    Ok(assignments)
}

/// Simple hash function for consistent hashing
fn simple_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Calculate target number of replicas based on metrics
#[allow(clippy::unnecessary_wraps)]
async fn calculate_target_replicas(
    pool: &ReadOnlyPool,
    health: &PoolHealth,
) -> Result<i32> {
    // Start with the current target
    let mut target = pool.spec.target_replicas;

    // If we have lagging replicas and load balancing is enabled, we might want more replicas
    if pool.spec.load_balancing.enabled && health.lagging_replicas > 0 {
        // Scale up if too many replicas are lagging
        let lagging_ratio = health.lagging_replicas as f64 / health.current_replicas as f64;
        if lagging_ratio > 0.5 {
            target = (target as f64 * 1.2).ceil() as i32;
        }
    }

    // Ensure within bounds
    target = target.max(pool.spec.min_replicas);
    target = target.min(pool.spec.max_replicas);

    Ok(target)
}

/// Ensure Deployment exists for the pool
async fn ensure_deployment(client: &Client, pool: &ReadOnlyPool) -> Result<()> {
    read_only_pool_resources::ensure_deployment(client, pool).await
}

/// Ensure Service exists for the pool
async fn ensure_service(client: &Client, pool: &ReadOnlyPool) -> Result<()> {
    read_only_pool_resources::ensure_service(client, pool).await
}

/// Update Service with weighted endpoints
#[allow(clippy::unnecessary_wraps)]
async fn update_service_weights(
    _client: &Client,
    pool: &ReadOnlyPool,
    weights: &[ReplicaWeight],
) -> Result<()> {
    // This would update the Service with endpoint weights
    // In Kubernetes, this could be done via EndpointSlice annotations
    // or using a service mesh configuration
    info!(
        "Updating service weights for ReadOnlyPool {} with {} weights",
        pool.name_any(),
        weights.len()
    );
    Ok(())
}

/// Update pod annotations with shard assignments
async fn update_pod_shard_assignments(
    client: &Client,
    pool: &ReadOnlyPool,
    assignments: &[ShardAssignment],
) -> Result<()> {
    let namespace = pool.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    for assignment in assignments {
        let pod = pod_api.get(&assignment.replica_name).await?;
        let mut annotations = pod.metadata.annotations.clone().unwrap_or_default();

        annotations.insert(
            "stellar.org/shard-id".to_string(),
            assignment.shard_id.to_string(),
        );
        annotations.insert(
            "stellar.org/archive-url".to_string(),
            assignment.archive_url.clone(),
        );

        let patch = serde_json::json!({
            "metadata": {
                "annotations": annotations
            }
        });

        pod_api
            .patch(
                &assignment.replica_name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Merge(&patch),
            )
            .await?;
    }

    Ok(())
}

/// Scale the Deployment to target replicas
async fn scale_deployment(
    client: &Client,
    pool: &ReadOnlyPool,
    target_replicas: i32,
) -> Result<()> {
    let namespace = pool.namespace().unwrap_or_else(|| "default".to_string());
    let deployment_api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let name = pool.name_any();

    let patch = serde_json::json!({
        "spec": {
            "replicas": target_replicas
        }
    });

    deployment_api
        .patch(
            &name,
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&patch),
        )
        .await?;

    Ok(())
}

/// Update the pool status
async fn update_pool_status(
    client: &Client,
    pool: &ReadOnlyPool,
    health: &PoolHealth,
    weights: &[ReplicaWeight],
    assignments: &[ShardAssignment],
) -> Result<()> {
    let namespace = pool.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ReadOnlyPool> = Api::namespaced(client.clone(), &namespace);

    let mut conditions = pool
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Update Ready condition
    if health.ready_replicas >= health.current_replicas && health.current_replicas > 0 {
        conditions::set_condition(
            &mut conditions,
            "Ready",
            "True",
            "AllReplicasReady",
            &format!("All {} replicas are ready", health.current_replicas),
        );
    } else {
        conditions::set_condition(
            &mut conditions,
            "Ready",
            "False",
            "ReplicasNotReady",
            &format!(
                "{}/{} replicas are ready",
                health.ready_replicas, health.current_replicas
            ),
        );
    }

    let status = ReadOnlyPoolStatus {
        current_replicas: health.current_replicas,
        ready_replicas: health.ready_replicas,
        fresh_replicas: health.fresh_replicas,
        lagging_replicas: health.lagging_replicas,
        observed_generation: pool.metadata.generation,
        conditions,
        replica_weights: weights.to_vec(),
        shard_assignments: assignments.to_vec(),
        average_ledger_sequence: health.average_ledger_sequence,
        network_latest_ledger: health.network_latest_ledger,
        average_lag: health.average_lag,
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &pool.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    Ok(())
}

/// Helper to update status
#[allow(clippy::unnecessary_wraps)]
async fn update_status<F>(client: &Client, pool: &ReadOnlyPool, f: F) -> Result<()>
where
    F: FnOnce(&mut ReadOnlyPoolStatus),
{
    let namespace = pool.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ReadOnlyPool> = Api::namespaced(client.clone(), &namespace);

    let mut status = pool.status.clone().unwrap_or_default();
    f(&mut status);

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &pool.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    Ok(())
}

/// Error policy for the controller
fn error_policy(
    pool: Arc<ReadOnlyPool>,
    error: &Error,
    _ctx: Arc<ReadOnlyPoolControllerState>,
) -> Action {
    error!("Reconciliation error for {}: {:?}", pool.name_any(), error);

    let retry_duration = if error.is_retriable() {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(60)
    };

    Action::requeue(retry_duration)
}
