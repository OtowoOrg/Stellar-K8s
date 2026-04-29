//! Monitors the Stellar Core sync state by querying the `/info` HTTP endpoint
//! on port 11626.
//!
//! The endpoint returns a JSON object whose `info.state` field contains one of:
//! - `"Synced!"` — node is fully caught up with the network
//! - `"Catching up"` — node is replaying historical ledgers (compute-intensive)
//! - `"Booting"` / `"Joining SCP"` / other — transitional states treated as Unknown
//!
//! # Usage
//!
//! ```rust,ignore
//! let state = query_core_sync_state("10.0.0.5").await?;
//! ```

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::{api::Api, Client, ResourceExt};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::crd::{CoreSyncState, NodeType, StellarNode};
use crate::error::{Error, Result};

/// Stellar Core `/info` response (only the fields we care about).
#[derive(Debug, Deserialize)]
struct CoreInfoResponse {
    info: CoreInfo,
}

#[derive(Debug, Deserialize)]
struct CoreInfo {
    state: String,
}

/// Query the stellar-core `/info` endpoint at `pod_ip:11626` and return the
/// parsed [`CoreSyncState`].
pub async fn query_core_sync_state(pod_ip: &str) -> Result<CoreSyncState> {
    let url = format!("http://{pod_ip}:11626/info");
    debug!("Querying stellar-core info endpoint: {}", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to build HTTP client: {e}")))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::ConfigError(format!("stellar-core /info unreachable: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::ConfigError(format!(
            "stellar-core /info returned HTTP {}",
            resp.status()
        )));
    }

    let info: CoreInfoResponse = resp
        .json()
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to parse /info response: {e}")))?;

    let state = parse_sync_state(&info.info.state);
    debug!("stellar-core state='{}' → {:?}", info.info.state, state);
    Ok(state)
}

/// Map the raw state string from stellar-core to a [`CoreSyncState`].
fn parse_sync_state(raw: &str) -> CoreSyncState {
    let lower = raw.to_lowercase();
    if lower.contains("synced") {
        CoreSyncState::Synced
    } else if lower.contains("catching") {
        CoreSyncState::CatchingUp
    } else {
        CoreSyncState::Unknown
    }
}

/// Resolve the sync state for a `StellarNode` by finding its first ready pod
/// and querying the stellar-core `/info` endpoint.
///
/// Returns `CoreSyncState::Unknown` if:
/// - The node is not a Validator
/// - No ready pod is found
/// - The endpoint is unreachable
pub async fn resolve_node_sync_state(client: &Client, node: &StellarNode) -> CoreSyncState {
    // Only Validators run stellar-core
    if node.spec.node_type != NodeType::Validator {
        return CoreSyncState::Unknown;
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let label_selector =
        format!("app.kubernetes.io/instance={name},app.kubernetes.io/name=stellar-node");

    let pods = match pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to list pods for {}/{}: {}", namespace, name, e);
            return CoreSyncState::Unknown;
        }
    };

    let pod_ip = pods
        .items
        .iter()
        .find(|p| is_pod_ready(p))
        .and_then(|p| p.status.as_ref())
        .and_then(|s| s.pod_ip.clone());

    match pod_ip {
        Some(ip) => match query_core_sync_state(&ip).await {
            Ok(state) => state,
            Err(e) => {
                warn!(
                    "Could not query sync state for {}/{}: {}",
                    namespace, name, e
                );
                CoreSyncState::Unknown
            }
        },
        None => {
            debug!(
                "No ready pod IP for {}/{}, sync state unknown",
                namespace, name
            );
            CoreSyncState::Unknown
        }
    }
}

fn is_pod_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|cs| cs.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sync_state() {
        assert_eq!(parse_sync_state("Synced!"), CoreSyncState::Synced);
        assert_eq!(parse_sync_state("synced!"), CoreSyncState::Synced);
        assert_eq!(parse_sync_state("Catching up"), CoreSyncState::CatchingUp);
        assert_eq!(parse_sync_state("catching up"), CoreSyncState::CatchingUp);
        assert_eq!(parse_sync_state("Booting"), CoreSyncState::Unknown);
        assert_eq!(parse_sync_state("Joining SCP"), CoreSyncState::Unknown);
        assert_eq!(parse_sync_state(""), CoreSyncState::Unknown);
    }
}
