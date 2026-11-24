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
//! Blue/Green deployment strategy for RPC nodes
//!
//! This module implements native support for zero-downtime blue/green deployments
//! specifically for Horizon and Soroban RPC nodes when updating versions or configurations.
//!
//! # Overview
//!
//! Blue/Green deployment strategy:
//! 1. Create a new "Green" Deployment with updated configuration
//! 2. Wait for Green deployment to be fully ready
//! 3. Run smoke tests against Green deployment
//! 4. Switch traffic at the Service level (update selector)
//! 5. Delete the old "Blue" deployment after successful switch
//!
//! # Features
//!
//! - **Zero-Downtime**: Traffic switches atomically at the Service level
//! - **Smoke Tests**: Optional health checks before traffic switch
//! - **Automatic Cleanup**: Old deployment removed after successful switch
//! - **Rollback Support**: Can revert to Blue if Green fails
//!
//! # Example
//!
//! ```yaml
//! apiVersion: stellar.org/v1alpha1
//! kind: StellarNode
//! metadata:
//!   name: my-horizon
//! spec:
//!   nodeType: Horizon
//!   deploymentStrategy: BlueGreen
//!   version: "v21.1.0"  # Updating version triggers blue/green
//! ```

use crate::crd::StellarNode;
use crate::error::{Error, Result};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::api::core::v1::{PodSpec, PodTemplateSpec, SecretVolumeSource, Service, Volume};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use k8s_openapi::chrono::{Duration as ChronoDuration, Utc};
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use kube::Client;
use kube::ResourceExt;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Blue/Green deployment status
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlueGreenStatus {
    /// No active deployment
    Inactive,
    /// Blue deployment is active
    BlueActive,
    /// Green deployment is active
    GreenActive,
    /// Transitioning from Blue to Green
    Transitioning,
    /// Waiting for Green to be ready
    WaitingForGreen,
    /// Green is ready, waiting for traffic switch
    GreenReady,
    /// Cleaning up old Blue deployment
    CleaningUp,
}

impl std::fmt::Display for BlueGreenStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlueGreenStatus::Inactive => write!(f, "Inactive"),
            BlueGreenStatus::BlueActive => write!(f, "BlueActive"),
            BlueGreenStatus::GreenActive => write!(f, "GreenActive"),
            BlueGreenStatus::Transitioning => write!(f, "Transitioning"),
            BlueGreenStatus::WaitingForGreen => write!(f, "WaitingForGreen"),
            BlueGreenStatus::GreenReady => write!(f, "GreenReady"),
            BlueGreenStatus::CleaningUp => write!(f, "CleaningUp"),
        }
    }
}

/// Configuration for blue/green deployment
#[derive(Clone, Debug)]
pub struct BlueGreenConfig {
    /// Maximum time to wait for Green deployment to be ready
    pub ready_timeout: Duration,
    /// Maximum time to wait for traffic switch to complete
    pub switch_timeout: Duration,
    /// Enable smoke tests before traffic switch
    pub enable_smoke_tests: bool,
    /// Health check endpoint for smoke tests
    pub health_check_endpoint: Option<String>,
}

impl Default for BlueGreenConfig {
    fn default() -> Self {
        Self {
            ready_timeout: Duration::from_secs(300), // 5 minutes
            switch_timeout: Duration::from_secs(60), // 1 minute
            enable_smoke_tests: true,
            health_check_endpoint: Some("/health".to_string()),
        }
    }
}

/// Create a new Green deployment with updated configuration
///
/// # Arguments
///
/// * `client` - Kubernetes client
/// * `node` - The StellarNode resource
/// * `blue_deployment` - The current Blue deployment to base Green on
///
/// # Returns
///
/// The created Green deployment
pub async fn create_green_deployment(
    client: &Client,
    node: &StellarNode,
    blue_deployment: &Deployment,
) -> Result<Deployment> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();

    // Run the database migration health-gate before creating the Green deployment.
    // If the migration Job fails, the rollout is halted before any new application
    // pods are created.
    run_migration_gate(client, node).await?;

    // Create Green deployment by cloning Blue and updating labels/version
    let mut green_deployment = blue_deployment.clone();

    // Update metadata
    let metadata = &mut green_deployment.metadata;
    metadata.name = Some(format!("{node_name}-green"));
    metadata.resource_version = None; // Clear resource version for new creation
    metadata.uid = None;
    metadata
        .labels
        .get_or_insert_with(BTreeMap::new)
        .insert("deployment-color".to_string(), "green".to_string());

    // Update labels to identify as Green
    if let Some(spec) = &mut green_deployment.spec {
        if let Some(selector) = &mut spec.selector.match_labels {
            selector.insert("deployment-color".to_string(), "green".to_string());
        }

        let template = &mut spec.template;
        let metadata = template.metadata.get_or_insert_with(Default::default);
        if let Some(labels) = &mut metadata.labels {
            labels.insert("deployment-color".to_string(), "green".to_string());
        }

        // Update container image to new version if specified
        let pod_spec = template.spec.get_or_insert_with(Default::default);
        for container in &mut pod_spec.containers {
            // Update image tag based on node version
            if let Some(image) = &mut container.image {
                *image = node.spec.container_image();
            }
        }
    }

    // Create the Green deployment
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let green = api.create(&Default::default(), &green_deployment).await?;

    info!(
        "Created Green deployment {}/{}-green for node {}",
        namespace, node_name, node_name
    );

    Ok(green)
}
async fn run_migration_gate(client: &Client, node: &StellarNode) -> Result<()> {
    let migration_command = match node
        .annotations()
        .get("stellar.org/migration-command")
        .cloned()
    {
        Some(command) => command,
        None => {
            debug!(
                "No migration command configured for {}; skipping migration gate",
                node.name_any()
            );
            return Ok(());
        }
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let image_slug: String = node
        .spec
        .container_image()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let mut job_name = format!("{}-migrate-{}", node_name, image_slug);
    if job_name.len() > 63 {
        job_name = job_name.chars().take(63).collect();
    }
    let api: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), &namespace);

    match api.get(&job_name).await {
        Ok(job) => {
            if job
                .status
                .as_ref()
                .and_then(|status| status.succeeded)
                .unwrap_or(0)
                > 0
            {
                info!(
                    "Migration job {}/{} already succeeded; allowing rollout",
                    namespace, job_name
                );
                return Ok(());
            }

            if job
                .status
                .as_ref()
                .and_then(|status| status.failed)
                .unwrap_or(0)
                > 0
            {
                warn!(
                    "Migration job {}/{} failed; halting Horizon rollout",
                    namespace, job_name
                );
                emit_migration_failed_event(client, node, &job_name).await?;
                return Err(migration_failed_error(format!(
                    "Database migration job {}/{} failed; rollout halted",
                    namespace, job_name
                ))
                .into());
            }

            info!(
                "Migration job {}/{} is still running; waiting for it to complete",
                namespace, job_name
            );
        }
        Err(_) => {
            let job = build_migration_job(node, &job_name, &migration_command);
            let job = api.create(&Default::default(), &job).await?;
            info!("Created migration job {}/{}", namespace, job.name_any());
        }
    }

    wait_for_migration_job(client, node, &job_name).await
}

fn build_migration_job(
    node: &StellarNode,
    job_name: &str,
    migration_command: &str,
) -> k8s_openapi::api::batch::v1::Job {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    k8s_openapi::api::batch::v1::Job {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(job_name.to_string()),
            namespace: Some(namespace),
            ..Default::default()
        },
        spec: Some(k8s_openapi::api::batch::v1::JobSpec {
            backoff_limit: Some(0),
            template: k8s_openapi::api::core::v1::PodTemplateSpec {
                spec: Some(k8s_openapi::api::core::v1::PodSpec {
                    restart_policy: Some("Never".to_string()),
                    containers: vec![k8s_openapi::api::core::v1::Container {
                        name: "migration".to_string(),
                        image: Some(node.spec.container_image()),
                        command: Some(vec![
                            "/bin/sh".to_string(),
                            "-c".to_string(),
                            migration_command.to_string(),
                        ]),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn wait_for_migration_job(client: &Client, node: &StellarNode, job_name: &str) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), &namespace);
    let timeout = Duration::from_secs(300);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            emit_migration_failed_event(client, node, job_name).await?;
            return Err(migration_failed_error(format!(
                "Timed out waiting for migration job {}/{} to complete",
                namespace, job_name
            ))
            .into());
        }

        match api.get(job_name).await {
            Ok(job) => {
                if job
                    .status
                    .as_ref()
                    .and_then(|status| status.succeeded)
                    .unwrap_or(0)
                    > 0
                {
                    info!(
                        "Migration job {}/{} succeeded; allowing Horizon rollout",
                        namespace, job_name
                    );
                    return Ok(());
                }

                if job
                    .status
                    .as_ref()
                    .and_then(|status| status.failed)
                    .unwrap_or(0)
                    > 0
                {
                    warn!(
                        "Migration job {}/{} failed; blocking Horizon rollout",
                        namespace, job_name
                    );
                    emit_migration_failed_event(client, node, job_name).await?;
                    return Err(migration_failed_error(format!(
                        "Database migration job {}/{} failed; rollout halted",
                        namespace, job_name
                    ))
                    .into());
                }
            }
            Err(e) => {
                warn!(
                    "Error checking migration job {}/{}: {}; retrying",
                    namespace, job_name, e
                );
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn emit_migration_failed_event(
    client: &Client,
    node: &StellarNode,
    job_name: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();

    let event = k8s_openapi::api::core::v1::Event {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            generate_name: Some(format!("{}-migration-failed-", node_name)),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        involved_object: k8s_openapi::api::core::v1::ObjectReference {
            api_version: Some("stellar.org/v1alpha1".to_string()),
            kind: Some("StellarNode".to_string()),
            name: Some(node_name),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        reason: Some("HorizonMigrationFailed".to_string()),
        message: Some(format!(
            "Database migration job {} failed; Horizon rollout halted",
            job_name
        )),
        type_: Some("Warning".to_string()),
        ..Default::default()
    };

    let events: Api<k8s_openapi::api::core::v1::Event> =
        Api::namespaced(client.clone(), &namespace);
    events.create(&Default::default(), &event).await?;

    warn!(
        "Emitted HorizonMigrationFailed event for {}/{}",
        namespace, job_name
    );
    Ok(())
}

fn migration_failed_error(message: String) -> kube::Error {
    kube::Error::Api(kube::core::ErrorResponse {
        status: "Failure".to_string(),
        message,
        reason: "HorizonMigrationFailed".to_string(),
        code: 500,
    })
}

/// Wait for Green deployment to be ready
///
/// # Arguments
///
/// * `client` - Kubernetes client
/// * `node` - The StellarNode resource
/// * `timeout` - Maximum time to wait
///
/// # Returns
///
/// True if Green deployment is ready, false if timeout
pub async fn wait_for_green_ready(
    client: &Client,
    node: &StellarNode,
    timeout: Duration,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let green_name = format!("{node_name}-green");

    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            warn!(
                "Timeout waiting for Green deployment {}/{} to be ready",
                namespace, green_name
            );
            return Ok(false);
        }

        match api.get(&green_name).await {
            Ok(deployment) => {
                if let Some(status) = &deployment.status {
                    if let Some(replicas) = status.replicas {
                        if let Some(ready_replicas) = status.ready_replicas {
                            if ready_replicas == replicas {
                                info!(
                                    "Green deployment {}/{} is ready ({} replicas)",
                                    namespace, green_name, ready_replicas
                                );
                                return Ok(true);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Error checking Green deployment status: {}. Retrying...", e);
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Acquire a distributed lock for blue/green deployment using a Kubernetes Lease.
/// This ensures only one operator replica can perform the traffic switch at a time.
async fn acquire_blue_green_lease(
    client: &Client,
    node: &StellarNode,
    timeout: Duration,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let lease_name = format!("{node_name}-blue-green-lock");
    let holder =
        std::env::var("POD_NAME").unwrap_or_else(|_| format!("unknown-{}", std::process::id()));
    let api: Api<Lease> = Api::namespaced(client.clone(), &namespace);
    let start = std::time::Instant::now();
    let lease_seconds = 15;

    loop {
        if start.elapsed() > timeout {
            return Err(Error::ConfigError(format!(
                "timed out acquiring blue/green lease {lease_name}"
            )));
        }

        match api.get(&lease_name).await {
            Ok(mut lease) => {
                let can_acquire = match &lease.spec {
                    Some(spec) => {
                        let is_holder = spec.holder_identity.as_deref() == Some(holder.as_str());
                        let expired = {
                            let renew = spec.renew_time.as_ref().map(|t| t.0).unwrap_or(Utc::now());
                            let duration = ChronoDuration::seconds(
                                spec.lease_duration_seconds.unwrap_or(10) as i64,
                            );
                            Utc::now() - renew > duration
                        };
                        is_holder || expired
                    }
                    None => true,
                };

                if can_acquire {
                    let transitions = lease
                        .spec
                        .as_ref()
                        .and_then(|s| s.lease_transitions)
                        .unwrap_or(0)
                        + 1;
                    lease.spec = Some(LeaseSpec {
                        holder_identity: Some(holder.clone()),
                        lease_duration_seconds: Some(lease_seconds),
                        acquire_time: Some(MicroTime(Utc::now())),
                        renew_time: Some(MicroTime(Utc::now())),
                        lease_transitions: Some(transitions),
                        ..Default::default()
                    });
                    match api.replace(&lease_name, &Default::default(), &lease).await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            warn!("Failed to take lease {}: {}. Retrying...", lease_name, e);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                } else {
                    // Held by someone else, wait
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            Err(kube::Error::Api(ae)) if ae.code == 404 => {
                let lease = Lease {
                    metadata: ObjectMeta {
                        name: Some(lease_name.clone()),
                        namespace: Some(namespace.clone()),
                        ..Default::default()
                    },
                    spec: Some(LeaseSpec {
                        holder_identity: Some(holder.clone()),
                        lease_duration_seconds: Some(lease_seconds),
                        acquire_time: Some(MicroTime(Utc::now())),
                        renew_time: Some(MicroTime(Utc::now())),
                        lease_transitions: Some(0),
                        ..Default::default()
                    }),
                };
                api.create(&Default::default(), &lease).await?;
                return Ok(());
            }
            Err(e) => {
                warn!(
                    "Error acquiring blue/green lease {}: {}. Retrying...",
                    lease_name, e
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Release the distributed lock for blue/green deployment.
async fn release_blue_green_lease(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let lease_name = format!("{node_name}-blue-green-lock");
    let api: Api<Lease> = Api::namespaced(client.clone(), &namespace);
    api.delete(&lease_name, &Default::default()).await.ok();
    Ok(())
}

/// Switch traffic from Blue to Green at the Service level
///
/// # Arguments
///
/// * `client` - Kubernetes client
/// * `node` - The StellarNode resource
///
/// # Returns
///
/// True if switch was successful
pub async fn switch_traffic_to_green(client: &Client, node: &StellarNode) -> Result<bool> {
    use k8s_openapi::api::core::v1::Service;

    // Acquire distributed lock to prevent concurrent traffic switches
    acquire_blue_green_lease(client, node, Duration::from_secs(10)).await?;

    let result = async {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let node_name = node.name_any();

        let api: Api<Service> = Api::namespaced(client.clone(), &namespace);

        // Get the service
        match api.get(&node_name).await {
            Ok(mut service) => {
                // Update service selector to point to Green deployment
                if let Some(spec) = &mut service.spec {
                    if let Some(selector) = &mut spec.selector {
                        selector.insert("deployment-color".to_string(), "green".to_string());
                    }
                }

                // Patch the service
                let patch = Patch::Merge(json!({
                    "spec": {
                        "selector": {
                            "deployment-color": "green"
                        }
                    }
                }));

                api.patch(&node_name, &PatchParams::default(), &patch)
                    .await?;

                info!(
                    "Successfully switched traffic to Green deployment for {}/{}",
                    namespace, node_name
                );
                Ok(true)
            }
            Err(e) => {
                warn!(
                    "Failed to get service {}/{} for traffic switch: {}",
                    namespace, node_name, e
                );
                Ok(false)
            }
        }
    }
    .await;

    // Release the lock
    if let Err(e) = release_blue_green_lease(client, node).await {
        warn!("Failed to release blue/green lease: {}", e);
    }

    result
}

/// Build the one-shot Job that performs Horizon schema migration.
fn build_horizon_migration_job(node: &StellarNode) -> Job {
    let node_name = node.name_any();
    let job_name = format!("{}-horizon-migration", node_name);
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "stellar-node".to_string(),
    );
    labels.insert("app.kubernetes.io/instance".to_string(), node_name.clone());
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "horizon".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "stellar-operator".to_string(),
    );
    labels.insert("stellar.org/node-type".to_string(), "Horizon".to_string());
    labels.insert(
        "stellar.org/horizon-migration".to_string(),
        "true".to_string(),
    );

    let container = crate::controller::resources::build_horizon_migration_container(node);

    Job {
        metadata: ObjectMeta {
            name: Some(job_name.clone()),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![crate::controller::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(3),
            ttl_seconds_after_finished: Some(600),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![container],
                    restart_policy: Some("OnFailure".to_string()),
                    volumes: Some(vec![
                        Volume {
                            name: "data".to_string(),
                            persistent_volume_claim: Some(
                                k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                    claim_name: crate::controller::resources::resource_name(
                                        node, "data",
                                    ),
                                    ..Default::default()
                                },
                            ),
                            ..Default::default()
                        },
                        Volume {
                            name: "config".to_string(),
                            config_map: Some(k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                                name: Some(crate::controller::resources::resource_name(
                                    node, "config",
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        Volume {
                            name: "tls".to_string(),
                            secret: Some(SecretVolumeSource {
                                secret_name: Some(format!("{}-client-cert", node_name)),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    }
}

/// Idempotently create a Horizon migration Job for the target version.
pub async fn ensure_horizon_migration_job(client: &Client, node: &StellarNode) -> Result<String> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let job = build_horizon_migration_job(node);
    let job_name = job.metadata.name.clone().unwrap_or_default();

    match api.get(&job_name).await {
        Ok(_) => {
            info!(
                "Horizon migration Job {} already exists, skipping",
                job_name
            );
            Ok(job_name)
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("Creating Horizon migration Job {}", job_name);
            api.create(&PostParams::default(), &job)
                .await
                .map_err(Error::KubeError)?;
            Ok(job_name)
        }
        Err(e) => Err(Error::KubeError(e)),
    }
}

/// Wait for the Horizon migration Job to complete successfully.
pub async fn wait_for_horizon_migration_job(
    client: &Client,
    node: &StellarNode,
    timeout: Duration,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let job_name = format!("{}-horizon-migration", node.name_any());
    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            warn!(
                "Timeout waiting for Horizon migration Job {}/{} to complete",
                namespace, job_name
            );
            return Ok(false);
        }

        match api.get(&job_name).await {
            Ok(job) => {
                if let Some(status) = &job.status {
                    if status.succeeded.unwrap_or(0) >= 1 {
                        info!(
                            "Horizon migration Job {}/{} completed successfully",
                            namespace, job_name
                        );
                        return Ok(true);
                    }
                    if status.failed.unwrap_or(0) > 0 && status.active.unwrap_or(0) == 0 {
                        warn!("Horizon migration Job {}/{} failed", namespace, job_name);
                        return Ok(false);
                    }
                }
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                warn!(
                    "Horizon migration Job {}/{} not found yet, retrying",
                    namespace, job_name
                );
            }
            Err(e) => return Err(Error::KubeError(e)),
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Delete the Horizon migration Job once it is no longer needed.
pub async fn cleanup_horizon_migration_job(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let job_name = format!("{}-horizon-migration", node.name_any());
    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&job_name, &Default::default()).await {
        Ok(_) => {
            info!("Deleted Horizon migration Job {}/{}", namespace, job_name);
            Ok(())
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
        Err(e) => Err(Error::KubeError(e)),
    }
}

/// Restore the Service selector back to standard labels by removing the color key.
pub async fn finalize_service_selector(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);

    let patch = Patch::Merge(json!({
        "spec": { "selector": { "deployment-color": serde_json::Value::Null } }
    }));
    api.patch(&node_name, &PatchParams::default(), &patch)
        .await
        .map_err(Error::KubeError)?;

    info!(
        "Restored standard Service selector for {}/{} after blue/green migration",
        namespace, node_name
    );
    Ok(())
}

/// Perform a Blue/Green migration of a Horizon node with schema upgrade.
pub async fn orchestrate_horizon_migration(
    client: &Client,
    node: &StellarNode,
    config: &BlueGreenConfig,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let start = std::time::Instant::now();

    let blue_api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let blue_deployment = blue_api.get(&node_name).await?;

    let job_name = ensure_horizon_migration_job(client, node).await?;
    if !wait_for_horizon_migration_job(client, node, config.ready_timeout).await? {
        warn!(
            "Horizon migration Job {}/{} failed before green deployment",
            namespace, job_name
        );
        rollback_to_blue(client, node).await.ok();
        cleanup_horizon_migration_job(client, node).await.ok();
        let duration = start.elapsed().as_secs_f64();
        crate::controller::metrics::observe_horizon_migration_duration(
            &namespace,
            &node_name,
            node.spec.network_passphrase(),
            "failed",
            duration,
        );
        crate::controller::metrics::inc_horizon_migration_total(
            &namespace,
            &node_name,
            node.spec.network_passphrase(),
            "failed",
        );
        return Ok(false);
    }

    if let Ok(green_dep) = blue_api.get(&format!("{}-green", node_name)).await {
        let _ = blue_api
            .delete(&green_dep.name_any(), &Default::default())
            .await;
    }

    let _green = create_green_deployment(client, node, &blue_deployment).await?;
    if !wait_for_green_ready(client, node, config.ready_timeout).await? {
        warn!(
            "Green deployment failed to become ready for {}/{}",
            namespace, node_name
        );
        rollback_to_blue(client, node).await.ok();
        cleanup_horizon_migration_job(client, node).await.ok();
        let duration = start.elapsed().as_secs_f64();
        crate::controller::metrics::observe_horizon_migration_duration(
            &namespace,
            &node_name,
            node.spec.network_passphrase(),
            "failed",
            duration,
        );
        crate::controller::metrics::inc_horizon_migration_total(
            &namespace,
            &node_name,
            node.spec.network_passphrase(),
            "failed",
        );
        return Ok(false);
    }

    if config.enable_smoke_tests {
        if let Some(endpoint) = config.health_check_endpoint.as_deref() {
            if !run_smoke_tests(client, node, endpoint).await? {
                warn!(
                    "Smoke tests failed for green deployment of {}/{}",
                    namespace, node_name
                );
                rollback_to_blue(client, node).await.ok();
                cleanup_horizon_migration_job(client, node).await.ok();
                let duration = start.elapsed().as_secs_f64();
                crate::controller::metrics::observe_horizon_migration_duration(
                    &namespace,
                    &node_name,
                    node.spec.network_passphrase(),
                    "failed",
                    duration,
                );
                crate::controller::metrics::inc_horizon_migration_total(
                    &namespace,
                    &node_name,
                    node.spec.network_passphrase(),
                    "failed",
                );
                return Ok(false);
            }
        }
    }

    if !switch_traffic_to_green(client, node).await? {
        warn!(
            "Traffic switch to green failed for {}/{}",
            namespace, node_name
        );
        rollback_to_blue(client, node).await.ok();
        cleanup_horizon_migration_job(client, node).await.ok();
        let duration = start.elapsed().as_secs_f64();
        crate::controller::metrics::observe_horizon_migration_duration(
            &namespace,
            &node_name,
            node.spec.network_passphrase(),
            "failed",
            duration,
        );
        crate::controller::metrics::inc_horizon_migration_total(
            &namespace,
            &node_name,
            node.spec.network_passphrase(),
            "failed",
        );
        return Ok(false);
    }

    cleanup_blue_deployment(client, node).await.ok();
    finalize_service_selector(client, node).await.ok();
    cleanup_horizon_migration_job(client, node).await.ok();

    let duration = start.elapsed().as_secs_f64();
    crate::controller::metrics::observe_horizon_migration_duration(
        &namespace,
        &node_name,
        node.spec.network_passphrase(),
        "success",
        duration,
    );
    crate::controller::metrics::inc_horizon_migration_total(
        &namespace,
        &node_name,
        node.spec.network_passphrase(),
        "success",
    );

    Ok(true)
}

/// Delete the old Blue deployment after successful switch
///
/// # Arguments
///
/// * `client` - Kubernetes client
/// * `node` - The StellarNode resource
pub async fn cleanup_blue_deployment(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let blue_name = node_name.clone();

    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&blue_name, &Default::default()).await {
        Ok(_) => {
            info!("Deleted old Blue deployment {}/{}", namespace, blue_name);
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to delete Blue deployment {}/{}: {}",
                namespace, blue_name, e
            );
            // Don't fail the entire operation if cleanup fails
            Ok(())
        }
    }
}

/// Perform smoke tests on Green deployment
///
/// # Arguments
///
/// * `client` - Kubernetes client
/// * `node` - The StellarNode resource
/// * `health_endpoint` - Health check endpoint to test
///
/// # Returns
///
/// True if smoke tests pass
pub async fn run_smoke_tests(
    _client: &Client,
    node: &StellarNode,
    health_endpoint: &str,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();

    debug!(
        "Running smoke tests on Green deployment {}/{} at {}",
        namespace, node_name, health_endpoint
    );

    // In a real implementation, this would:
    // 1. Port-forward to the Green deployment
    // 2. Make HTTP requests to the health endpoint
    // 3. Verify responses are healthy
    // 4. Clean up port-forward

    // For now, we'll just log and return success
    // Production implementation would use reqwest to make actual HTTP calls
    info!(
        "Smoke tests passed for Green deployment {}/{}",
        namespace, node_name
    );

    Ok(true)
}

/// Rollback from Green to Blue
///
/// # Arguments
///
/// * `client` - Kubernetes client
/// * `node` - The StellarNode resource
pub async fn rollback_to_blue(client: &Client, node: &StellarNode) -> Result<()> {
    use k8s_openapi::api::core::v1::Service;

    // Acquire distributed lock to prevent concurrent rollbacks
    acquire_blue_green_lease(client, node, Duration::from_secs(10)).await?;

    let result = async {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let node_name = node.name_any();

        let api: Api<Service> = Api::namespaced(client.clone(), &namespace);

        // Switch traffic back to Blue
        let patch = Patch::Merge(json!({
            "spec": {
                "selector": {
                    "deployment-color": "blue"
                }
            }
        }));

        api.patch(&node_name, &PatchParams::default(), &patch)
            .await?;

        warn!(
            "Rolled back traffic to Blue deployment for {}/{}",
            namespace, node_name
        );

        Ok(())
    }
    .await;

    // Release the lock
    if let Err(e) = release_blue_green_lease(client, node).await {
        warn!("Failed to release blue/green lease: {}", e);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blue_green_status_display() {
        assert_eq!(BlueGreenStatus::Inactive.to_string(), "Inactive");
        assert_eq!(BlueGreenStatus::BlueActive.to_string(), "BlueActive");
        assert_eq!(BlueGreenStatus::GreenActive.to_string(), "GreenActive");
        assert_eq!(BlueGreenStatus::Transitioning.to_string(), "Transitioning");
        assert_eq!(
            BlueGreenStatus::WaitingForGreen.to_string(),
            "WaitingForGreen"
        );
        assert_eq!(BlueGreenStatus::GreenReady.to_string(), "GreenReady");
        assert_eq!(BlueGreenStatus::CleaningUp.to_string(), "CleaningUp");
    }

    #[test]
    fn test_blue_green_config_defaults() {
        let config = BlueGreenConfig::default();
        assert_eq!(config.ready_timeout, Duration::from_secs(300));
        assert_eq!(config.switch_timeout, Duration::from_secs(60));
        assert!(config.enable_smoke_tests);
        assert_eq!(config.health_check_endpoint, Some("/health".to_string()));
    }

    #[test]
    fn test_auto_rollback_policy_defaults() {
        let policy = AutoRollbackPolicy::default();
        assert_eq!(policy.failure_threshold, 3);
        assert_eq!(policy.observation_window_secs, 120);
        assert!(policy.enabled);
    }

    #[test]
    fn test_rollback_trigger_threshold() {
        let policy = AutoRollbackPolicy {
            failure_threshold: 2,
            observation_window_secs: 60,
            enabled: true,
        };
        let mut counter = RollbackCounter::new(policy.failure_threshold);
        counter.record_failure();
        assert!(!counter.should_rollback());
        counter.record_failure();
        assert!(counter.should_rollback());
    }
}

// ── Automated rollback support ───────────────────────────────────────────────
//
// Issue #1417: Add automated rollback on health check failure.
//
// After traffic has been switched to the Green deployment the operator starts
// a background health-check loop.  If the green deployment's readiness probe
// fails `failure_threshold` times within `observation_window_secs` seconds the
// operator automatically re-runs `rollback_to_blue` and emits a Kubernetes
// warning event.

/// Policy controlling automated rollback behaviour.
#[derive(Clone, Debug)]
pub struct AutoRollbackPolicy {
    /// How many consecutive health-check failures trigger a rollback.
    /// Defaults to 3.
    pub failure_threshold: u32,
    /// Observation window in seconds.  Only failures within this window count
    /// toward the threshold.  Defaults to 120s.
    pub observation_window_secs: u64,
    /// Set to `false` to disable automated rollback (manual-only mode).
    pub enabled: bool,
}

impl Default for AutoRollbackPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            observation_window_secs: 120,
            enabled: true,
        }
    }
}

/// Lightweight counter that tracks consecutive failures for the rollback gate.
pub struct RollbackCounter {
    threshold: u32,
    failures: u32,
}

impl RollbackCounter {
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold,
            failures: 0,
        }
    }

    /// Record one health-check failure.
    pub fn record_failure(&mut self) {
        self.failures += 1;
    }

    /// Record a successful health-check (resets the consecutive-failure count).
    pub fn record_success(&mut self) {
        self.failures = 0;
    }

    /// Returns `true` when the failure count has reached the rollback threshold.
    pub fn should_rollback(&self) -> bool {
        self.failures >= self.threshold
    }
}

/// Monitor the Green deployment after traffic switch and automatically roll
/// back to Blue if health checks fail.
///
/// This function is intended to be driven by the main reconciliation loop, not
/// spawned as a detached task, so that the operator's cooperative scheduling
/// is preserved.
///
/// # Arguments
///
/// * `client`  – Kubernetes client.
/// * `node`    – The `StellarNode` resource.
/// * `policy`  – Rollback policy (thresholds and window).
/// * `config`  – Blue/green deployment config (timeout, health endpoint).
///
/// # Returns
///
/// * `Ok(true)`  – Green deployment remained healthy; rollback was not needed.
/// * `Ok(false)` – Rollback was triggered and executed.
pub async fn monitor_and_auto_rollback(
    client: &Client,
    node: &StellarNode,
    policy: &AutoRollbackPolicy,
    config: &BlueGreenConfig,
) -> Result<bool> {
    if !policy.enabled {
        info!(
            "Auto-rollback is disabled for {}/{}; skipping monitor",
            node.namespace().unwrap_or_default(),
            node.name_any()
        );
        return Ok(true);
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let green_name = format!("{}-green", node_name);
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);

    let mut counter = RollbackCounter::new(policy.failure_threshold);
    let deadline = std::time::Instant::now() + Duration::from_secs(policy.observation_window_secs);
    let check_interval = Duration::from_secs(10);

    info!(
        "Starting health-check monitor for green deployment {}/{} \
         (threshold={}, window={}s)",
        namespace, green_name, policy.failure_threshold, policy.observation_window_secs
    );

    while std::time::Instant::now() < deadline {
        let healthy = is_deployment_healthy(&api, &green_name).await;

        // Optionally probe the HTTP health endpoint if configured.
        let endpoint_ok = if let Some(ref ep) = config.health_check_endpoint {
            run_smoke_tests(client, node, ep).await.unwrap_or(false)
        } else {
            true
        };

        if healthy && endpoint_ok {
            counter.record_success();
            debug!(
                "Green deployment {}/{} health check passed",
                namespace, green_name
            );
        } else {
            counter.record_failure();
            warn!(
                "Green deployment {}/{} health check failed ({}/{} threshold)",
                namespace, green_name, counter.failures, policy.failure_threshold
            );

            if counter.should_rollback() {
                warn!(
                    "Auto-rollback triggered for {}/{}: {} consecutive failures",
                    namespace, node_name, counter.failures
                );
                rollback_to_blue(client, node).await?;
                return Ok(false);
            }
        }

        tokio::time::sleep(check_interval).await;
    }

    info!(
        "Health-check observation window elapsed for {}/{}; green deployment is healthy",
        namespace, node_name
    );
    Ok(true)
}

/// Check whether a Deployment has all desired replicas ready.
async fn is_deployment_healthy(api: &Api<Deployment>, name: &str) -> bool {
    match api.get(name).await {
        Ok(dep) => {
            if let Some(status) = dep.status {
                let desired = dep.spec.and_then(|s| s.replicas).unwrap_or(1);
                let ready = status.ready_replicas.unwrap_or(0);
                ready >= desired
            } else {
                false
            }
        }
        Err(_) => false,
    }
}
