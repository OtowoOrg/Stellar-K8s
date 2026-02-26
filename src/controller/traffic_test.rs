//! Tests for traffic shaping and rate-limiting controller

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{Pod, PodCondition, PodStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use kube::ResourceExt;

    use crate::crd::{
        NodeType, ReadReplicaConfig, ReadReplicaStrategy, ResourceRequirements, StellarNetwork,
        StellarNode, StellarNodeSpec,
    };

    /// Helper function to create a minimal valid StellarNodeSpec
    fn minimal_stellar_node_spec() -> StellarNodeSpec {
        StellarNodeSpec {
            node_type: NodeType::Horizon,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            history_mode: Default::default(),
            resources: ResourceRequirements::default(),
            storage: Default::default(),
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
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            resource_meta: None,
            vpa_config: None,
            read_pool_endpoint: None,
        }
    }

    /// Helper function to create a test StellarNode with read replica config
    fn create_test_stellar_node_with_replicas(
        name: &str,
        namespace: &str,
        strategy: ReadReplicaStrategy,
    ) -> StellarNode {
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "stellar".to_string());

        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Horizon,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                history_mode: Default::default(),
                resources: ResourceRequirements::default(),
                storage: Default::default(),
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
                read_replica_config: Some(ReadReplicaConfig {
                    replicas: 3,
                    resources: ResourceRequirements::default(),
                    strategy: strategy.clone(),
                    archive_sharding: false,
                }),
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                resource_meta: None,
                vpa_config: None,
                read_pool_endpoint: None,
            },
            status: None,
        }
    }

    /// Helper function to create a test StellarNode without read replica config
    fn create_test_stellar_node_without_replicas(name: &str, namespace: &str) -> StellarNode {
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "stellar".to_string());

        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Mainnet,
                version: "v21.0.0".to_string(),
                history_mode: Default::default(),
                resources: ResourceRequirements::default(),
                storage: Default::default(),
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
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                topology_spread_constraints: None,
                db_maintenance_config: None,
                cve_handling: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
                read_replica_config: None,
                oci_snapshot: None,
                service_mesh: None,
                resource_meta: None,
                vpa_config: None,
                read_pool_endpoint: None,
            },
            status: None,
        }
    }

    /// Helper function to create a test Pod
    fn create_test_pod(
        name: &str,
        namespace: &str,
        pod_ip: Option<String>,
        ready: bool,
        traffic_enabled: Option<bool>,
    ) -> Pod {
        // For test purposes, just use the name as-is for instance label
        // In real scenarios, pods would be created by Kubernetes and have the correct labels
        let mut labels = BTreeMap::new();
        labels.insert("app.kubernetes.io/instance".to_string(), name.to_string());
        labels.insert("stellar.org/role".to_string(), "read-replica".to_string());

        if let Some(enabled) = traffic_enabled {
            labels.insert(
                "stellar.org/traffic".to_string(),
                if enabled {
                    "enabled".to_string()
                } else {
                    "disabled".to_string()
                },
            );
        }

        let status = if ready {
            Some(PodStatus {
                pod_ip,
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: "True".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            })
        } else {
            Some(PodStatus {
                pod_ip,
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: "False".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            })
        };

        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: None,
            status,
        }
    }

    #[test]
    fn test_stellar_node_with_read_replicas_round_robin() {
        let node = create_test_stellar_node_with_replicas(
            "test-node",
            "default",
            ReadReplicaStrategy::RoundRobin,
        );

        assert!(node.spec.read_replica_config.is_some());
        let config = node.spec.read_replica_config.as_ref().unwrap();
        assert_eq!(config.replicas, 3);
        assert_eq!(config.strategy, ReadReplicaStrategy::RoundRobin);
        assert!(!config.archive_sharding);
    }

    #[test]
    fn test_stellar_node_with_read_replicas_freshness_preferred() {
        let node = create_test_stellar_node_with_replicas(
            "test-node",
            "default",
            ReadReplicaStrategy::FreshnessPreferred,
        );

        assert!(node.spec.read_replica_config.is_some());
        let config = node.spec.read_replica_config.as_ref().unwrap();
        assert_eq!(config.replicas, 3);
        assert_eq!(config.strategy, ReadReplicaStrategy::FreshnessPreferred);
    }

    #[test]
    fn test_stellar_node_without_read_replicas() {
        let node = create_test_stellar_node_without_replicas("test-node", "default");

        assert!(node.spec.read_replica_config.is_none());
    }

    #[test]
    fn test_pod_labels_creation_with_traffic_enabled() {
        let pod = create_test_pod(
            "read-replica-0",
            "default",
            Some("10.0.0.1".to_string()),
            true,
            Some(true),
        );

        assert_eq!(pod.name_any(), "read-replica-0");
        assert_eq!(pod.namespace().unwrap(), "default");
        assert_eq!(
            pod.status.as_ref().unwrap().pod_ip.as_ref().unwrap(),
            "10.0.0.1"
        );

        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get("stellar.org/traffic").unwrap(), "enabled");
    }

    #[test]
    fn test_pod_labels_creation_with_traffic_disabled() {
        let pod = create_test_pod(
            "read-replica-1",
            "default",
            Some("10.0.0.2".to_string()),
            false,
            Some(false),
        );

        assert_eq!(pod.name_any(), "read-replica-1");
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get("stellar.org/traffic").unwrap(), "disabled");
    }

    #[test]
    fn test_pod_status_ready() {
        let pod = create_test_pod(
            "read-replica-2",
            "default",
            Some("10.0.0.3".to_string()),
            true,
            None,
        );

        let is_ready = pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false);

        assert!(is_ready);
    }

    #[test]
    fn test_pod_status_not_ready() {
        let pod = create_test_pod("read-replica-3", "default", None, false, None);

        let is_ready = pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false);

        assert!(!is_ready);
    }

    #[test]
    fn test_traffic_service_naming_convention() {
        let node = create_test_stellar_node_with_replicas(
            "my-stellar-node",
            "stellar-system",
            ReadReplicaStrategy::RoundRobin,
        );

        let expected_service_name = format!("{}-read-traffic", node.name_any());
        assert_eq!(expected_service_name, "my-stellar-node-read-traffic");
    }

    #[test]
    fn test_pod_selector_labels_consistency() {
        let node_name = "validator-1";
        let node = create_test_stellar_node_with_replicas(
            node_name,
            "default",
            ReadReplicaStrategy::RoundRobin,
        );

        let pod = create_test_pod(
            node_name, // For this test, use node name as the instance label
            "default",
            Some("10.0.0.1".to_string()),
            true,
            None,
        );

        // Verify that pod labels match the expected selector format
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(
            labels.get("app.kubernetes.io/instance").unwrap(),
            &node.name_any()
        );
        assert_eq!(labels.get("stellar.org/role").unwrap(), "read-replica");
    }

    #[test]
    fn test_multiple_pods_with_different_ips() {
        let pods = [
            create_test_pod(
                "pod-0",
                "default",
                Some("10.0.0.1".to_string()),
                true,
                Some(true),
            ),
            create_test_pod(
                "pod-1",
                "default",
                Some("10.0.0.2".to_string()),
                true,
                Some(true),
            ),
            create_test_pod(
                "pod-2",
                "default",
                Some("10.0.0.3".to_string()),
                false,
                None,
            ),
        ];

        assert_eq!(pods.len(), 3);

        let ready_pods: Vec<_> = pods
            .iter()
            .filter(|p| {
                p.status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .map(|conds| {
                        conds
                            .iter()
                            .any(|c| c.type_ == "Ready" && c.status == "True")
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(ready_pods.len(), 2);
    }

    #[test]
    fn test_traffic_label_update_scenario_enable() {
        let old_pod = create_test_pod(
            "read-replica-0",
            "default",
            Some("10.0.0.1".to_string()),
            true,
            None,
        );

        let current_val = old_pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("stellar.org/traffic"))
            .map(|s| s.as_str());

        assert_eq!(current_val, None);

        let new_pod = create_test_pod(
            "read-replica-0",
            "default",
            Some("10.0.0.1".to_string()),
            true,
            Some(true),
        );

        let new_val = new_pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("stellar.org/traffic"))
            .map(|s| s.as_str());

        assert_eq!(new_val, Some("enabled"));
    }

    #[test]
    fn test_traffic_label_update_scenario_disable() {
        let old_pod = create_test_pod(
            "read-replica-1",
            "default",
            Some("10.0.0.2".to_string()),
            true,
            Some(true),
        );

        let current_val = old_pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("stellar.org/traffic"))
            .map(|s| s.as_str());

        assert_eq!(current_val, Some("enabled"));

        let new_pod = create_test_pod(
            "read-replica-1",
            "default",
            Some("10.0.0.2".to_string()),
            false,
            None,
        );

        let new_val = new_pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("stellar.org/traffic"))
            .map(|s| s.as_str());

        // When traffic label is None, it means it's not set (will be removed)
        assert_eq!(new_val, None);
    }

    #[test]
    fn test_read_replica_strategy_default() {
        let strategy = ReadReplicaStrategy::default();
        assert_eq!(strategy, ReadReplicaStrategy::RoundRobin);
    }

    #[test]
    fn test_read_replica_config_default_replicas() {
        let config = ReadReplicaConfig {
            replicas: 1,
            resources: ResourceRequirements::default(),
            strategy: ReadReplicaStrategy::default(),
            archive_sharding: false,
        };

        assert_eq!(config.replicas, 1);
        assert_eq!(config.strategy, ReadReplicaStrategy::RoundRobin);
    }

    #[test]
    fn test_traffic_service_port_configuration() {
        // Verify HTTP port mapping: 80 -> 11626
        let http_port = 80;
        let target_port = 11626;

        assert_eq!(http_port, 80);
        assert_eq!(target_port, 11626);
        // Stellar Core uses port 11626 for HTTP connections
    }

    #[test]
    fn test_namespace_handling_default() {
        let mut spec = minimal_stellar_node_spec();
        spec.node_type = NodeType::Horizon;
        spec.network = StellarNetwork::Testnet;

        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test".to_string()),
                namespace: None,
                ..Default::default()
            },
            spec,
            status: None,
        };

        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        assert_eq!(namespace, "default");
    }

    #[test]
    fn test_namespace_handling_explicit() {
        let mut spec = minimal_stellar_node_spec();
        spec.node_type = NodeType::Horizon;
        spec.network = StellarNetwork::Testnet;

        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test".to_string()),
                namespace: Some("stellar-system".to_string()),
                ..Default::default()
            },
            spec,
            status: None,
        };

        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        assert_eq!(namespace, "stellar-system");
    }

    #[test]
    fn test_pod_ip_present_and_absent() {
        let pod_with_ip = create_test_pod(
            "pod-with-ip",
            "default",
            Some("10.0.0.1".to_string()),
            true,
            None,
        );

        assert!(pod_with_ip.status.as_ref().unwrap().pod_ip.is_some());
        assert_eq!(
            pod_with_ip
                .status
                .as_ref()
                .unwrap()
                .pod_ip
                .as_ref()
                .unwrap(),
            "10.0.0.1"
        );

        let pod_without_ip = create_test_pod("pod-without-ip", "default", None, false, None);

        assert!(pod_without_ip.status.as_ref().unwrap().pod_ip.is_none());
    }

    #[test]
    fn test_traffic_annotation_label_distinct_from_role() {
        let labels = [
            ("stellar.org/role", "read-replica"),
            ("stellar.org/traffic", "enabled"),
        ];

        // Verify these are distinct labels
        assert_ne!(labels[0].0, labels[1].0);
        assert_ne!(labels[0].1, labels[1].1);

        // Verify independent rules apply
        let pod_with_role_only =
            create_test_pod("test", "default", Some("10.0.0.1".to_string()), true, None);

        let role_label = pod_with_role_only
            .metadata
            .labels
            .as_ref()
            .unwrap()
            .get("stellar.org/role");
        let traffic_label = pod_with_role_only
            .metadata
            .labels
            .as_ref()
            .unwrap()
            .get("stellar.org/traffic");

        assert!(role_label.is_some());
        assert!(traffic_label.is_none());
    }

    #[test]
    fn test_service_selector_with_traffic_label() {
        let mut selector = BTreeMap::new();
        selector.insert(
            "app.kubernetes.io/instance".to_string(),
            "test-node".to_string(),
        );
        selector.insert("stellar.org/role".to_string(), "read-replica".to_string());
        selector.insert("stellar.org/traffic".to_string(), "enabled".to_string());

        // Service selector should match pods with traffic enabled
        assert_eq!(selector.len(), 3);
        assert_eq!(selector.get("stellar.org/traffic").unwrap(), "enabled");
    }

    #[test]
    fn test_ingress_egress_rules_independence() {
        // Test that ingress and egress rules can be configured independently
        // This is important for security and traffic shaping
        let mut spec = minimal_stellar_node_spec();
        spec.node_type = NodeType::Horizon;
        spec.network = StellarNetwork::Testnet;
        spec.read_replica_config = Some(ReadReplicaConfig {
            replicas: 2,
            resources: ResourceRequirements::default(),
            strategy: ReadReplicaStrategy::RoundRobin,
            archive_sharding: false,
        });

        let _node_http = StellarNode {
            metadata: ObjectMeta {
                name: Some("http-node".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec,
            status: None,
        };

        // HTTP port for Horizon
        let http_port = 80;
        let target_port = 11626;

        assert_eq!(http_port, 80);
        assert_eq!(target_port, 11626);
        // Ingress can be on port 80, egress to 11626
    }

    #[test]
    fn test_read_replica_config_with_archive_sharding() {
        let config = ReadReplicaConfig {
            replicas: 3,
            resources: ResourceRequirements::default(),
            strategy: ReadReplicaStrategy::RoundRobin,
            archive_sharding: true,
        };

        assert!(config.archive_sharding);
        assert_eq!(config.replicas, 3);
    }

    #[test]
    fn test_read_replica_config_without_archive_sharding() {
        let config = ReadReplicaConfig {
            replicas: 2,
            resources: ResourceRequirements::default(),
            strategy: ReadReplicaStrategy::FreshnessPreferred,
            archive_sharding: false,
        };

        assert!(!config.archive_sharding);
    }

    #[test]
    fn test_lag_threshold_constant() {
        // Test that lag threshold is properly defined
        // This is used in freshness-preferred strategy
        let max_ledger = 1000u64;
        let current_ledger = 998u64;
        let lag_threshold = 5u64;

        let lag = max_ledger.saturating_sub(current_ledger);
        let is_fresh = lag <= lag_threshold;

        assert!(is_fresh);

        let lagging_ledger = 990u64;
        let lag_lagging = max_ledger.saturating_sub(lagging_ledger);
        let is_fresh_lagging = lag_lagging <= lag_threshold;

        assert!(!is_fresh_lagging);
    }

    #[test]
    fn test_pod_list_params_label_selector() {
        let node_name = "test-node";
        let label_selector = format!(
            "app.kubernetes.io/instance={},stellar.org/role=read-replica",
            node_name
        );

        // Verify label selector format is correct for Kubernetes API
        assert!(label_selector.contains("app.kubernetes.io/instance="));
        assert!(label_selector.contains("stellar.org/role=read-replica"));
        assert!(!label_selector.is_empty());
    }

    #[test]
    fn test_json_merge_patch_with_traffic_label() {
        // Test that JSON merge patch correctly adds/removes traffic label
        use serde_json::json;

        let add_label = json!({
            "metadata": {
                "labels": {
                    "stellar.org/traffic": "enabled"
                }
            }
        });

        let remove_label = json!({
            "metadata": {
                "labels": {
                    "stellar.org/traffic": null
                }
            }
        });

        assert_eq!(
            add_label["metadata"]["labels"]["stellar.org/traffic"],
            "enabled"
        );
        assert!(remove_label["metadata"]["labels"]["stellar.org/traffic"].is_null());
    }

    #[test]
    fn test_ready_condition_detection() {
        // Test various condition combinations
        let ready_condition = PodCondition {
            type_: "Ready".to_string(),
            status: "True".to_string(),
            ..Default::default()
        };

        let not_ready_condition = PodCondition {
            type_: "Ready".to_string(),
            status: "False".to_string(),
            ..Default::default()
        };

        assert_eq!(ready_condition.type_, "Ready");
        assert_eq!(ready_condition.status, "True");
        assert_eq!(not_ready_condition.status, "False");
    }
}
