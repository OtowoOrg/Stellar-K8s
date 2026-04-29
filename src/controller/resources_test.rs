//! Unit tests for Kubernetes resource builders.
//!
//! Run with: `cargo test -p stellar-k8s resources_test`

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::TopologySpreadConstraint;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

    use crate::controller::resources::build_topology_spread_constraints;
    use crate::crd::{
        types::{HorizonConfig, PodAntiAffinityStrength, ResourceRequirements, ResourceSpec},
        NodeType, StellarNetwork, StellarNodeSpec,
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
    use crate::crd::types::ValidatorConfig;
    use crate::crd::StellarNode;
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
        assert_eq!(selector_labels.get("deployment-color"), Some(&"blue".to_string()));

        let pod_labels = spec
            .template
            .metadata
            .as_ref()
            .and_then(|m| m.labels.as_ref())
            .expect("pod labels must exist");
        assert_eq!(pod_labels.get("deployment-color"), Some(&"blue".to_string()));

        let init_containers = spec
            .template
            .spec
            .as_ref()
            .and_then(|ps| ps.init_containers.as_ref());
        assert!(init_containers.is_none(), "Blue/Green deployments should not use init container migrations");
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
            known_peers: Some(r#"["1.2.3.4:11625", "example.com:11625"]"#.to_string()),
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

        assert!(spec
            .policy_types
            .as_ref()
            .unwrap()
            .contains(&"Ingress".to_string()));
        assert!(spec
            .policy_types
            .as_ref()
            .unwrap()
            .contains(&"Egress".to_string()));

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
