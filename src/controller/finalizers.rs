//! Finalizer handling for StellarNode cleanup
//!
//! Finalizers ensure that when a StellarNode is deleted:
//! 1. All dependent resources (Deployments, Services, ConfigMaps) are cleaned up
//! 2. Persistent Volumes/Claims are deleted based on retention policy
//! 3. External resources (cloud storage, DNS) are properly removed

use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::info;

use crate::crd::StellarNode;
use crate::error::Result;

/// Finalizer name used to protect StellarNode resources
///
/// This finalizer is added when a StellarNode is created and prevents
/// the resource from being deleted until cleanup is complete.
pub const STELLAR_NODE_FINALIZER: &str = "stellarnode.stellar.org/finalizer";

/// Add finalizer to a StellarNode if not present
///
/// Called during the Apply phase to ensure the finalizer is set.
/// The kube-rs `finalizer` helper handles this automatically, but
/// this function can be used for manual finalizer management.
#[allow(dead_code)]
pub async fn add_finalizer(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let finalizers: Vec<String> = node.finalizers().to_vec();
    if !finalizers.contains(&STELLAR_NODE_FINALIZER.to_string()) {
        let mut new_finalizers = finalizers;
        new_finalizers.push(STELLAR_NODE_FINALIZER.to_string());

        let patch = json!({
            "metadata": {
                "finalizers": new_finalizers
            }
        });
        api.patch(
            &node.name_any(),
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&patch),
        )
        .await?;
        info!("Added finalizer to StellarNode: {}", node.name_any());
    }
    Ok(())
}

/// Remove finalizer after cleanup is complete
///
/// Called after all resources have been cleaned up. Once the finalizer
/// is removed, Kubernetes will complete the deletion of the StellarNode.
#[allow(dead_code)]
pub async fn remove_finalizer(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let finalizers: Vec<String> = node
        .finalizers()
        .iter()
        .filter(|f| f.as_str() != STELLAR_NODE_FINALIZER)
        .cloned()
        .collect();

    let patch = json!({
        "metadata": {
            "finalizers": finalizers
        }
    });

    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    info!("Removed finalizer from StellarNode: {}", node.name_any());
    Ok(())
}

/// Check if the node is being deleted
///
/// A deletion timestamp indicates the user has requested deletion,
/// but finalizers are preventing the actual removal.
#[allow(dead_code)]
pub fn is_being_deleted(node: &StellarNode) -> bool {
    node.metadata.deletion_timestamp.is_some()
}

/// Check if the node has our finalizer
#[allow(dead_code)]
pub fn has_finalizer(node: &StellarNode) -> bool {
    node.finalizers()
        .iter()
        .any(|f| f == STELLAR_NODE_FINALIZER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        NodeType, ResourceRequirements, ResourceSpec, StellarNetwork, StellarNodeSpec,
        StorageConfig,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use kube::api::ObjectMeta;

    fn create_test_spec() -> StellarNodeSpec {
        StellarNodeSpec {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            history_mode: Default::default(),
            resources: ResourceRequirements {
                requests: ResourceSpec {
                    cpu: "500m".to_string(),
                    memory: "1Gi".to_string(),
                },
                limits: ResourceSpec {
                    cpu: "2".to_string(),
                    memory: "4Gi".to_string(),
                },
            },
            storage: StorageConfig {
                storage_class: "standard".to_string(),
                size: "100Gi".to_string(),
                retention_policy: Default::default(),
                annotations: None,
                ..Default::default()
            },
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
            pod_anti_affinity: Default::default(),
            placement: Default::default(),
            topology_spread_constraints: None,
            cve_handling: None,
            snapshot_schedule: None,
            restore_from_snapshot: None,
            read_replica_config: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            forensic_snapshot: None,
            label_propagation: None,
            read_pool_endpoint: None,
            resource_meta: None,
            sidecars: None,
            cert_manager: None,
            nat_traversal: None,
            custom_network_passphrase: None,
            cross_cloud_failover: None,
            hitless_upgrade: None,
            ..Default::default()
        }
    }

    #[test]
    fn test_finalizer_name() {
        assert_eq!(STELLAR_NODE_FINALIZER, "stellarnode.stellar.org/finalizer");
    }

    #[test]
    fn test_has_finalizer_returns_true_when_present() {
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                finalizers: Some(vec![STELLAR_NODE_FINALIZER.to_string()]),
                ..Default::default()
            },
            spec: create_test_spec(),
            status: None,
        };

        assert!(has_finalizer(&node), "Should detect finalizer when present");
    }

    #[test]
    fn test_has_finalizer_returns_false_when_absent() {
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                finalizers: Some(vec!["other.finalizer/test".to_string()]),
                ..Default::default()
            },
            spec: create_test_spec(),
            status: None,
        };

        assert!(
            !has_finalizer(&node),
            "Should not detect finalizer when absent"
        );
    }

    #[test]
    fn test_is_being_deleted_returns_true_when_deletion_timestamp_set() {
        use chrono::Utc;

        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                deletion_timestamp: Some(Time(Utc::now())),
                finalizers: Some(vec![STELLAR_NODE_FINALIZER.to_string()]),
                ..Default::default()
            },
            spec: create_test_spec(),
            status: None,
        };

        assert!(
            is_being_deleted(&node),
            "Should detect deletion when timestamp is set"
        );
    }

    #[test]
    fn test_is_being_deleted_returns_false_when_no_deletion_timestamp() {
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                deletion_timestamp: None,
                finalizers: Some(vec![STELLAR_NODE_FINALIZER.to_string()]),
                ..Default::default()
            },
            spec: create_test_spec(),
            status: None,
        };

        assert!(
            !is_being_deleted(&node),
            "Should not detect deletion when timestamp is absent"
        );
    }

    // -----------------------------------------------------------------------
    // PVC retention policy tests
    // -----------------------------------------------------------------------

    fn spec_with_retention(policy: crate::crd::types::RetentionPolicy) -> StellarNodeSpec {
        let mut spec = create_test_spec();
        spec.storage.retention_policy = policy;
        spec
    }

    #[test]
    fn test_should_delete_pvc_when_policy_is_delete() {
        let spec = spec_with_retention(crate::crd::types::RetentionPolicy::Delete);
        assert!(
            spec.should_delete_pvc(),
            "should_delete_pvc must return true when retention policy is Delete"
        );
    }

    #[test]
    fn test_should_not_delete_pvc_when_policy_is_retain() {
        let spec = spec_with_retention(crate::crd::types::RetentionPolicy::Retain);
        assert!(
            !spec.should_delete_pvc(),
            "should_delete_pvc must return false when retention policy is Retain"
        );
    }

    #[test]
    fn test_default_retention_policy_is_delete() {
        // The default StorageConfig uses RetentionPolicy::Delete, so PVCs are
        // cleaned up automatically unless the user explicitly opts into Retain.
        let spec = create_test_spec();
        assert!(
            spec.should_delete_pvc(),
            "default retention policy must be Delete"
        );
    }

    #[test]
    fn test_finalizer_present_on_node_with_delete_policy() {
        // A node with Delete policy must still carry the finalizer so the
        // operator has a chance to remove the PVC before the resource is gone.
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("validator-delete".to_string()),
                namespace: Some("default".to_string()),
                finalizers: Some(vec![STELLAR_NODE_FINALIZER.to_string()]),
                ..Default::default()
            },
            spec: spec_with_retention(crate::crd::types::RetentionPolicy::Delete),
            status: None,
        };

        assert!(has_finalizer(&node));
        assert!(node.spec.should_delete_pvc());
    }

    #[test]
    fn test_finalizer_present_on_node_with_retain_policy() {
        // A node with Retain policy also carries the finalizer; the operator
        // skips PVC deletion but still cleans up other resources.
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("validator-retain".to_string()),
                namespace: Some("default".to_string()),
                finalizers: Some(vec![STELLAR_NODE_FINALIZER.to_string()]),
                ..Default::default()
            },
            spec: spec_with_retention(crate::crd::types::RetentionPolicy::Retain),
            status: None,
        };

        assert!(has_finalizer(&node));
        assert!(!node.spec.should_delete_pvc());
    }

    #[test]
    fn test_retention_policy_roundtrip_delete() {
        let policy = crate::crd::types::RetentionPolicy::Delete;
        let json = serde_json::to_string(&policy).expect("serialize");
        let restored: crate::crd::types::RetentionPolicy =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(policy, restored);
    }

    #[test]
    fn test_retention_policy_roundtrip_retain() {
        let policy = crate::crd::types::RetentionPolicy::Retain;
        let json = serde_json::to_string(&policy).expect("serialize");
        let restored: crate::crd::types::RetentionPolicy =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(policy, restored);
    }
}
