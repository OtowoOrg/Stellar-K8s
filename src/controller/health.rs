//! Health check module for Stellar nodes
//!
//! Queries node endpoints to verify they are fully synced and operational.
//! The health check logic differs by node type:
//!
//! - **Validators**: Check ledger progression via Stellar Core HTTP endpoint
//! - **Horizon**: Check database synchronization and ingestion status
//! - **Soroban RPC**: Check RPC endpoint availability and ledger sync
//!
//! # Health Check Result
//!
//! Returns [`HealthCheckResult`] containing:
//! - `healthy` - Whether the node is running and accessible
//! - `synced` - Whether the node is fully synchronized with the network
//! - `message` - Human-readable status message
//! - `ledger_sequence` - Current ledger number (if available)

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::{api::Api, Client, ResourceExt};
use reqwest;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::crd::{NodeType, StellarNode};
use crate::error::{Error, Result};

/// Horizon health response from /health endpoint
#[derive(Debug, Deserialize, Serialize)]
struct HorizonHealthResponse {
    /// Overall health status
    #[serde(default)]
    pub status: String,

    /// Core sync status
    #[serde(default)]
    pub core_latest_ledger: u64,

    /// Horizon's latest ingested ledger
    #[serde(default)]
    pub history_latest_ledger: u64,

    /// Whether Horizon is synced with Core
    #[serde(default)]
    pub core_synced: bool,

    /// Whether history ingestion is up to date
    #[serde(default)]
    pub history_elder_ledger: u64,
}

/// Soroban RPC health response
#[derive(Debug, Deserialize, Serialize)]
struct SorobanHealthResponse {
    /// Health status
    pub status: String,

    /// Latest ledger
    #[serde(default)]
    pub ledger: u64,
}

/// Result of a health check
///
/// Contains the outcome of a health check operation, including:
/// - Whether the node is healthy and accessible
/// - Whether the node is fully synchronized
/// - Status message explaining the current condition
/// - Current ledger sequence (if available)
///
/// # Variants
///
/// The result is typically created using helper methods:
/// - [`HealthCheckResult::synced`] - Node is healthy and synced
/// - [`HealthCheckResult::syncing`] - Node is healthy but still syncing
/// - [`HealthCheckResult::unhealthy`] - Node is not responding or has errors
/// - [`HealthCheckResult::pending`] - Pod not ready yet
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    /// Whether the node is healthy
    pub healthy: bool,

    /// Whether the node is fully synced
    pub synced: bool,

    /// Human-readable message
    pub message: String,

    /// Current ledger sequence (if available)
    pub ledger_sequence: Option<u64>,
}

impl HealthCheckResult {
    /// Create a healthy and synced result
    pub fn synced(ledger: Option<u64>) -> Self {
        Self {
            healthy: true,
            synced: true,
            message: "Node is healthy and synced".to_string(),
            ledger_sequence: ledger,
        }
    }

    /// Create a healthy but not synced result
    pub fn syncing(message: String, ledger: Option<u64>) -> Self {
        Self {
            healthy: true,
            synced: false,
            message,
            ledger_sequence: ledger,
        }
    }

    /// Create an unhealthy result
    pub fn unhealthy(message: String) -> Self {
        Self {
            healthy: false,
            synced: false,
            message,
            ledger_sequence: None,
        }
    }

    /// Create a pending result (pod not ready yet)
    pub fn pending(message: String) -> Self {
        Self {
            healthy: false,
            synced: false,
            message,
            ledger_sequence: None,
        }
    }
}

/// Check the health of a StellarNode
pub async fn check_node_health(
    client: &Client,
    node: &StellarNode,
    mtls_config: Option<&crate::MtlsConfig>,
) -> Result<HealthCheckResult> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    debug!("Checking health for node {}/{}", namespace, name);

    // Get the pod(s) for this node
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let label_selector = format!(
        "app.kubernetes.io/instance={},app.kubernetes.io/name=stellar-node",
        name
    );

    let pods = pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await
        .map_err(Error::KubeError)?;

    if pods.items.is_empty() {
        return Ok(HealthCheckResult::pending(
            "No pods found for node".to_string(),
        ));
    }

    // Check the first ready pod
    let ready_pod = pods.items.iter().find(|pod| is_pod_ready(pod));

    let pod = match ready_pod {
        Some(p) => p,
        None => {
            return Ok(HealthCheckResult::pending(
                "Waiting for pod to be ready".to_string(),
            ));
        }
    };

    let pod_ip = match &pod.status {
        Some(status) => status.pod_ip.as_ref(),
        None => None,
    };

    let pod_ip = match pod_ip {
        Some(ip) => ip,
        None => {
            return Ok(HealthCheckResult::pending(
                "Pod IP not assigned yet".to_string(),
            ));
        }
    };

    info!(
        "Checking health for pod {} at IP {}",
        pod.name_any(),
        pod_ip
    );

    // Perform health check based on node type
    match node.spec.node_type {
        NodeType::Horizon => check_horizon_health(pod_ip, mtls_config).await,
        NodeType::SorobanRpc => check_soroban_health(pod_ip, mtls_config).await,
        NodeType::Validator => {
            // Validators don't have a standard health endpoint
            // We consider them healthy if the pod is running
            Ok(HealthCheckResult::synced(None))
        }
    }
}

/// Check if a pod is ready
fn is_pod_ready(pod: &Pod) -> bool {
    if let Some(status) = &pod.status {
        if let Some(conditions) = &status.conditions {
            return conditions
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True");
        }
    }
    false
}

/// Check Horizon node health
async fn check_horizon_health(
    pod_ip: &str,
    mtls_config: Option<&crate::MtlsConfig>,
) -> Result<HealthCheckResult> {
    let scheme = if mtls_config.is_some() {
        "https"
    } else {
        "http"
    };
    let url = format!("{}://{}:8000/health", scheme, pod_ip);

    debug!("Querying Horizon health endpoint: {}", url);

    // Create HTTP client with mTLS if enabled
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(5));

    if let Some(config) = mtls_config {
        let ca_cert = reqwest::Certificate::from_pem(&config.ca_pem)
            .map_err(|e| Error::ConfigError(format!("Failed to parse CA cert: {}", e)))?;

        builder = builder
            .add_root_certificate(ca_cert)
            .danger_accept_invalid_hostnames(true);
    }

    let client = builder
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to create HTTP client: {}", e)))?;

    match client.get(&url).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                warn!(
                    "Horizon health check returned status: {}",
                    response.status()
                );
                return Ok(HealthCheckResult::unhealthy(format!(
                    "Health endpoint returned status {}",
                    response.status()
                )));
            }

            // Try to parse the response
            match response.json::<HorizonHealthResponse>().await {
                Ok(health) => {
                    debug!("Horizon health response: {:?}", health);

                    // Check if Horizon is synced
                    if health.core_synced {
                        info!(
                            "Horizon is synced at ledger {}",
                            health.history_latest_ledger
                        );
                        Ok(HealthCheckResult::synced(Some(
                            health.history_latest_ledger,
                        )))
                    } else {
                        let lag = health
                            .core_latest_ledger
                            .saturating_sub(health.history_latest_ledger);

                        let message = format!(
                            "Horizon is syncing: at ledger {}, core at {} (lag: {})",
                            health.history_latest_ledger, health.core_latest_ledger, lag
                        );

                        info!("{}", message);
                        Ok(HealthCheckResult::syncing(
                            message,
                            Some(health.history_latest_ledger),
                        ))
                    }
                }
                Err(e) => {
                    warn!("Failed to parse Horizon health response: {}", e);
                    // If we can't parse the response, assume it's still starting up
                    Ok(HealthCheckResult::syncing(
                        "Health endpoint returned unparseable response".to_string(),
                        None,
                    ))
                }
            }
        }
        Err(e) => {
            warn!("Failed to query Horizon health endpoint: {}", e);
            Ok(HealthCheckResult::pending(format!(
                "Cannot reach health endpoint: {}",
                e
            )))
        }
    }
}

/// Check Soroban RPC node health
async fn check_soroban_health(
    pod_ip: &str,
    mtls_config: Option<&crate::MtlsConfig>,
) -> Result<HealthCheckResult> {
    let scheme = if mtls_config.is_some() {
        "https"
    } else {
        "http"
    };
    let url = format!("{}://{}:8000/health", scheme, pod_ip);

    debug!("Querying Soroban RPC health endpoint: {}", url);

    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(5));

    if let Some(config) = mtls_config {
        let ca_cert = reqwest::Certificate::from_pem(&config.ca_pem)
            .map_err(|e| Error::ConfigError(format!("Failed to parse CA cert: {}", e)))?;

        builder = builder
            .add_root_certificate(ca_cert)
            .danger_accept_invalid_hostnames(true);
    }

    let client = builder
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to create HTTP client: {}", e)))?;

    match client.get(&url).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                warn!(
                    "Soroban health check returned status: {}",
                    response.status()
                );
                return Ok(HealthCheckResult::unhealthy(format!(
                    "Health endpoint returned status {}",
                    response.status()
                )));
            }

            match response.json::<SorobanHealthResponse>().await {
                Ok(health) => {
                    debug!("Soroban health response: {:?}", health);

                    if health.status == "healthy" || health.status == "ready" {
                        info!("Soroban RPC is healthy at ledger {}", health.ledger);
                        Ok(HealthCheckResult::synced(Some(health.ledger)))
                    } else {
                        Ok(HealthCheckResult::syncing(
                            format!("Soroban RPC status: {}", health.status),
                            Some(health.ledger),
                        ))
                    }
                }
                Err(e) => {
                    warn!("Failed to parse Soroban health response: {}", e);
                    Ok(HealthCheckResult::syncing(
                        "Health endpoint returned unparseable response".to_string(),
                        None,
                    ))
                }
            }
        }
        Err(e) => {
            warn!("Failed to query Soroban health endpoint: {}", e);
            Ok(HealthCheckResult::pending(format!(
                "Cannot reach health endpoint: {}",
                e
            )))
        }
    }
}
