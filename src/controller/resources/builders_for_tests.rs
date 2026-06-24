//! Test builder wrappers.

use super::prelude::*;
use super::helpers::*;

// ============================================================================
// Test helpers — thin wrappers that expose private builders for unit tests
// (Issue #298)
// ============================================================================

#[cfg(test)]
pub(crate) fn build_pdb_for_test(
    node: &StellarNode,
) -> Option<k8s_openapi::api::policy::v1::PodDisruptionBudget> {
    build_pdb(node)
}

#[cfg(test)]
pub(crate) fn build_pvc_for_test(
    node: &StellarNode,
    storage_class: String,
) -> k8s_openapi::api::core::v1::PersistentVolumeClaim {
    build_pvc(node, storage_class)
}

#[cfg(test)]
pub(crate) fn build_config_map_for_test(node: &StellarNode) -> ConfigMap {
    build_config_map(node, None, false)
}

#[cfg(test)]
pub(crate) fn build_deployment_for_test(
    node: &StellarNode,
) -> k8s_openapi::api::apps::v1::Deployment {
    build_deployment(node, false)
}

#[cfg(test)]
pub(crate) fn build_statefulset_for_test(
    node: &StellarNode,
) -> k8s_openapi::api::apps::v1::StatefulSet {
    build_statefulset(node, false, None)
}

#[cfg(test)]
pub(crate) fn build_service_for_test(node: &StellarNode) -> k8s_openapi::api::core::v1::Service {
    build_service(node, false)
}

#[cfg(test)]
mod ensure_pvc_tests {
    use super::{build_hpa, build_pvc, pvc_needs_update, resolve_pvc_storage_class};
    use crate::crd::{
        types::{ResourceRequirements, ResourceSpec, StorageMode},
        NodeType, StellarNetwork, StellarNode, StellarNodeSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn test_node() -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("stellar-system".to_string()),
                uid: Some("abc-123".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
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
                read_pool_endpoint: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                forensic_snapshot: None,
                label_propagation: None,
                resource_meta: None,
                sidecars: None,
                cert_manager: None,
                nat_traversal: None,
                custom_network_passphrase: None,
                cross_cloud_failover: None,
                hitless_upgrade: None,
                history_mode: Default::default(),
                storage: Default::default(),
                ..Default::default()
            },
            status: None,
        }
    }

    #[test]
    fn resolves_storage_class_with_explicit_value() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class = "fast-ssd".to_string();

        let resolved = resolve_pvc_storage_class(&node, true, true);
        assert_eq!(resolved, "fast-ssd");
    }

    #[test]
    fn resolves_storage_class_to_local_path_for_local_mode() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class.clear();

        let resolved = resolve_pvc_storage_class(&node, true, false);
        assert_eq!(resolved, "local-path");
    }

    #[test]
    fn resolves_storage_class_to_local_storage_when_path_missing() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class.clear();

        let resolved = resolve_pvc_storage_class(&node, false, true);
        assert_eq!(resolved, "local-storage");
    }

    #[test]
    fn resolves_storage_class_to_empty_when_no_local_class_found() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class.clear();

        let resolved = resolve_pvc_storage_class(&node, false, false);
        assert!(resolved.is_empty());
    }

    #[test]
    fn build_pvc_uses_resolved_storage_class() {
        let node = test_node();
        let pvc = build_pvc(&node, "gp3".to_string());

        assert_eq!(
            pvc.spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref()),
            Some("gp3")
        );
    }

    #[test]
    fn pvc_update_detects_storage_class_change() {
        let node = test_node();
        let existing = build_pvc(&node, "standard".to_string());
        let desired = build_pvc(&node, "gp3".to_string());

        assert!(pvc_needs_update(&existing, &desired));
    }

    #[test]
    fn pvc_update_skips_when_specs_match() {
        let node = test_node();
        let existing = build_pvc(&node, "standard".to_string());
        let desired = build_pvc(&node, "standard".to_string());

        assert!(!pvc_needs_update(&existing, &desired));
    }

    // -----------------------------------------------------------------------
    // Retention policy — Delete scenario
    // -----------------------------------------------------------------------

    #[test]
    fn should_delete_pvc_returns_true_for_delete_policy() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Delete;
        assert!(
            node.spec.should_delete_pvc(),
            "Delete policy must trigger PVC deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Retention policy — Retain scenario
    // -----------------------------------------------------------------------

    #[test]
    fn should_delete_pvc_returns_false_for_retain_policy() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Retain;
        assert!(
            !node.spec.should_delete_pvc(),
            "Retain policy must prevent PVC deletion"
        );
    }

    #[test]
    fn default_retention_policy_is_delete() {
        // StorageConfig::default() must use Delete so orphaned PVCs are
        // cleaned up unless the user explicitly opts into Retain.
        let node = test_node();
        assert!(
            node.spec.should_delete_pvc(),
            "default retention policy must be Delete"
        );
    }

    #[test]
    fn pvc_built_with_delete_policy_has_correct_storage_class() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Delete;
        let pvc = build_pvc(&node, "fast-ssd".to_string());
        assert_eq!(
            pvc.spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref()),
            Some("fast-ssd"),
            "PVC storage class must be preserved regardless of retention policy"
        );
    }

    #[test]
    fn pvc_built_with_retain_policy_has_correct_storage_class() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Retain;
        let pvc = build_pvc(&node, "standard".to_string());
        assert_eq!(
            pvc.spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref()),
            Some("standard"),
            "PVC storage class must be preserved regardless of retention policy"
        );
    }

    #[test]
    fn build_hpa_includes_supported_custom_metrics() {
        use crate::crd::types::AutoscalingConfig;

        let mut node = test_node();
        node.spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 1,
            max_replicas: 5,
            custom_metrics: vec![
                "stellar_horizon_tps".to_string(),
                "stellar_queue_length".to_string(),
            ],
            ..Default::default()
        });

        let hpa = build_hpa(&node).expect("HPA should build with supported custom metrics");
        let metrics = hpa
            .spec
            .as_ref()
            .and_then(|spec| spec.metrics.as_ref())
            .expect("HPA spec metrics should exist");

        let metric_names: Vec<String> = metrics
            .iter()
            .filter_map(|spec| {
                spec.object
                    .as_ref()
                    .map(|object| object.metric.name.clone())
            })
            .collect();

        assert!(metric_names.contains(&"stellar_horizon_tps".to_string()));
        assert!(metric_names.contains(&"stellar_horizon_queue_length".to_string()));
    }
}
