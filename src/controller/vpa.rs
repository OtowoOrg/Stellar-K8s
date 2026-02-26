// ============================================================
// FILE: src/controller/vpa.rs   (new file — create this)
// ============================================================
//! Vertical Pod Autoscaler resource generation and lifecycle management.
//!
//! The VPA CRD lives in API group `autoscaling.k8s.io/v1`. Because
//! `k8s-openapi` does not ship VPA types we model the resource with plain
//! `serde_json::Value` and use `kube`'s dynamic/unstructured client helpers
//! (`DynamicObject` + `ApiResource`).
//!
//! # Lifecycle
//! - **Created** when `spec.vpaConfig` is first set.
//! - **Updated** (server-side apply) on every reconcile so that mode/policy
//!   changes are reflected immediately.
//! - **Deleted** when `spec.vpaConfig` is removed.

use kube::{
    api::{Api, ApiResource, DynamicObject, GroupVersionKind, Patch, PatchParams},
    core::ObjectMeta,
    Client, Resource, ResourceExt,
};
use serde_json::json;
use tracing::{debug, info};

use crate::crd::{
    types::{VpaConfig, VpaUpdateMode},
    NodeType, StellarNode,
};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const VPA_GROUP: &str = "autoscaling.k8s.io";
const VPA_VERSION: &str = "v1";
const VPA_KIND: &str = "VerticalPodAutoscaler";
const FIELD_MANAGER: &str = "stellar-operator";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the `ApiResource` descriptor used to build the dynamic VPA API.
fn vpa_api_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind {
        group: VPA_GROUP.to_string(),
        version: VPA_VERSION.to_string(),
        kind: VPA_KIND.to_string(),
    })
}

/// Derives the VPA name from the owning `StellarNode` name.
/// Convention: `<node-name>-vpa`
pub fn vpa_name(node: &StellarNode) -> String {
    format!("{}-vpa", node.name_any())
}

/// Returns the string that the VPA spec expects for `updateMode`.
fn update_mode_str(mode: &VpaUpdateMode) -> &'static str {
    match mode {
        VpaUpdateMode::Initial => "Initial",
        VpaUpdateMode::Auto => "Auto",
    }
}

/// Determines the target reference `kind` and `name` for the VPA based on
/// the node type (Validator → StatefulSet, everything else → Deployment).
fn target_ref(node: &StellarNode) -> (&'static str, String) {
    match node.spec.node_type {
        NodeType::Validator => ("StatefulSet", node.name_any()),
        NodeType::Horizon | NodeType::SorobanRpc => ("Deployment", node.name_any()),
    }
}

// ---------------------------------------------------------------------------
// Resource builder
// ---------------------------------------------------------------------------

/// Builds the full VPA `DynamicObject` ready to be server-side applied.
pub fn build_vpa(node: &StellarNode, config: &VpaConfig) -> DynamicObject {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = vpa_name(node);
    let (target_kind, target_name) = target_ref(node);
    let mode = update_mode_str(&config.update_mode);

    // Build container policies array (may be empty → VPA uses its defaults).
    let container_policies: Vec<serde_json::Value> = config
        .container_policies
        .iter()
        .map(|p| {
            let mut policy = json!({
                "containerName": p.container_name,
            });
            if let Some(min) = &p.min_allowed {
                policy["minAllowed"] = json!(min);
            }
            if let Some(max) = &p.max_allowed {
                policy["maxAllowed"] = json!(max);
            }
            policy
        })
        .collect();

    // Build the full spec.
    let spec = if container_policies.is_empty() {
        json!({
            "targetRef": {
                "apiVersion": "apps/v1",
                "kind":       target_kind,
                "name":       target_name,
            },
            "updatePolicy": {
                "updateMode": mode,
            },
        })
    } else {
        json!({
            "targetRef": {
                "apiVersion": "apps/v1",
                "kind":       target_kind,
                "name":       target_name,
            },
            "updatePolicy": {
                "updateMode": mode,
            },
            "resourcePolicy": {
                "containerPolicies": container_policies,
            },
        })
    };

    // Owner reference so the VPA is garbage-collected when the StellarNode
    // is deleted.
    let owner_ref = node.controller_owner_ref(&()).map(|mut r| {
        r.block_owner_deletion = Some(true);
        r
    });

    let mut obj = DynamicObject::new(&name, &vpa_api_resource());
    obj.metadata = ObjectMeta {
        name: Some(name.clone()),
        namespace: Some(namespace.clone()),
        labels: Some(
            [
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    FIELD_MANAGER.to_string(),
                ),
                ("app.kubernetes.io/instance".to_string(), node.name_any()),
                (
                    "stellar.org/node-type".to_string(),
                    format!("{:?}", node.spec.node_type).to_lowercase(),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        owner_references: owner_ref.map(|r| vec![r]),
        ..Default::default()
    };
    obj.data = spec;
    obj
}

// ---------------------------------------------------------------------------
// Reconcile helpers
// ---------------------------------------------------------------------------

/// Creates or updates the VPA resource for the given node.
///
/// Uses server-side apply so that the operator owns only the fields it sets.
pub async fn ensure_vpa(client: &Client, node: &StellarNode, config: &VpaConfig) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let ar = vpa_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &ar);

    let vpa = build_vpa(node, config);
    let name = vpa_name(node);

    let patch_params = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(&name, &patch_params, &Patch::Apply(&vpa))
        .await
        .map_err(|e| {
            // Surface a clear error if the VPA CRD is not installed.
            if let kube::Error::Api(ref ae) = e {
                if ae.code == 404 || ae.code == 422 {
                    return Error::ConfigError(format!(
                        "VPA CRD not installed or misconfigured ({}). \
                         Install the VPA controller before enabling vpaConfig.",
                        ae.message
                    ));
                }
            }
            Error::KubeError(e)
        })?;

    info!(
        "VPA ensured for StellarNode {}/{} (mode={:?})",
        namespace,
        node.name_any(),
        config.update_mode
    );
    Ok(())
}

/// Deletes the VPA resource if it exists, ignoring 404 errors.
pub async fn delete_vpa(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let ar = vpa_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &ar);
    let name = vpa_name(node);

    match api.delete(&name, &Default::default()).await {
        Ok(_) => {
            info!(
                "VPA deleted for StellarNode {}/{}",
                namespace,
                node.name_any()
            );
        }
        Err(kube::Error::Api(ref ae)) if ae.code == 404 => {
            debug!(
                "VPA {}/{} already absent, nothing to delete",
                namespace, name
            );
        }
        Err(e) => return Err(Error::KubeError(e)),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        types::{
            ResourceRequirements, StorageConfig, VpaConfig, VpaContainerPolicy, VpaUpdateMode,
        },
        NodeType, StellarNetwork, StellarNodeSpec,
    };
    use kube::core::ObjectMeta as KObjectMeta;

    // -----------------------------------------------------------------------
    // Helper: build a minimal StellarNode for testing
    // -----------------------------------------------------------------------
    fn make_node(name: &str, node_type: NodeType) -> StellarNode {
        StellarNode {
            metadata: KObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("stellar-test".to_string()),
                uid: Some("test-uid-1234".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                history_mode: Default::default(),
                resources: ResourceRequirements::default(),
                storage: StorageConfig::default(),
                validator_config: None,
                horizon_config: None,
                soroban_config: None,
                replicas: 1,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                topology_spread_constraints: None,
                cve_handling: None,
                read_replica_config: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                resource_meta: None,
                read_pool_endpoint: None,
            },
            status: None,
        }
    }

    // -----------------------------------------------------------------------
    // VPA name derivation
    // -----------------------------------------------------------------------

    #[test]
    fn test_vpa_name_convention() {
        let node = make_node("my-validator", NodeType::Validator);
        assert_eq!(vpa_name(&node), "my-validator-vpa");
    }

    // -----------------------------------------------------------------------
    // Target reference: Validator → StatefulSet, Horizon → Deployment
    // -----------------------------------------------------------------------

    #[test]
    fn test_target_ref_validator_is_statefulset() {
        let node = make_node("val-1", NodeType::Validator);
        let (kind, name) = target_ref(&node);
        assert_eq!(kind, "StatefulSet");
        assert_eq!(name, "val-1");
    }

    #[test]
    fn test_target_ref_horizon_is_deployment() {
        let node = make_node("horizon-1", NodeType::Horizon);
        let (kind, name) = target_ref(&node);
        assert_eq!(kind, "Deployment");
        assert_eq!(name, "horizon-1");
    }

    #[test]
    fn test_target_ref_soroban_is_deployment() {
        let node = make_node("soroban-1", NodeType::SorobanRpc);
        let (kind, name) = target_ref(&node);
        assert_eq!(kind, "Deployment");
        assert_eq!(name, "soroban-1");
    }

    // -----------------------------------------------------------------------
    // Update mode string mapping
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_mode_initial() {
        assert_eq!(update_mode_str(&VpaUpdateMode::Initial), "Initial");
    }

    #[test]
    fn test_update_mode_auto() {
        assert_eq!(update_mode_str(&VpaUpdateMode::Auto), "Auto");
    }

    // -----------------------------------------------------------------------
    // build_vpa – basic structure
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_vpa_basic_structure() {
        let node = make_node("horizon-node", NodeType::Horizon);
        let config = VpaConfig {
            update_mode: VpaUpdateMode::Initial,
            container_policies: vec![],
        };

        let vpa = build_vpa(&node, &config);

        // Name and namespace
        assert_eq!(vpa.metadata.name.as_deref(), Some("horizon-node-vpa"));
        assert_eq!(vpa.metadata.namespace.as_deref(), Some("stellar-test"));

        // targetRef should point to a Deployment
        let target = &vpa.data["targetRef"];
        assert_eq!(target["kind"], "Deployment");
        assert_eq!(target["name"], "horizon-node");
        assert_eq!(target["apiVersion"], "apps/v1");

        // updateMode
        assert_eq!(vpa.data["updatePolicy"]["updateMode"], "Initial");

        // No resourcePolicy when no policies given
        assert!(vpa.data.get("resourcePolicy").is_none());
    }

    #[test]
    fn test_build_vpa_auto_mode_for_validator() {
        let node = make_node("core-validator", NodeType::Validator);
        let config = VpaConfig {
            update_mode: VpaUpdateMode::Auto,
            container_policies: vec![],
        };

        let vpa = build_vpa(&node, &config);

        assert_eq!(vpa.data["updatePolicy"]["updateMode"], "Auto");
        assert_eq!(vpa.data["targetRef"]["kind"], "StatefulSet");
    }

    // -----------------------------------------------------------------------
    // build_vpa – container policies
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_vpa_with_container_policies() {
        let node = make_node("horizon-big", NodeType::Horizon);
        let config = VpaConfig {
            update_mode: VpaUpdateMode::Auto,
            container_policies: vec![VpaContainerPolicy {
                container_name: "stellar-horizon".to_string(),
                min_allowed: Some(
                    [
                        ("cpu".to_string(), "200m".to_string()),
                        ("memory".to_string(), "256Mi".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                max_allowed: Some(
                    [
                        ("cpu".to_string(), "4".to_string()),
                        ("memory".to_string(), "8Gi".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
            }],
        };

        let vpa = build_vpa(&node, &config);

        let policies = &vpa.data["resourcePolicy"]["containerPolicies"];
        assert!(policies.is_array());
        let first = &policies[0];
        assert_eq!(first["containerName"], "stellar-horizon");
        assert_eq!(first["minAllowed"]["cpu"], "200m");
        assert_eq!(first["maxAllowed"]["memory"], "8Gi");
    }

    #[test]
    fn test_build_vpa_multiple_container_policies() {
        let node = make_node("horizon-multi", NodeType::Horizon);
        let config = VpaConfig {
            update_mode: VpaUpdateMode::Initial,
            container_policies: vec![
                VpaContainerPolicy {
                    container_name: "stellar-horizon".to_string(),
                    min_allowed: None,
                    max_allowed: Some([("cpu".to_string(), "2".to_string())].into_iter().collect()),
                },
                VpaContainerPolicy {
                    container_name: "stellar-core".to_string(),
                    min_allowed: Some(
                        [("memory".to_string(), "512Mi".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    max_allowed: None,
                },
            ],
        };

        let vpa = build_vpa(&node, &config);
        let policies = &vpa.data["resourcePolicy"]["containerPolicies"];
        assert_eq!(policies.as_array().unwrap().len(), 2);
    }

    // -----------------------------------------------------------------------
    // build_vpa – labels
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_vpa_labels_present() {
        let node = make_node("val-test", NodeType::Validator);
        let config = VpaConfig {
            update_mode: VpaUpdateMode::Initial,
            container_policies: vec![],
        };
        let vpa = build_vpa(&node, &config);
        let labels = vpa
            .metadata
            .labels
            .as_ref()
            .expect("labels must be present");
        assert_eq!(
            labels
                .get("app.kubernetes.io/managed-by")
                .map(|s| s.as_str()),
            Some(FIELD_MANAGER)
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(|s| s.as_str()),
            Some("val-test")
        );
    }

    // -----------------------------------------------------------------------
    // VpaUpdateMode default
    // -----------------------------------------------------------------------

    #[test]
    fn test_vpa_update_mode_default_is_initial() {
        assert_eq!(VpaUpdateMode::default(), VpaUpdateMode::Initial);
    }
}
