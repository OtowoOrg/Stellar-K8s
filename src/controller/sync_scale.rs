//! In-place resource scaling for Stellar Core pods based on sync state.
//!
//! Uses the Kubernetes `InPlacePodVerticalScaling` feature (stable in 1.33,
//! beta in 1.27+) to update container CPU/memory limits and requests without
//! restarting the pod.  The patch targets the `stellar-node` container inside
//! the StatefulSet pod.
//!
//! # Restart-free guarantee
//!
//! Kubernetes only restarts a container when the resize policy requires it.
//! By default, CPU changes are applied without restart; memory changes may
//! require a restart depending on the kubelet version.  To avoid restarts
//! during critical sync phases, the operator sets `resizePolicy` to
//! `RestartNotRequired` for both CPU and memory on the managed container.
//! This is applied when the StatefulSet is first created (see `resources.rs`).
//!
//! If the cluster does not support in-place scaling (feature gate absent), the
//! patch will be silently ignored by the API server and the operator will log a
//! warning.  The node continues to run with its original resources.

use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::crd::{CoreSyncState, StellarNode, SyncStateScalingConfig};
use crate::error::{Error, Result};

const FIELD_MANAGER: &str = "stellar-operator";
/// Name of the main stellar-core container inside the pod.
const STELLAR_CORE_CONTAINER: &str = "stellar-node";

/// Apply the appropriate resource profile to all running pods of the node
/// based on the current `sync_state`.
///
/// - `CatchingUp` → apply `config.catching_up` resources
/// - `Synced`     → apply `config.synced` resources
/// - `Unknown`    → no-op (keep current resources)
///
/// Returns `Ok(true)` if a patch was applied, `Ok(false)` if skipped.
pub async fn reconcile_sync_scaling(
    client: &Client,
    node: &StellarNode,
    config: &SyncStateScalingConfig,
    sync_state: &CoreSyncState,
) -> Result<bool> {
    if !config.enabled {
        return Ok(false);
    }

    let profile = match sync_state {
        CoreSyncState::CatchingUp => &config.catching_up,
        CoreSyncState::Synced => &config.synced,
        CoreSyncState::Unknown => {
            debug!(
                "Sync state unknown for {}, skipping resource scaling",
                node.name_any()
            );
            return Ok(false);
        }
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let label_selector =
        format!("app.kubernetes.io/instance={name},app.kubernetes.io/name=stellar-node");

    let pods = pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await
        .map_err(Error::KubeError)?;

    if pods.items.is_empty() {
        debug!(
            "No pods found for {}/{}, skipping sync scaling",
            namespace, name
        );
        return Ok(false);
    }

    let patch_body = build_resource_patch(
        &profile.cpu_request,
        &profile.memory_request,
        &profile.cpu_limit,
        &profile.memory_limit,
    );

    let mut patched = false;
    for pod in &pods.items {
        let pod_name = pod.name_any();

        // Skip pods that already have the desired resources to avoid noisy patches.
        if pod_already_has_resources(pod, profile) {
            debug!(
                "Pod {}/{} already has {:?} resources, skipping patch",
                namespace, pod_name, sync_state
            );
            continue;
        }

        info!(
            "Applying {:?} resource profile to pod {}/{}: cpu={}/{} mem={}/{}",
            sync_state,
            namespace,
            pod_name,
            profile.cpu_request,
            profile.cpu_limit,
            profile.memory_request,
            profile.memory_limit,
        );

        match pod_api
            .patch(
                &pod_name,
                &PatchParams::apply(FIELD_MANAGER).force(),
                &Patch::Apply(&patch_body),
            )
            .await
        {
            Ok(_) => {
                patched = true;
            }
            Err(e) => {
                // Log but don't fail reconciliation — the cluster may not support
                // in-place scaling, or the pod may be terminating.
                warn!(
                    "In-place resource patch failed for pod {}/{}: {}. \
                     Ensure InPlacePodVerticalScaling feature gate is enabled.",
                    namespace, pod_name, e
                );
            }
        }
    }

    Ok(patched)
}

/// Build the SSA patch body that sets container resources on the pod spec.
fn build_resource_patch(
    cpu_req: &str,
    mem_req: &str,
    cpu_lim: &str,
    mem_lim: &str,
) -> serde_json::Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "spec": {
            "containers": [{
                "name": STELLAR_CORE_CONTAINER,
                "resources": {
                    "requests": {
                        "cpu": cpu_req,
                        "memory": mem_req,
                    },
                    "limits": {
                        "cpu": cpu_lim,
                        "memory": mem_lim,
                    }
                }
            }]
        }
    })
}

/// Check whether the pod's first container already matches the desired profile
/// to avoid redundant API calls.
fn pod_already_has_resources(pod: &Pod, profile: &crate::crd::SyncPhaseResources) -> bool {
    let container = pod.spec.as_ref().and_then(|s| {
        s.containers
            .iter()
            .find(|c| c.name == STELLAR_CORE_CONTAINER)
    });

    let Some(c) = container else {
        return false;
    };
    let Some(res) = &c.resources else {
        return false;
    };

    let req_cpu = res
        .requests
        .as_ref()
        .and_then(|r| r.get("cpu"))
        .map(|q| q.0.as_str())
        .unwrap_or("");
    let req_mem = res
        .requests
        .as_ref()
        .and_then(|r| r.get("memory"))
        .map(|q| q.0.as_str())
        .unwrap_or("");
    let lim_cpu = res
        .limits
        .as_ref()
        .and_then(|r| r.get("cpu"))
        .map(|q| q.0.as_str())
        .unwrap_or("");
    let lim_mem = res
        .limits
        .as_ref()
        .and_then(|r| r.get("memory"))
        .map(|q| q.0.as_str())
        .unwrap_or("");

    req_cpu == profile.cpu_request
        && req_mem == profile.memory_request
        && lim_cpu == profile.cpu_limit
        && lim_mem == profile.memory_limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::SyncPhaseResources;

    fn make_profile(
        cpu_req: &str,
        mem_req: &str,
        cpu_lim: &str,
        mem_lim: &str,
    ) -> SyncPhaseResources {
        SyncPhaseResources {
            cpu_request: cpu_req.to_string(),
            memory_request: mem_req.to_string(),
            cpu_limit: cpu_lim.to_string(),
            memory_limit: mem_lim.to_string(),
        }
    }

    #[test]
    fn test_build_resource_patch_shape() {
        let patch = build_resource_patch("4", "8Gi", "8", "16Gi");
        let containers = &patch["spec"]["containers"];
        assert_eq!(containers[0]["name"], "stellar-node");
        assert_eq!(containers[0]["resources"]["requests"]["cpu"], "4");
        assert_eq!(containers[0]["resources"]["limits"]["memory"], "16Gi");
    }

    #[test]
    fn test_pod_already_has_resources_empty_pod() {
        let pod = Pod::default();
        let profile = make_profile("500m", "2Gi", "2", "4Gi");
        // Empty pod → no container → returns false (will patch)
        assert!(!pod_already_has_resources(&pod, &profile));
    }
}
