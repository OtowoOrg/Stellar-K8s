#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{HorizonConfig, NodeType, SorobanConfig, StellarNetwork, StellarNode, StellarNodeSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn create_test_horizon_node() -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                annotations: Some(BTreeMap::from([
                    (MIGRATION_SOURCE_TYPE.to_string(), "Horizon".to_string()),
                ])),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Horizon,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                horizon_config: Some(HorizonConfig {
                    database_secret_ref: "test-db".to_string(),
                    enable_ingest: true,
                    stellar_core_url: "http://core:11626".to_string(),
                    ingest_workers: 2,
                    enable_experimental_ingestion: false,
                    auto_migration: true,
                }),
                ..Default::default()
            },
            status: None,
        }
    }

    fn create_test_soroban_node() -> StellarNode {
        let mut node = create_test_horizon_node();
        node.spec.node_type = NodeType::SorobanRpc;
        node.spec.horizon_config = None;
        node.spec.soroban_config = Some(SorobanConfig {
            stellar_core_url: "http://core:11626".to_string(),
            captive_core_config: None,
            captive_core_structured_config: None,
            enable_preflight: true,
            max_events_per_request: 10000,
        });
        node
    }

    #[test]
    fn test_migrate_config_preserves_core_url() {
        let horizon = HorizonConfig {
            database_secret_ref: "test".to_string(),
            enable_ingest: true,
            stellar_core_url: "http://custom-core:11626".to_string(),
            ingest_workers: 4,
            enable_experimental_ingestion: true,
            auto_migration: true,
        };

        let soroban = migrate_config(&horizon);
        assert_eq!(soroban.stellar_core_url, "http://custom-core:11626");
        assert!(soroban.enable_preflight);
        assert_eq!(soroban.max_events_per_request, 10000);
    }

    #[test]
    fn test_migration_annotation_detection() {
        let node = create_test_soroban_node();
        
        let source_type = node
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(MIGRATION_SOURCE_TYPE));
        
        assert_eq!(source_type, Some(&"Horizon".to_string()));
    }

    #[test]
    fn test_node_type_change_detection() {
        let horizon_node = create_test_horizon_node();
        let soroban_node = create_test_soroban_node();

        assert_eq!(horizon_node.spec.node_type, NodeType::Horizon);
        assert_eq!(soroban_node.spec.node_type, NodeType::SorobanRpc);
        assert!(horizon_node.spec.horizon_config.is_some());
        assert!(soroban_node.spec.soroban_config.is_some());
    }
}
