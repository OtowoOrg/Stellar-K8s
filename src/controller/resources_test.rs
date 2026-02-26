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
        types::{ResourceRequirements, ResourceSpec, StorageConfig},
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
            storage: StorageConfig::default(),
            validator_config: None,
            horizon_config: None,
            soroban_config: None,
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
            topology_spread_constraints: None,
            cve_handling: None,
            snapshot_schedule: None,
            restore_from_snapshot: None,
            read_replica_config: None,
            read_pool_endpoint: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            resource_meta: None,
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
    fn test_default_label_selector_matches_app_instance() {
        let spec = minimal_spec(NodeType::Horizon);
        let node_name = "my-horizon-node";
        let constraints = build_topology_spread_constraints(&spec, node_name);

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
                labels.get("app.kubernetes.io/instance").map(|s| s.as_str()),
                Some(node_name),
                "labelSelector must target the node instance"
            );
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
}
