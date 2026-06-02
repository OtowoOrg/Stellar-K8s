//! Unit tests for Kubernetes resource builders.
//!
//! Run with: `cargo test -p stellar-k8s resources_test`

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{ConfigMapVolumeSource, Volume, VolumeMount, TopologySpreadConstraint};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

    use crate::controller::resources::build_topology_spread_constraints;
    use crate::crd::{
        NodeType, StellarNetwork, StellarNodeSpec,
        types::{HorizonConfig, PodAntiAffinityStrength, ResourceRequirements, ResourceSpec},
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn minimal_spec(node_type: NodeType) -> StellarNodeSpec {
        StellarNodeSpec {
            node_type,
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
            replicas: 3,
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
            sidecars: None,
            cert_manager: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            forensic_snapshot: None,
            label_propagation: None,
            resource_meta: None,
            history_mode: Default::default(),
            storage: Default::default(),
            validator_config: None,
            horizon_config: None,
            soroban_config: None,
            nat_traversal: None,
            custom_network_passphrase: None,
            cross_cloud_failover: None,
            hitless_upgrade: None,
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // build_topology_spread_constraints — default behaviour
    // -----------------------------------------------------------------------

    #[test]
    fn test_defaults_returned_when_spec_is_none() {
        let spec = minimal_spec(NodeType::Validator);
        let constraints = build_topology_spread_constraints(&spec, "my-validator");

        // Should produce exactly 2 default constraints
        assert_eq!(constraints.len(), 2, "expected 2 default constraints");
    }

    #[test]
    fn test_default_includes_hostname_topology_key() {
        let spec = minimal_spec(NodeType::Horizon);
        let constraints = build_topology_spread_constraints(&spec, "my-horizon");

        let has_hostname = constraints
            .iter()
            .any(|c| c.topology_key == "kubernetes.io/hostname");
        assert!(
            has_hostname,
            "default constraints must include kubernetes.io/hostname"
        );
    }

    #[test]
    fn test_default_includes_zone_topology_key() {
        let spec = minimal_spec(NodeType::SorobanRpc);
        let constraints = build_topology_spread_constraints(&spec, "my-soroban");

        let has_zone = constraints
            .iter()
            .any(|c| c.topology_key == "topology.kubernetes.io/zone");
        assert!(
            has_zone,
            "default constraints must include topology.kubernetes.io/zone"
        );
    }

    #[test]
    fn test_default_max_skew_is_one() {
        let spec = minimal_spec(NodeType::Validator);
        let constraints = build_topology_spread_constraints(&spec, "val");

        for c in &constraints {
            assert_eq!(
                c.max_skew, 1,
                "default max_skew must be 1, got {}",
                c.max_skew
            );
        }
    }

    #[test]
    fn test_default_when_unsatisfiable_is_do_not_schedule() {
        let spec = minimal_spec(NodeType::Validator);
        let constraints = build_topology_spread_constraints(&spec, "val");

        for c in &constraints {
            assert_eq!(
                c.when_unsatisfiable, "DoNotSchedule",
                "default whenUnsatisfiable must be DoNotSchedule"
            );
        }
    }

    #[test]
    fn test_default_label_selector_matches_network_and_component() {
        let spec = minimal_spec(NodeType::Horizon);
        let constraints = build_topology_spread_constraints(&spec, "ignored-instance");

        for c in &constraints {
            let selector = c
                .label_selector
                .as_ref()
                .expect("label_selector must be set");
            let labels = selector
                .match_labels
                .as_ref()
                .expect("matchLabels must be set");
            assert_eq!(
                labels.get("app.kubernetes.io/name").map(|s| s.as_str()),
                Some("stellar-node"),
            );
            assert_eq!(
                labels.get("stellar-network").map(|s| s.as_str()),
                Some("testnet"),
            );
            assert_eq!(
                labels
                    .get("app.kubernetes.io/component")
                    .map(|s| s.as_str()),
                Some("horizon"),
            );
        }
    }

    #[test]
    fn test_soft_anti_affinity_uses_schedule_anyway_for_topology_spread() {
        let mut spec = minimal_spec(NodeType::Validator);
        spec.pod_anti_affinity = PodAntiAffinityStrength::Soft;
        let constraints = build_topology_spread_constraints(&spec, "val");
        for c in &constraints {
            assert_eq!(c.when_unsatisfiable, "ScheduleAnyway");
        }
    }

    // -----------------------------------------------------------------------
    // build_topology_spread_constraints — user-provided overrides
    // -----------------------------------------------------------------------

    #[test]
    fn test_user_provided_constraints_are_used_as_is() {
        let mut spec = minimal_spec(NodeType::Validator);
        spec.topology_spread_constraints = Some(vec![TopologySpreadConstraint {
            max_skew: 2,
            topology_key: "custom.io/rack".to_string(),
            when_unsatisfiable: "ScheduleAnyway".to_string(),
            label_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([("app".to_string(), "my-app".to_string())])),
                ..Default::default()
            }),
            ..Default::default()
        }]);

        let constraints = build_topology_spread_constraints(&spec, "val");

        assert_eq!(
            constraints.len(),
            1,
            "should use exactly the user-provided constraints"
        );
        assert_eq!(constraints[0].topology_key, "custom.io/rack");
        assert_eq!(constraints[0].max_skew, 2);
        assert_eq!(constraints[0].when_unsatisfiable, "ScheduleAnyway");
    }

    #[test]
    fn test_user_provided_multiple_constraints() {
        let mut spec = minimal_spec(NodeType::Validator);
        spec.topology_spread_constraints = Some(vec![
            TopologySpreadConstraint {
                max_skew: 1,
                topology_key: "kubernetes.io/hostname".to_string(),
                when_unsatisfiable: "DoNotSchedule".to_string(),
                label_selector: None,
                ..Default::default()
            },
            TopologySpreadConstraint {
                max_skew: 1,
                topology_key: "topology.kubernetes.io/zone".to_string(),
                when_unsatisfiable: "DoNotSchedule".to_string(),
                label_selector: None,
                ..Default::default()
            },
            TopologySpreadConstraint {
                max_skew: 2,
                topology_key: "topology.kubernetes.io/region".to_string(),
                when_unsatisfiable: "ScheduleAnyway".to_string(),
                label_selector: None,
                ..Default::default()
            },
        ]);

        let constraints = build_topology_spread_constraints(&spec, "val");
        assert_eq!(constraints.len(), 3);
    }

    #[test]
    fn test_empty_user_provided_vec_falls_back_to_defaults() {
        let mut spec = minimal_spec(NodeType::Validator);
        // Explicitly set to empty vec — should fall back to defaults
        spec.topology_spread_constraints = Some(vec![]);

        let constraints = build_topology_spread_constraints(&spec, "val");
        assert_eq!(
            constraints.len(),
            2,
            "empty user vec should fall back to 2 defaults"
        );
    }

    // -----------------------------------------------------------------------
    // Default constraints differ by node type
    // -----------------------------------------------------------------------

    #[test]
    fn test_validator_gets_default_constraints() {
        let spec = minimal_spec(NodeType::Validator);
        let constraints = build_topology_spread_constraints(&spec, "val");
        assert!(!constraints.is_empty());
    }

    #[test]
    fn test_horizon_gets_default_constraints() {
        let spec = minimal_spec(NodeType::Horizon);
        let constraints = build_topology_spread_constraints(&spec, "h");
        assert!(!constraints.is_empty());
    }

    #[test]
    fn test_soroban_gets_default_constraints() {
        let spec = minimal_spec(NodeType::SorobanRpc);
        let constraints = build_topology_spread_constraints(&spec, "s");
        assert!(!constraints.is_empty());
    }

    // -----------------------------------------------------------------------
    // Label selector contents
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_selector_has_node_type_label() {
        let spec = minimal_spec(NodeType::Validator);
        let constraints = build_topology_spread_constraints(&spec, "val");

        for c in &constraints {
            let labels = c
                .label_selector
                .as_ref()
                .and_then(|s| s.match_labels.as_ref())
                .expect("matchLabels must be present");
            assert!(
                labels.contains_key("app.kubernetes.io/name"),
                "selector must include app.kubernetes.io/name"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Issue #298 — standard labels and ownerReferences on all resource builders
    // -----------------------------------------------------------------------

    use crate::controller::resources::{
        build_config_map_for_test, build_deployment_for_test, build_network_policy,
        build_pvc_for_test, build_service_for_test, build_statefulset_for_test,
        merge_workload_affinity, owner_reference, standard_labels,
    };
    use crate::crd::StellarNode;
    use crate::crd::types::ValidatorConfig;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[test]
    fn test_scp_aware_anti_affinity_injection() {
        let mut node = make_node(NodeType::Validator);
        node.spec.placement.scp_aware_anti_affinity = true;
        node.spec.validator_config = Some(ValidatorConfig {
            seed_secret_ref: String::new(),
            seed_secret_source: None,
            quorum_set: Some(
                r#"
[VALIDATORS]
peer-1 = "G..."
peer-2 = "G..."
"#
                .to_string(),
            ),
            enable_history_archive: false,
            history_archive_urls: vec![],
            catchup_complete: false,
            key_source: Default::default(),
            kms_config: None,
            vl_source: None,
            hsm_config: None,
            ..Default::default()
        });

        let affinity = merge_workload_affinity(&node).expect("affinity should be generated");
        let pa = affinity
            .pod_anti_affinity
            .expect("podAntiAffinity should be generated");
        let preferred = pa
            .preferred_during_scheduling_ignored_during_execution
            .expect("preferred terms should be generated");

        assert_eq!(preferred.len(), 2);

        let instances: Vec<String> = preferred
            .iter()
            .filter_map(|t| {
                t.pod_affinity_term
                    .label_selector
                    .as_ref()?
                    .match_labels
                    .as_ref()?
                    .get("app.kubernetes.io/instance")
                    .cloned()
            })
            .collect();

        assert!(instances.contains(&"peer-1".to_string()));
        assert!(instances.contains(&"peer-2".to_string()));

        for t in preferred {
            assert_eq!(t.pod_affinity_term.topology_key, "kubernetes.io/hostname");
            assert_eq!(t.weight, 100);
        }
    }

    fn make_node(node_type: NodeType) -> StellarNode {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("stellar-system".to_string()),
                uid: Some("abc-123".to_string()),
                ..Default::default()
            },
            spec: minimal_spec(node_type),
            status: None,
        }
    }

    fn assert_standard_labels(meta: &ObjectMeta, node: &StellarNode) {
        let labels = meta.labels.as_ref().expect("labels must be set");
        assert_eq!(
            labels.get("app.kubernetes.io/name").map(|s| s.as_str()),
            Some("stellar-node"),
            "app.kubernetes.io/name must be 'stellar-node'"
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(|s| s.as_str()),
            Some(node.metadata.name.as_deref().unwrap_or("")),
            "app.kubernetes.io/instance must match node name"
        );
        assert_eq!(
            labels
                .get("app.kubernetes.io/managed-by")
                .map(|s| s.as_str()),
            Some("stellar-operator"),
            "app.kubernetes.io/managed-by must be 'stellar-operator'"
        );
        assert!(
            labels.contains_key("app.kubernetes.io/component"),
            "app.kubernetes.io/component must be set"
        );
    }

    fn assert_owner_reference(meta: &ObjectMeta, node: &StellarNode) {
        let refs = meta
            .owner_references
            .as_ref()
            .expect("ownerReferences must be set");
        assert_eq!(refs.len(), 1, "exactly one ownerReference expected");
        let oref = &refs[0];
        assert_eq!(
            oref.name,
            node.metadata.name.as_deref().unwrap_or(""),
            "ownerReference.name must match node name"
        );
        assert_eq!(
            oref.uid,
            node.metadata.uid.as_deref().unwrap_or(""),
            "ownerReference.uid must match node uid"
        );
        assert_eq!(
            oref.controller,
            Some(true),
            "ownerReference.controller must be true"
        );
        assert_eq!(
            oref.block_owner_deletion,
            Some(true),
            "ownerReference.blockOwnerDeletion must be true"
        );
    }

    #[test]
    fn test_pvc_has_standard_labels_and_owner_ref() {
        let node = make_node(NodeType::Validator);
        let pvc = build_pvc_for_test(&node, "standard".to_string());
        assert_standard_labels(&pvc.metadata, &node);
        assert_owner_reference(&pvc.metadata, &node);
    }

    #[test]
    fn test_config_map_has_standard_labels_and_owner_ref() {
        let node = make_node(NodeType::Validator);
        let cm = build_config_map_for_test(&node);
        assert_standard_labels(&cm.metadata, &node);
        assert_owner_reference(&cm.metadata, &node);
    }

    #[test]
    fn test_deployment_has_standard_labels_and_owner_ref() {
        let node = make_node(NodeType::Horizon);
        let deploy = build_deployment_for_test(&node);
        assert_standard_labels(&deploy.metadata, &node);
        assert_owner_reference(&deploy.metadata, &node);
    }

    #[test]
    fn test_horizon_blue_green_deployment_has_color_label_and_no_migration_init_container() {
        let mut node = make_node(NodeType::Horizon);
        node.spec.strategy.strategy_type = crate::crd::types::RolloutStrategyType::BlueGreen;
        node.spec.horizon_config = Some(HorizonConfig {
            database_secret_ref: "db-secret".to_string(),
            enable_ingest: true,
            stellar_core_url: "http://core:8000".to_string(),
            ingest_workers: 1,
            enable_experimental_ingestion: false,
            auto_migration: true,
        });

        let deploy = build_deployment_for_test(&node);
        let spec = deploy.spec.as_ref().expect("deployment spec must exist");
        let selector_labels = spec
            .selector
            .match_labels
            .as_ref()
            .expect("selector labels must exist");
        assert_eq!(
            selector_labels.get("deployment-color"),
            Some(&"blue".to_string())
        );

        let pod_labels = spec
            .template
            .metadata
            .as_ref()
            .and_then(|m| m.labels.as_ref())
            .expect("pod labels must exist");
        assert_eq!(
            pod_labels.get("deployment-color"),
            Some(&"blue".to_string())
        );

        let init_containers = spec
            .template
            .spec
            .as_ref()
            .and_then(|ps| ps.init_containers.as_ref());
        assert!(
            init_containers.is_none(),
            "Blue/Green deployments should not use init container migrations"
        );
    }

    #[test]
    fn test_statefulset_has_standard_labels_and_owner_ref() {
        let node = make_node(NodeType::Validator);
        let sts = build_statefulset_for_test(&node);
        assert_standard_labels(&sts.metadata, &node);
        assert_owner_reference(&sts.metadata, &node);
    }

    #[test]
    fn test_service_has_standard_labels_and_owner_ref() {
        let node = make_node(NodeType::Horizon);
        let svc = build_service_for_test(&node);
        assert_standard_labels(&svc.metadata, &node);
        assert_owner_reference(&svc.metadata, &node);
    }

    #[test]
    fn test_service_merges_custom_service_labels_and_annotations() {
        let mut node = make_node(NodeType::Horizon);
        node.spec.service_labels = Some(BTreeMap::from([
            ("team".to_string(), "infra".to_string()),
            ("app.kubernetes.io/managed-by".to_string(), "evil".to_string()),
        ]));
        node.spec.service_annotations = Some(BTreeMap::from([
            ("stellar.org/custom".to_string(), "${name}-service".to_string()),
        ]));

        let svc = build_service_for_test(&node);
        let labels = svc.metadata.labels.as_ref().expect("labels must exist");
        assert_eq!(labels.get("team"), Some(&"infra".to_string()));
        assert_eq!(labels.get("app.kubernetes.io/managed-by"), Some(&"stellar-operator".to_string()));

        let annotations = svc.metadata.annotations.as_ref().expect("annotations must exist");
        assert_eq!(annotations.get("stellar.org/custom"), Some(&"test-node-service".to_string()));
    }

    #[test]
    fn test_custom_volumes_and_volume_mounts_are_injected_into_pod_spec() {
        let mut node = make_node(NodeType::Horizon);
        node.spec.volumes = Some(vec![Volume {
            name: "custom-config".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: Some("my-config".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }]);
        node.spec.volume_mounts = Some(vec![VolumeMount {
            name: "custom-config".to_string(),
            mount_path: "/custom".to_string(),
            ..Default::default()
        }]);

        let deploy = build_deployment_for_test(&node);
        let pod_spec = deploy
            .spec
            .as_ref()
            .expect("deployment spec present")
            .template
            .spec
            .as_ref()
            .expect("pod spec present");

        assert!(pod_spec
            .volumes
            .as_ref()
            .expect("volumes present")
            .iter()
            .any(|v| v.name == "custom-config"));

        let main_container = pod_spec
            .containers
            .iter()
            .find(|c| c.name == "stellar-node")
            .expect("main container present");
        assert!(main_container
            .volume_mounts
            .as_ref()
            .expect("volume mounts present")
            .iter()
            .any(|m| m.name == "custom-config" && m.mount_path == "/custom"));
    }

    #[test]
    fn test_standard_labels_all_four_keys_present() {
        let node = make_node(NodeType::SorobanRpc);
        let labels = standard_labels(&node);
        for key in &[
            "app.kubernetes.io/name",
            "app.kubernetes.io/instance",
            "app.kubernetes.io/managed-by",
            "app.kubernetes.io/component",
        ] {
            assert!(
                labels.contains_key(*key),
                "standard_labels must contain '{key}'"
            );
        }
    }

    #[test]
    fn test_owner_reference_fields() {
        let node = make_node(NodeType::Validator);
        let oref = owner_reference(&node);
        assert_eq!(oref.name, "test-node");
        assert_eq!(oref.uid, "abc-123");
        assert_eq!(oref.controller, Some(true));
        assert_eq!(oref.block_owner_deletion, Some(true));
        assert!(!oref.api_version.is_empty());
        assert!(!oref.kind.is_empty());
    }

    #[test]
    fn test_validator_component_label() {
        let node = make_node(NodeType::Validator);
        let labels = standard_labels(&node);
        let component = labels
            .get("app.kubernetes.io/component")
            .expect("component label must be set");
        assert!(
            component.to_lowercase().contains("validator"),
            "component label should reflect validator type, got: {component}"
        );
    }

    #[test]
    fn test_horizon_component_label() {
        let node = make_node(NodeType::Horizon);
        let labels = standard_labels(&node);
        let component = labels
            .get("app.kubernetes.io/component")
            .expect("component label must be set");
        assert!(
            component.to_lowercase().contains("horizon"),
            "component label should reflect horizon type, got: {component}"
        );
    }

    // -----------------------------------------------------------------------
    // Sidecar injection tests (#507)
    // -----------------------------------------------------------------------

    use k8s_openapi::api::core::v1::{Container, VolumeMount};

    fn make_sidecar(name: &str) -> Container {
        Container {
            name: name.to_string(),
            image: Some(format!("example/{name}:latest")),
            ..Default::default()
        }
    }

    fn make_sidecar_with_volume_mount(name: &str, volume: &str, mount_path: &str) -> Container {
        Container {
            name: name.to_string(),
            image: Some(format!("example/{name}:latest")),
            volume_mounts: Some(vec![VolumeMount {
                name: volume.to_string(),
                mount_path: mount_path.to_string(),
                read_only: Some(true),
                ..Default::default()
            }]),
            ..Default::default()
        }
    }

    #[test]
    fn test_sidecar_injected_into_statefulset() {
        let mut node = make_node(NodeType::Validator);
        node.spec.sidecars = Some(vec![make_sidecar("log-forwarder")]);

        let sts = build_statefulset_for_test(&node);
        let containers = sts.spec.unwrap().template.spec.unwrap().containers;

        assert!(
            containers.iter().any(|c| c.name == "log-forwarder"),
            "sidecar 'log-forwarder' must be present in StatefulSet pod spec"
        );
    }

    #[test]
    fn test_sidecar_injected_into_deployment() {
        let mut node = make_node(NodeType::Horizon);
        node.spec.sidecars = Some(vec![make_sidecar("metrics-proxy")]);

        let deploy = build_deployment_for_test(&node);
        let containers = deploy.spec.unwrap().template.spec.unwrap().containers;

        assert!(
            containers.iter().any(|c| c.name == "metrics-proxy"),
            "sidecar 'metrics-proxy' must be present in Deployment pod spec"
        );
    }

    #[test]
    fn test_multiple_sidecars_all_injected() {
        let mut node = make_node(NodeType::Validator);
        node.spec.sidecars = Some(vec![
            make_sidecar("log-forwarder"),
            make_sidecar("metrics-proxy"),
            make_sidecar("custom-proxy"),
        ]);

        let sts = build_statefulset_for_test(&node);
        let containers = sts.spec.unwrap().template.spec.unwrap().containers;

        for name in &["log-forwarder", "metrics-proxy", "custom-proxy"] {
            assert!(
                containers.iter().any(|c| c.name.as_str() == *name),
                "sidecar '{name}' must be present in pod spec"
            );
        }
    }

    #[test]
    fn test_no_sidecars_does_not_add_extra_containers() {
        let node = make_node(NodeType::Validator);
        // sidecars is None by default in minimal_spec

        let sts = build_statefulset_for_test(&node);
        let containers = sts.spec.unwrap().template.spec.unwrap().containers;

        // Only the main stellar-node container should be present
        assert_eq!(
            containers.len(),
            1,
            "no sidecars configured — only the main container should be present"
        );
    }

    #[test]
    fn test_sidecar_can_mount_shared_data_volume() {
        let mut node = make_node(NodeType::Validator);
        node.spec.sidecars = Some(vec![make_sidecar_with_volume_mount(
            "log-forwarder",
            "data",
            "/stellar-data",
        )]);

        let sts = build_statefulset_for_test(&node);
        let pod_spec = sts.spec.unwrap().template.spec.unwrap();

        // The "data" volume must exist in the pod spec
        let volumes = pod_spec.volumes.expect("pod spec must have volumes");
        assert!(
            volumes.iter().any(|v| v.name == "data"),
            "shared 'data' volume must be defined in pod spec"
        );

        // The sidecar must reference it
        let sidecar = pod_spec
            .containers
            .iter()
            .find(|c| c.name == "log-forwarder")
            .expect("log-forwarder sidecar must be present");

        let mounts = sidecar
            .volume_mounts
            .as_ref()
            .expect("sidecar must have volume mounts");
        assert!(
            mounts.iter().any(|m| m.name == "data"),
            "sidecar must mount the 'data' volume"
        );
    }

    #[test]
    fn test_sidecar_can_mount_shared_config_volume() {
        let mut node = make_node(NodeType::Validator);
        node.spec.sidecars = Some(vec![make_sidecar_with_volume_mount(
            "config-watcher",
            "config",
            "/stellar-config",
        )]);

        let sts = build_statefulset_for_test(&node);
        let pod_spec = sts.spec.unwrap().template.spec.unwrap();

        let volumes = pod_spec.volumes.expect("pod spec must have volumes");
        assert!(
            volumes.iter().any(|v| v.name == "config"),
            "shared 'config' volume must be defined in pod spec"
        );

        let sidecar = pod_spec
            .containers
            .iter()
            .find(|c| c.name == "config-watcher")
            .expect("config-watcher sidecar must be present");

        let mounts = sidecar
            .volume_mounts
            .as_ref()
            .expect("sidecar must have volume mounts");
        assert!(
            mounts.iter().any(|m| m.name == "config"),
            "sidecar must mount the 'config' volume"
        );
    }

    #[test]
    fn test_main_container_is_first_in_pod_spec() {
        // The main stellar-node container must always be index 0 regardless of sidecars
        let mut node = make_node(NodeType::Validator);
        node.spec.sidecars = Some(vec![make_sidecar("log-forwarder")]);

        let sts = build_statefulset_for_test(&node);
        let containers = sts.spec.unwrap().template.spec.unwrap().containers;

        assert_ne!(
            containers[0].name, "log-forwarder",
            "main container must come before sidecars"
        );
        assert_eq!(
            containers.last().unwrap().name,
            "log-forwarder",
            "sidecar must be appended after the main container"
        );
    }
    #[test]
    fn test_network_policy_stellar_native_egress() {
        let mut node = make_node(NodeType::Validator);
        let vc = ValidatorConfig {
            known_peers: Some(
                r#"KNOWN_PEERS = ["1.2.3.4:11625", "example.com:11625"]"#.to_string(),
            ),
            quorum_set: Some(
                r#"[VALIDATORS]
"5.6.7.8" = "G..."
"G..." = "G..."
"#
                .to_string(),
            ),
            ..Default::default()
        };
        node.spec.validator_config = Some(vc);

        let config = crate::crd::types::NetworkPolicyConfig {
            enabled: true,
            ..Default::default()
        };

        let netpol = build_network_policy(&node, &config);
        let spec = netpol.spec.expect("spec must be present");

        assert!(
            spec.policy_types
                .as_ref()
                .unwrap()
                .contains(&"Ingress".to_string())
        );
        assert!(
            spec.policy_types
                .as_ref()
                .unwrap()
                .contains(&"Egress".to_string())
        );

        let egress = spec.egress.expect("egress rules must be present");

        // 1. DNS egress
        let has_dns = egress.iter().any(|rule| {
            rule.ports.as_ref().is_some_and(|ports| {
                ports.iter().any(|p| {
                    p.port.as_ref()
                        == Some(&k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53))
                })
            })
        });
        assert!(has_dns, "must have DNS egress rule");

        // 2. Peer egress
        let has_peers = egress.iter().any(|rule| {
            rule.to.as_ref().is_some_and(|to| {
                to.iter().any(|p| {
                    p.ip_block
                        .as_ref()
                        .is_some_and(|ip| ip.cidr == "1.2.3.4/32" || ip.cidr == "5.6.7.8/32")
                })
            })
        });
        assert!(
            has_peers,
            "must have peer egress rule for IPs 1.2.3.4 and 5.6.7.8"
        );
    }

    #[test]
    fn test_horizon_network_policy_allows_external_http_ingress() {
        let mut node = make_node(NodeType::Horizon);
        let config = crate::crd::types::NetworkPolicyConfig {
            enabled: true,
            ..Default::default()
        };

        let netpol = build_network_policy(&node, &config);
        let spec = netpol.spec.expect("spec must be present");
        let ingress = spec.ingress.expect("ingress rules must be present");

        let has_public_http = ingress.iter().any(|rule| {
            rule.from.is_none()
                && rule.ports.as_ref().is_some_and(|ports| {
                    ports.iter().any(|p| {
                        p.port.as_ref()
                            == Some(&k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8000))
                    })
                })
        });

        assert!(has_public_http, "Horizon must allow port 8000 ingress from external sources");
    }
}

// -----------------------------------------------------------------------
// apply_probe_override — #510 customizable probes
// -----------------------------------------------------------------------

#[test]
fn test_probe_override_none_returns_none_when_no_base() {
    let result = crate::controller::resources::apply_probe_override_pub(None, None);
    assert!(result.is_none());
}

#[test]
fn test_probe_override_returns_base_when_no_override() {
    use k8s_openapi::api::core::v1::Probe;
    let base = Probe {
        period_seconds: Some(10),
        ..Default::default()
    };
    let result = crate::controller::resources::apply_probe_override_pub(Some(base.clone()), None);
    assert_eq!(result, Some(base));
}

#[test]
fn test_probe_override_applies_all_fields() {
    use crate::crd::types::ProbeOverride;
    let cfg = ProbeOverride {
        initial_delay_seconds: Some(30),
        period_seconds: Some(15),
        timeout_seconds: Some(5),
        success_threshold: Some(1),
        failure_threshold: Some(6),
    };
    let result = crate::controller::resources::apply_probe_override_pub(None, Some(&cfg));
    let probe = result.expect("should produce a probe");
    assert_eq!(probe.initial_delay_seconds, Some(30));
    assert_eq!(probe.period_seconds, Some(15));
    assert_eq!(probe.timeout_seconds, Some(5));
    assert_eq!(probe.success_threshold, Some(1));
    assert_eq!(probe.failure_threshold, Some(6));
}

#[test]
fn test_probe_override_merges_onto_base() {
    use crate::crd::types::ProbeOverride;
    use k8s_openapi::api::core::v1::Probe;
    let base = Probe {
        period_seconds: Some(10),
        failure_threshold: Some(3),
        ..Default::default()
    };
    let cfg = ProbeOverride {
        failure_threshold: Some(10),
        ..Default::default()
    };
    let result = crate::controller::resources::apply_probe_override_pub(Some(base), Some(&cfg));
    let probe = result.expect("should produce a probe");
    assert_eq!(
        probe.period_seconds,
        Some(10),
        "base period_seconds preserved"
    );
    assert_eq!(
        probe.failure_threshold,
        Some(10),
        "override failure_threshold applied"
    );
}

#[test]
fn test_probe_config_validation_rejects_zero_period() {
    use crate::crd::types::{ProbeConfig, ProbeOverride};
    let cfg = ProbeConfig {
        liveness: Some(ProbeOverride {
            period_seconds: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let errs = cfg.validate();
    assert!(
        !errs.is_empty(),
        "zero periodSeconds should fail validation"
    );
    assert!(errs[0].contains("periodSeconds"));
}

#[test]
fn test_probe_config_validation_accepts_valid_config() {
    use crate::crd::types::{ProbeConfig, ProbeOverride};
    let cfg = ProbeConfig {
        liveness: Some(ProbeOverride {
            initial_delay_seconds: Some(0),
            period_seconds: Some(10),
            failure_threshold: Some(3),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(cfg.validate().is_empty());
}

// -----------------------------------------------------------------------
// init_containers injection tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod init_containers_tests {
    use k8s_openapi::api::core::v1::Container;

    use crate::controller::resources::{build_deployment_for_test, build_statefulset_for_test};
    use crate::crd::{
        NodeType, StellarNetwork, StellarNodeSpec,
        types::{ResourceRequirements, ResourceSpec, ValidatorConfig},
    };

    fn make_node(
        node_type: NodeType,
        init_containers: Option<Vec<Container>>,
    ) -> crate::crd::StellarNode {
        use kube::CustomResourceExt;
        let spec = StellarNodeSpec {
            node_type: node_type.clone(),
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
            replicas: 1,
            validator_config: if node_type == NodeType::Validator {
                Some(ValidatorConfig {
                    seed_secret_ref: "my-seed".to_string(),
                    ..Default::default()
                })
            } else {
                None
            },
            init_containers,
            ..Default::default()
        };

        let mut node = crate::crd::StellarNode::new("test-node", spec);
        node.metadata.namespace = Some("default".to_string());
        node
    }

    fn make_init_container(name: &str) -> Container {
        Container {
            name: name.to_string(),
            image: Some("busybox:latest".to_string()),
            command: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo hello".to_string(),
            ]),
            ..Default::default()
        }
    }

    // --- StatefulSet (Validator) tests ---

    #[test]
    fn test_no_user_init_containers_validator() {
        let node = make_node(NodeType::Validator, None);
        let sts = build_statefulset_for_test(&node);
        let init_containers = sts
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();
        // No user init containers; only operator-managed ones (none for this minimal spec)
        assert!(
            init_containers.iter().all(|c| c.name != "user-init"),
            "no user init containers should be present"
        );
    }

    #[test]
    fn test_single_user_init_container_appended_to_statefulset() {
        let user_init = make_init_container("fetch-config");
        let node = make_node(NodeType::Validator, Some(vec![user_init]));
        let sts = build_statefulset_for_test(&node);
        let init_containers = sts
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();

        let names: Vec<&str> = init_containers.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"fetch-config"),
            "user init container 'fetch-config' must be present, got: {:?}",
            names
        );
    }

    #[test]
    fn test_multiple_user_init_containers_all_appended_to_statefulset() {
        let containers = vec![
            make_init_container("step-one"),
            make_init_container("step-two"),
        ];
        let node = make_node(NodeType::Validator, Some(containers));
        let sts = build_statefulset_for_test(&node);
        let init_containers = sts
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();

        let names: Vec<&str> = init_containers.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"step-one"), "step-one must be present");
        assert!(names.contains(&"step-two"), "step-two must be present");
    }

    #[test]
    fn test_user_init_container_image_preserved_in_statefulset() {
        let mut container = make_init_container("restore-state");
        container.image = Some("my-registry/restore:v1.2.3".to_string());
        let node = make_node(NodeType::Validator, Some(vec![container]));
        let sts = build_statefulset_for_test(&node);
        let init_containers = sts
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();

        let found = init_containers
            .iter()
            .find(|c| c.name == "restore-state")
            .expect("restore-state init container must be present");
        assert_eq!(
            found.image.as_deref(),
            Some("my-registry/restore:v1.2.3"),
            "image must be preserved exactly"
        );
    }

    // --- Deployment (Horizon) tests ---

    #[test]
    fn test_single_user_init_container_appended_to_deployment() {
        let user_init = make_init_container("preflight-check");
        let node = make_node(NodeType::Horizon, Some(vec![user_init]));
        let dep = build_deployment_for_test(&node);
        let init_containers = dep
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();

        let names: Vec<&str> = init_containers.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"preflight-check"),
            "user init container 'preflight-check' must be present, got: {:?}",
            names
        );
    }

    #[test]
    fn test_no_user_init_containers_deployment() {
        let node = make_node(NodeType::Horizon, None);
        let dep = build_deployment_for_test(&node);
        let init_containers = dep
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();
        // No user init containers should be injected
        assert!(
            init_containers.iter().all(|c| c.name != "fetch-config"),
            "no user init containers should be present when spec.initContainers is None"
        );
    }

    #[test]
    fn test_user_init_container_order_preserved() {
        // User init containers must appear in the order specified
        let containers = vec![
            make_init_container("first"),
            make_init_container("second"),
            make_init_container("third"),
        ];
        let node = make_node(NodeType::Horizon, Some(containers));
        let dep = build_deployment_for_test(&node);
        let init_containers = dep
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();

        // Find the positions of the user containers
        let pos_first = init_containers.iter().position(|c| c.name == "first");
        let pos_second = init_containers.iter().position(|c| c.name == "second");
        let pos_third = init_containers.iter().position(|c| c.name == "third");

        assert!(pos_first.is_some(), "first must be present");
        assert!(pos_second.is_some(), "second must be present");
        assert!(pos_third.is_some(), "third must be present");
        assert!(
            pos_first < pos_second && pos_second < pos_third,
            "user init containers must appear in declaration order"
        );
    }

    #[test]
    fn test_user_init_containers_appended_after_operator_managed_ones() {
        // For Horizon with auto_migration, the operator injects a migration init container.
        // User init containers must come after it.
        use crate::crd::types::HorizonConfig;
        let user_init = make_init_container("my-custom-init");
        let spec = StellarNodeSpec {
            node_type: NodeType::Horizon,
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
            replicas: 1,
            horizon_config: Some(HorizonConfig {
                database_secret_ref: "db-secret".to_string(),
                auto_migration: true,
                ..Default::default()
            }),
            init_containers: Some(vec![user_init]),
            ..Default::default()
        };
        let mut node = crate::crd::StellarNode::new("test-node", spec);
        node.metadata.namespace = Some("default".to_string());

        let dep = build_deployment_for_test(&node);
        let init_containers = dep
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .init_containers
            .unwrap_or_default();

        let pos_migration = init_containers
            .iter()
            .position(|c| c.name == "horizon-migration");
        let pos_custom = init_containers
            .iter()
            .position(|c| c.name == "my-custom-init");

        assert!(
            pos_migration.is_some(),
            "operator migration init container must be present"
        );
        assert!(pos_custom.is_some(), "user init container must be present");
        assert!(
            pos_migration < pos_custom,
            "operator-managed init containers must come before user-defined ones"
        );
    }
}

// -----------------------------------------------------------------------
// diagnostic sidecar resource tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod diagnostic_sidecar_resource_tests {
    use k8s_openapi::api::core::v1::Container;

    use crate::controller::resources::{build_deployment_for_test, build_statefulset_for_test};
    use crate::crd::{
        NodeType, StellarNetwork, StellarNode, StellarNodeSpec,
        types::{ResourceRequirements, ResourceSpec, ValidatorConfig},
    };

    fn make_node(node_type: NodeType) -> StellarNode {
        let spec = StellarNodeSpec {
            node_type: node_type.clone(),
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

    use crate::controller::resources::{build_deployment_for_test, build_statefulset_for_test};
    use crate::crd::{
        NodeType, StellarNetwork, StellarNode, StellarNodeSpec,
        types::{ResourceRequirements, ResourceSpec, ValidatorConfig},
    };

    fn make_node(node_type: NodeType) -> StellarNode {
        let spec = StellarNodeSpec {
            node_type: node_type.clone(),
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
            replicas: 1,
            validator_config: if node_type == NodeType::Validator {
                Some(ValidatorConfig {
                    seed_secret_ref: "my-seed".to_string(),
                    ..Default::default()
                })
            } else {
                None
            },
            ..Default::default()
        };

        let mut node = StellarNode::new("test-node", spec);
        node.metadata.namespace = Some("default".to_string());
        node
    }

    fn health_sidecar(containers: &[Container]) -> &Container {
        containers
            .iter()
            .find(|container| container.name == "stellar-health-check")
            .expect("diagnostic sidecar must be present")
    }

    #[test]
    fn applies_default_diagnostic_sidecar_resources_to_statefulset() {
        let node = make_node(NodeType::Validator);
        let sts = build_statefulset_for_test(&node);
        let pod_spec = sts.spec.unwrap().template.spec.unwrap();
        let resources = health_sidecar(&pod_spec.containers)
            .resources
            .as_ref()
            .expect("diagnostic sidecar resources must be set");

        let requests = resources.requests.as_ref().expect("requests must be set");
        let limits = resources.limits.as_ref().expect("limits must be set");

        assert_eq!(requests.get("cpu").unwrap().0, "50m");
        assert_eq!(requests.get("memory").unwrap().0, "64Mi");
        assert_eq!(limits.get("cpu").unwrap().0, "50m");
        assert_eq!(limits.get("memory").unwrap().0, "64Mi");
    }

    #[test]
    fn applies_crd_override_diagnostic_sidecar_resources_to_deployment() {
        let mut node = make_node(NodeType::Horizon);
        node.spec.diagnostic_sidecar_resources = Some(ResourceRequirements {
            requests: ResourceSpec {
                cpu: "75m".to_string(),
                memory: "96Mi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "150m".to_string(),
                memory: "128Mi".to_string(),
            },
        });

        let deployment = build_deployment_for_test(&node);
        let pod_spec = deployment.spec.unwrap().template.spec.unwrap();
        let resources = health_sidecar(&pod_spec.containers)
            .resources
            .as_ref()
            .expect("diagnostic sidecar resources must be set");

        let requests = resources.requests.as_ref().expect("requests must be set");
        let limits = resources.limits.as_ref().expect("limits must be set");

        assert_eq!(requests.get("cpu").unwrap().0, "75m");
        assert_eq!(requests.get("memory").unwrap().0, "96Mi");
        assert_eq!(limits.get("cpu").unwrap().0, "150m");
        assert_eq!(limits.get("memory").unwrap().0, "128Mi");
    }
}

// -----------------------------------------------------------------------
// #704 — Advanced liveness/readiness probes for Stellar-Core
// -----------------------------------------------------------------------

#[cfg(test)]
mod advanced_probe_tests {
    use crate::controller::resources::build_statefulset_for_test;
    use crate::crd::{
        types::{ResourceRequirements, ResourceSpec},
        NodeType, StellarNetwork, StellarNode, StellarNodeSpec,
    };
    use kube::api::ObjectMeta;

    fn validator_node(name: &str) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                uid: Some("uid-probe-test".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                replicas: 1,
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
                ..Default::default()
            },
            status: None,
        }
    }

    /// Liveness probe for a Validator must use TCP socket on port 11625.
    /// This ensures the pod is only killed when the process is truly unresponsive,
    /// not merely syncing.
    #[test]
    fn test_validator_liveness_probe_is_tcp_socket() {
        let node = validator_node("v-liveness");
        let sts = build_statefulset_for_test(&node);
        let container = &sts.spec.unwrap().template.spec.unwrap().containers[0];
        let probe = container
            .liveness_probe
            .as_ref()
            .expect("liveness probe must be set");
        assert!(
            probe.tcp_socket.is_some(),
            "Validator liveness probe must be TCP socket (not HTTP), got: {:?}",
            probe
        );
        assert!(
            probe.http_get.is_none(),
            "Validator liveness probe must NOT be HTTP GET"
        );
        assert!(
            probe.exec.is_none(),
            "Validator liveness probe must NOT be exec"
        );
        let tcp = probe.tcp_socket.as_ref().unwrap();
        assert_eq!(
            tcp.port,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625),
            "Validator liveness probe must target port 11625"
        );
    }

    /// Readiness probe for a Validator must use an exec probe that queries /info
    /// and rejects CATCHING_UP / SYNCING states.
    #[test]
    fn test_validator_readiness_probe_is_exec_checking_info() {
        let node = validator_node("v-readiness");
        let sts = build_statefulset_for_test(&node);
        let container = &sts.spec.unwrap().template.spec.unwrap().containers[0];
        let probe = container
            .readiness_probe
            .as_ref()
            .expect("readiness probe must be set");
        assert!(
            probe.exec.is_some(),
            "Validator readiness probe must be exec (not HTTP GET), got: {:?}",
            probe
        );
        assert!(
            probe.http_get.is_none(),
            "Validator readiness probe must NOT be HTTP GET"
        );
        let exec = probe.exec.as_ref().unwrap();
        let cmd = exec.command.as_ref().expect("exec command must be set");
        let script = cmd.join(" ");
        assert!(
            script.contains("11626"),
            "readiness probe must query port 11626 (Stellar-Core HTTP)"
        );
        assert!(
            script.contains("CATCHING_UP"),
            "readiness probe must check for CATCHING_UP state"
        );
        assert!(
            script.contains("SYNCING"),
            "readiness probe must check for SYNCING state"
        );
    }

    /// A node in CATCHING_UP/SYNCING should be Not Ready (liveness still passes).
    /// This test verifies the probe script logic: the script must exit non-zero
    /// when the /info response contains a syncing state.
    #[test]
    fn test_readiness_script_rejects_catching_up_state() {
        let node = validator_node("v-sync-check");
        let sts = build_statefulset_for_test(&node);
        let container = &sts.spec.unwrap().template.spec.unwrap().containers[0];
        let probe = container.readiness_probe.as_ref().unwrap();
        let exec = probe.exec.as_ref().unwrap();
        let cmd = exec.command.as_ref().unwrap();
        // The script must use grep -qv (invert match) so that presence of
        // CATCHING_UP or SYNCING causes a non-zero exit.
        let script = cmd.join(" ");
        assert!(
            script.contains("grep -qv"),
            "script must use 'grep -qv' to invert-match syncing states: {}",
            script
        );
    }
}

// -----------------------------------------------------------------------
// #707 — PodDisruptionBudgets for Stellar-Core nodes
// -----------------------------------------------------------------------

#[cfg(test)]
mod pdb_tests {
    use crate::controller::resources::build_pdb_for_test;
    use crate::crd::{
        types::{ResourceRequirements, ResourceSpec},
        NodeType, StellarNetwork, StellarNode, StellarNodeSpec,
    };
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    use kube::api::ObjectMeta;

    fn node_with_replicas(node_type: NodeType, replicas: i32) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                uid: Some("uid-pdb-test".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                replicas,
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
                ..Default::default()
            },
            status: None,
        }
    }

    /// Validator with replicas=1 gets minAvailable=1 (edge case).
    #[test]
    fn test_validator_pdb_replicas_1_min_available_1() {
        let node = node_with_replicas(NodeType::Validator, 1);
        let pdb = build_pdb_for_test(&node).expect("PDB must be generated for Validator");
        let spec = pdb.spec.unwrap();
        assert_eq!(
            spec.min_available,
            Some(IntOrString::Int(1)),
            "replicas=1 Validator must have minAvailable=1"
        );
        assert!(spec.max_unavailable.is_none());
    }

    /// Validator with replicas=3 gets minAvailable=2 (quorum majority).
    #[test]
    fn test_validator_pdb_replicas_3_min_available_2() {
        let node = node_with_replicas(NodeType::Validator, 3);
        let pdb = build_pdb_for_test(&node).expect("PDB must be generated for Validator");
        let spec = pdb.spec.unwrap();
        assert_eq!(
            spec.min_available,
            Some(IntOrString::Int(2)),
            "replicas=3 Validator must have minAvailable=2"
        );
    }

    /// Validator with replicas=5 gets minAvailable=3.
    #[test]
    fn test_validator_pdb_replicas_5_min_available_3() {
        let node = node_with_replicas(NodeType::Validator, 5);
        let pdb = build_pdb_for_test(&node).expect("PDB must be generated for Validator");
        let spec = pdb.spec.unwrap();
        assert_eq!(spec.min_available, Some(IntOrString::Int(3)));
    }

    /// PDB owner reference points to the StellarNode CR for garbage collection.
    #[test]
    fn test_validator_pdb_has_owner_reference() {
        let node = node_with_replicas(NodeType::Validator, 3);
        let pdb = build_pdb_for_test(&node).expect("PDB must be generated");
        let owners = pdb.metadata.owner_references.expect("must have owner refs");
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].name, "test-node");
    }

    /// Non-Validator with replicas=1 returns None (no PDB needed).
    #[test]
    fn test_non_validator_single_replica_no_pdb() {
        let node = node_with_replicas(NodeType::Horizon, 1);
        assert!(
            build_pdb_for_test(&node).is_none(),
            "single-replica Horizon must not get a PDB"
        );
    }

    /// Non-Validator with replicas=3 gets default maxUnavailable=1.
    #[test]
    fn test_non_validator_multi_replica_default_pdb() {
        let node = node_with_replicas(NodeType::Horizon, 3);
        let pdb = build_pdb_for_test(&node).expect("PDB must be generated for multi-replica Horizon");
        let spec = pdb.spec.unwrap();
        assert_eq!(spec.max_unavailable, Some(IntOrString::Int(1)));
        assert!(spec.min_available.is_none());
    }
#[test]
fn test_validator_custom_env_overrides_defaults() {
    use k8s_openapi::api::core::v1::EnvVar;

    use crate::crd::types::{ResourceRequirements, ResourceSpec, ValidatorConfig};
    use crate::crd::{NodeType, StellarNetwork, StellarNodeSpec};

    let spec = StellarNodeSpec {
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
        replicas: 1,
        validator_config: Some(ValidatorConfig {
            seed_secret_ref: "my-seed".to_string(),
            ..Default::default()
        }),
        stellar_core_env: vec![
            EnvVar {
                name: "STELLAR_CORE_WORKER_THREADS".to_string(),
                value: Some("99".to_string()),
                ..Default::default()
            },
            EnvVar {
                name: "CUSTOM_CORE_FLAG".to_string(),
                value: Some("enabled".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let mut node = crate::crd::StellarNode::new("test", spec);
    node.metadata.namespace = Some("default".to_string());
    let sts = crate::controller::resources::build_statefulset_for_test(&node);
    let container = sts
        .spec
        .unwrap()
        .template
        .spec
        .unwrap()
        .containers
        .into_iter()
        .next()
        .unwrap();
    let env = container.env.unwrap_or_default();

    assert!(
        env.iter().any(|e| {
            e.name == "STELLAR_CORE_WORKER_THREADS" && e.value.as_deref() == Some("99")
        }),
        "custom env must override default STELLAR_CORE_WORKER_THREADS"
    );
    assert!(
        env.iter()
            .any(|e| e.name == "CUSTOM_CORE_FLAG" && e.value.as_deref() == Some("enabled")),
        "custom env must be appended for validator container"
    );
}

#[test]
fn test_horizon_custom_env_injected() {
    use k8s_openapi::api::core::v1::EnvVar;

    use crate::crd::types::{HorizonConfig, ResourceRequirements, ResourceSpec};
    use crate::crd::{NodeType, StellarNetwork, StellarNodeSpec};

    let spec = StellarNodeSpec {
        node_type: NodeType::Horizon,
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
        replicas: 1,
        horizon_config: Some(HorizonConfig {
            database_secret_ref: "db".to_string(),
            ..Default::default()
        }),
        horizon_env: vec![EnvVar {
            name: "HORIZON_LOG_LEVEL".to_string(),
            value: Some("debug".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut node = crate::crd::StellarNode::new("test", spec);
    node.metadata.namespace = Some("default".to_string());
    let dep = crate::controller::resources::build_deployment_for_test(&node);
    let container = dep
        .spec
        .unwrap()
        .template
        .spec
        .unwrap()
        .containers
        .into_iter()
        .next()
        .unwrap();
    let env = container.env.unwrap_or_default();

    assert!(
        env.iter()
            .any(|e| e.name == "HORIZON_LOG_LEVEL" && e.value.as_deref() == Some("debug")),
        "custom env must be injected for horizon container"
    );
}

#[test]
fn test_spec_and_jurisdiction_tolerations_are_applied() {
    use k8s_openapi::api::core::v1::Toleration;

    use crate::crd::types::{
        JurisdictionConfig, PlacementConfig, ResourceRequirements, ResourceSpec, ValidatorConfig,
    };
    use crate::crd::{NodeType, StellarNetwork, StellarNodeSpec};

    let spec = StellarNodeSpec {
        node_type: NodeType::Validator,
        network: StellarNetwork::Testnet,
        version: "v21.0.0".to_string(),
        resources: ResourceRequirements {
            requests: ResourceSpec {
                cpu: "500m".to_string(),
                memory: "1Gi".to_string(),
            },
            replicas: 1,
            validator_config: if node_type == NodeType::Validator {
                Some(ValidatorConfig {
                    seed_secret_ref: "my-seed".to_string(),
                    ..Default::default()
                })
            } else {
                None
            },
            ..Default::default()
        };

        let mut node = StellarNode::new("test-node", spec);
        node.metadata.namespace = Some("default".to_string());
        node
    }

    fn health_sidecar(containers: &[Container]) -> &Container {
        containers
            .iter()
            .find(|container| container.name == "stellar-health-check")
            .expect("diagnostic sidecar must be present")
    }

    #[test]
    fn applies_default_diagnostic_sidecar_resources_to_statefulset() {
        let node = make_node(NodeType::Validator);
        let sts = build_statefulset_for_test(&node);
        let pod_spec = sts.spec.unwrap().template.spec.unwrap();
        let resources = health_sidecar(&pod_spec.containers)
            .resources
            .as_ref()
            .expect("diagnostic sidecar resources must be set");

        let requests = resources.requests.as_ref().expect("requests must be set");
        let limits = resources.limits.as_ref().expect("limits must be set");

        assert_eq!(requests.get("cpu").unwrap().0, "50m");
        assert_eq!(requests.get("memory").unwrap().0, "64Mi");
        assert_eq!(limits.get("cpu").unwrap().0, "50m");
        assert_eq!(limits.get("memory").unwrap().0, "64Mi");
    }

    #[test]
    fn applies_crd_override_diagnostic_sidecar_resources_to_deployment() {
        let mut node = make_node(NodeType::Horizon);
        node.spec.diagnostic_sidecar_resources = Some(ResourceRequirements {
            requests: ResourceSpec {
                cpu: "75m".to_string(),
                memory: "96Mi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "150m".to_string(),
                memory: "128Mi".to_string(),
            },
        });

        let deployment = build_deployment_for_test(&node);
        let pod_spec = deployment.spec.unwrap().template.spec.unwrap();
        let resources = health_sidecar(&pod_spec.containers)
            .resources
            .as_ref()
            .expect("diagnostic sidecar resources must be set");

        let requests = resources.requests.as_ref().expect("requests must be set");
        let limits = resources.limits.as_ref().expect("limits must be set");

        assert_eq!(requests.get("cpu").unwrap().0, "75m");
        assert_eq!(requests.get("memory").unwrap().0, "96Mi");
        assert_eq!(limits.get("cpu").unwrap().0, "150m");
        assert_eq!(limits.get("memory").unwrap().0, "128Mi");
    }
        }),
        tolerations: vec![Toleration {
            key: Some("dedicated".to_string()),
            operator: Some("Equal".to_string()),
            value: Some("stellar".to_string()),
            effect: Some("NoSchedule".to_string()),
            ..Default::default()
        }],
        placement: PlacementConfig {
            jurisdiction: Some(JurisdictionConfig {
                code: "EU".to_string(),
                regions: vec!["eu-west-1".to_string()],
                label_key: "topology.kubernetes.io/region".to_string(),
                tolerations: vec![Toleration {
                    key: Some("jurisdiction".to_string()),
                    operator: Some("Equal".to_string()),
                    value: Some("EU".to_string()),
                    effect: Some("NoSchedule".to_string()),
                    ..Default::default()
                }],
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let mut node = crate::crd::StellarNode::new("test", spec);
    node.metadata.namespace = Some("default".to_string());
    let sts = crate::controller::resources::build_statefulset_for_test(&node);
    let pod_spec = sts.spec.unwrap().template.spec.unwrap();
    let tolerations = pod_spec.tolerations.unwrap_or_default();

    assert!(
        tolerations.iter().any(|t| {
            t.key.as_deref() == Some("dedicated") && t.value.as_deref() == Some("stellar")
        }),
        "spec tolerations must be propagated"
    );
    assert!(
        tolerations
            .iter()
            .any(|t| t.key.as_deref() == Some("jurisdiction") && t.value.as_deref() == Some("EU")),
        "jurisdiction tolerations must be merged"
    );
}
