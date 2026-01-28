//! Unit tests for StellarNodeSpec validation
//!
//! Tests the `StellarNodeSpec::validate()` function to ensure it correctly
//! accepts valid configurations and rejects invalid ones.

#[cfg(test)]
mod stellar_node_spec_validation {
    use crate::crd::{
        AutoscalingConfig, HorizonConfig, IngressConfig, IngressHost, IngressPath, NodeType,
        ResourceRequirements, ResourceSpec, SorobanConfig, SpecValidationError, StellarNetwork,
        StellarNodeSpec, StorageConfig, ValidatorConfig,
    };

    /// Helper to create a minimal valid StellarNodeSpec for a Validator
    fn valid_validator_spec() -> StellarNodeSpec {
        StellarNodeSpec {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            resources: default_resources(),
            storage: default_storage(),
            validator_config: Some(ValidatorConfig {
                seed_secret_ref: "validator-seed".to_string(),
                quorum_set: None,
                enable_history_archive: false,
                history_archive_urls: vec![],
                catchup_complete: false,
                key_source: Default::default(),
                kms_config: None,
                vl_source: None,
                hsm_config: None,
            }),
            horizon_config: None,
            soroban_config: None,
            replicas: 1,
            min_available: None,
            max_unavailable: None,
            suspended: false,
            alerting: false,
            database: None,
            autoscaling: None,
            ingress: None,
            strategy: Default::default(),
            maintenance_mode: false,
            network_policy: None,
            dr_config: None,
            topology_spread_constraints: None,
            resource_meta: None,
        }
    }

    /// Helper to create a minimal valid StellarNodeSpec for Horizon
    fn valid_horizon_spec() -> StellarNodeSpec {
        StellarNodeSpec {
            node_type: NodeType::Horizon,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            resources: default_resources(),
            storage: default_storage(),
            validator_config: None,
            horizon_config: Some(HorizonConfig {
                database_secret_ref: "horizon-db".to_string(),
                enable_ingest: true,
                stellar_core_url: "http://stellar-core:11626".to_string(),
                ingest_workers: 1,
                enable_experimental_ingestion: false,
                auto_migration: false,
            }),
            soroban_config: None,
            replicas: 2,
            min_available: None,
            max_unavailable: None,
            suspended: false,
            alerting: false,
            database: None,
            autoscaling: None,
            ingress: None,
            strategy: Default::default(),
            maintenance_mode: false,
            network_policy: None,
            dr_config: None,
            topology_spread_constraints: None,
            resource_meta: None,
        }
    }

    /// Helper to create a minimal valid StellarNodeSpec for SorobanRpc
    fn valid_soroban_spec() -> StellarNodeSpec {
        StellarNodeSpec {
            node_type: NodeType::SorobanRpc,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            resources: default_resources(),
            storage: default_storage(),
            validator_config: None,
            horizon_config: None,
            soroban_config: Some(SorobanConfig {
                stellar_core_url: "http://stellar-core:11626".to_string(),
                captive_core_config: None,
                enable_preflight: true,
                max_events_per_request: 10000,
            }),
            replicas: 2,
            min_available: None,
            max_unavailable: None,
            suspended: false,
            alerting: false,
            database: None,
            autoscaling: None,
            ingress: None,
            strategy: Default::default(),
            maintenance_mode: false,
            network_policy: None,
            dr_config: None,
            topology_spread_constraints: None,
            resource_meta: None,
        }
    }

    fn default_resources() -> ResourceRequirements {
        ResourceRequirements {
            requests: ResourceSpec {
                cpu: "500m".to_string(),
                memory: "1Gi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "2".to_string(),
                memory: "4Gi".to_string(),
            },
        }
    }

    fn default_storage() -> StorageConfig {
        StorageConfig {
            storage_class: "standard".to_string(),
            size: "100Gi".to_string(),
            retention_policy: Default::default(),
            annotations: None,
        }
    }

    // =========================================================================
    // Validator Node Tests
    // =========================================================================

    #[test]
    fn test_valid_validator_passes_validation() {
        let spec = valid_validator_spec();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_validator_missing_config_fails() {
        let mut spec = valid_validator_spec();
        spec.validator_config = None;

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.validatorConfig",
                "validatorConfig is required for Validator nodes",
                "Add a spec.validatorConfig section with the required validator settings when nodeType is Validator.",
            )
        }));
    }

    #[test]
    fn test_validator_multi_replica_fails() {
        let mut spec = valid_validator_spec();
        spec.replicas = 2;

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.replicas",
                "Validator nodes must have exactly 1 replica",
                "Set spec.replicas to 1 for Validator nodes.",
            )
        }));
    }

    #[test]
    fn test_validator_zero_replica_fails() {
        let mut spec = valid_validator_spec();
        spec.replicas = 0;

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.replicas",
                "Validator nodes must have exactly 1 replica",
                "Set spec.replicas to 1 for Validator nodes.",
            )
        }));
    }

    #[test]
    fn test_validator_with_autoscaling_fails() {
        let mut spec = valid_validator_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 1,
            max_replicas: 3,
            target_cpu_utilization_percentage: Some(80),
            custom_metrics: vec![],
            behavior: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.autoscaling",
                "autoscaling is not supported for Validator nodes",
                "Remove spec.autoscaling when nodeType is Validator; autoscaling is only supported for Horizon and SorobanRpc.",
            )
        }));
    }

    #[test]
    fn test_validator_with_ingress_fails() {
        let mut spec = valid_validator_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "validator.example.com".to_string(),
                paths: vec![IngressPath {
                    path: "/".to_string(),
                    path_type: Some("Prefix".to_string()),
                }],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.ingress",
                "ingress is not supported for Validator nodes",
                "Remove spec.ingress for Validator nodes; expose Validator nodes using peer discovery or other supported mechanisms.",
            )
        }));
    }

    #[test]
    fn test_validator_history_archive_enabled_without_urls_fails() {
        let mut spec = valid_validator_spec();
        if let Some(ref mut vc) = spec.validator_config {
            vc.enable_history_archive = true;
            vc.history_archive_urls = vec![];
        }

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.validatorConfig.historyArchiveUrls",
                "historyArchiveUrls must not be empty when enableHistoryArchive is true",
                "Provide at least one valid history archive URL in spec.validatorConfig.historyArchiveUrls when enableHistoryArchive is true.",
            )
        }));
    }

    #[test]
    fn test_validator_history_archive_enabled_with_urls_passes() {
        let mut spec = valid_validator_spec();
        if let Some(ref mut vc) = spec.validator_config {
            vc.enable_history_archive = true;
            vc.history_archive_urls =
                vec!["https://history.stellar.org/prd/core-testnet".to_string()];
        }

        assert!(spec.validate().is_ok());
    }

    // =========================================================================
    // Horizon Node Tests
    // =========================================================================

    #[test]
    fn test_valid_horizon_passes_validation() {
        let spec = valid_horizon_spec();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_horizon_missing_config_fails() {
        let mut spec = valid_horizon_spec();
        spec.horizon_config = None;

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.horizonConfig",
                "horizonConfig is required for Horizon nodes",
                "Add a spec.horizonConfig section with the required Horizon settings when nodeType is Horizon.",
            )
        }));
    }

    #[test]
    fn test_horizon_with_multiple_replicas_passes() {
        let mut spec = valid_horizon_spec();
        spec.replicas = 5;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_horizon_valid_autoscaling_passes() {
        let mut spec = valid_horizon_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 2,
            max_replicas: 10,
            target_cpu_utilization_percentage: Some(80),
            custom_metrics: vec![],
            behavior: None,
        });

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_horizon_autoscaling_min_replicas_zero_fails() {
        let mut spec = valid_horizon_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 0,
            max_replicas: 5,
            target_cpu_utilization_percentage: Some(80),
            custom_metrics: vec![],
            behavior: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.autoscaling.minReplicas",
                "autoscaling.minReplicas must be at least 1",
                "Set spec.autoscaling.minReplicas to 1 or greater.",
            )
        }));
    }

    #[test]
    fn test_horizon_autoscaling_max_less_than_min_fails() {
        let mut spec = valid_horizon_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 5,
            max_replicas: 2,
            target_cpu_utilization_percentage: Some(80),
            custom_metrics: vec![],
            behavior: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.autoscaling.maxReplicas",
                "autoscaling.maxReplicas must be >= minReplicas",
                "Set spec.autoscaling.maxReplicas to be greater than or equal to minReplicas.",
            )
        }));
    }

    #[test]
    fn test_horizon_valid_ingress_passes() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "horizon.example.com".to_string(),
                paths: vec![IngressPath {
                    path: "/".to_string(),
                    path_type: Some("Prefix".to_string()),
                }],
            }],
            tls_secret_name: Some("horizon-tls".to_string()),
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_horizon_ingress_empty_hosts_fails() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.ingress.hosts",
                "ingress.hosts must not be empty",
                "Provide at least one host entry under spec.ingress.hosts.",
            )
        }));
    }

    #[test]
    fn test_horizon_ingress_empty_host_name_fails() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "   ".to_string(),
                paths: vec![IngressPath {
                    path: "/".to_string(),
                    path_type: Some("Prefix".to_string()),
                }],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.ingress.hosts[].host",
                "ingress.hosts[].host must not be empty",
                "Set a non-empty hostname for each ingress host entry.",
            )
        }));
    }

    #[test]
    fn test_horizon_ingress_empty_paths_fails() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "horizon.example.com".to_string(),
                paths: vec![],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.ingress.hosts[].paths",
                "ingress.hosts[].paths must not be empty",
                "Provide at least one path under spec.ingress.hosts[].paths for each host.",
            )
        }));
    }

    #[test]
    fn test_horizon_ingress_empty_path_value_fails() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "horizon.example.com".to_string(),
                paths: vec![IngressPath {
                    path: "  ".to_string(),
                    path_type: Some("Prefix".to_string()),
                }],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.ingress.hosts[].paths[].path",
                "ingress.hosts[].paths[].path must not be empty",
                "Set a non-empty HTTP path for each ingress path entry.",
            )
        }));
    }

    #[test]
    fn test_horizon_ingress_invalid_path_type_fails() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "horizon.example.com".to_string(),
                paths: vec![IngressPath {
                    path: "/api".to_string(),
                    path_type: Some("Regex".to_string()),
                }],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.ingress.hosts[].paths[].pathType",
                "ingress.hosts[].paths[].pathType must be either Prefix or Exact",
                "Set pathType to either \"Prefix\" or \"Exact\" for each ingress path.",
            )
        }));
    }

    #[test]
    fn test_horizon_ingress_exact_path_type_passes() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "horizon.example.com".to_string(),
                paths: vec![IngressPath {
                    path: "/health".to_string(),
                    path_type: Some("Exact".to_string()),
                }],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        assert!(spec.validate().is_ok());
    }

    // =========================================================================
    // SorobanRpc Node Tests
    // =========================================================================

    #[test]
    fn test_valid_soroban_passes_validation() {
        let spec = valid_soroban_spec();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_soroban_missing_config_fails() {
        let mut spec = valid_soroban_spec();
        spec.soroban_config = None;

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.sorobanConfig",
                "sorobanConfig is required for SorobanRpc nodes",
                "Add a spec.sorobanConfig section with the required Soroban RPC settings when nodeType is SorobanRpc.",
            )
        }));
    }

    #[test]
    fn test_soroban_with_multiple_replicas_passes() {
        let mut spec = valid_soroban_spec();
        spec.replicas = 10;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_soroban_valid_autoscaling_passes() {
        let mut spec = valid_soroban_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 3,
            max_replicas: 20,
            target_cpu_utilization_percentage: Some(70),
            custom_metrics: vec!["rpc_requests_per_second".to_string()],
            behavior: None,
        });

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_soroban_autoscaling_min_replicas_zero_fails() {
        let mut spec = valid_soroban_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 0,
            max_replicas: 10,
            target_cpu_utilization_percentage: None,
            custom_metrics: vec![],
            behavior: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.autoscaling.minReplicas",
                "autoscaling.minReplicas must be at least 1",
                "Set spec.autoscaling.minReplicas to 1 or greater.",
            )
        }));
    }

    #[test]
    fn test_soroban_autoscaling_max_less_than_min_fails() {
        let mut spec = valid_soroban_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 10,
            max_replicas: 5,
            target_cpu_utilization_percentage: Some(80),
            custom_metrics: vec![],
            behavior: None,
        });

        let result = spec.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| {
            e == &SpecValidationError::new(
                "spec.autoscaling.maxReplicas",
                "autoscaling.maxReplicas must be >= minReplicas",
                "Set spec.autoscaling.maxReplicas to be greater than or equal to minReplicas.",
            )
        }));
    }

    #[test]
    fn test_soroban_valid_ingress_passes() {
        let mut spec = valid_soroban_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "soroban.example.com".to_string(),
                paths: vec![IngressPath {
                    path: "/".to_string(),
                    path_type: Some("Prefix".to_string()),
                }],
            }],
            tls_secret_name: Some("soroban-tls".to_string()),
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: Some("letsencrypt-prod".to_string()),
            annotations: None,
        });

        assert!(spec.validate().is_ok());
    }

    // =========================================================================
    // Network Variant Tests
    // =========================================================================

    #[test]
    fn test_validator_mainnet_passes() {
        let mut spec = valid_validator_spec();
        spec.network = StellarNetwork::Mainnet;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_validator_futurenet_passes() {
        let mut spec = valid_validator_spec();
        spec.network = StellarNetwork::Futurenet;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_validator_custom_network_passes() {
        let mut spec = valid_validator_spec();
        spec.network = StellarNetwork::Custom("My Private Network".to_string());
        assert!(spec.validate().is_ok());
    }

    // =========================================================================
    // Edge Cases and Boundary Tests
    // =========================================================================

    #[test]
    fn test_validator_exactly_one_replica_passes() {
        let spec = valid_validator_spec();
        assert_eq!(spec.replicas, 1);
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_horizon_autoscaling_equal_min_max_passes() {
        let mut spec = valid_horizon_spec();
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 3,
            max_replicas: 3,
            target_cpu_utilization_percentage: Some(80),
            custom_metrics: vec![],
            behavior: None,
        });

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_suspended_validator_passes() {
        let mut spec = valid_validator_spec();
        spec.suspended = true;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_validator_with_alerting_passes() {
        let mut spec = valid_validator_spec();
        spec.alerting = true;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_validator_in_maintenance_mode_passes() {
        let mut spec = valid_validator_spec();
        spec.maintenance_mode = true;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_ingress_multiple_hosts_passes() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![
                IngressHost {
                    host: "horizon.example.com".to_string(),
                    paths: vec![IngressPath {
                        path: "/".to_string(),
                        path_type: Some("Prefix".to_string()),
                    }],
                },
                IngressHost {
                    host: "horizon-backup.example.com".to_string(),
                    paths: vec![IngressPath {
                        path: "/".to_string(),
                        path_type: Some("Prefix".to_string()),
                    }],
                },
            ],
            tls_secret_name: Some("horizon-tls".to_string()),
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_ingress_multiple_paths_passes() {
        let mut spec = valid_horizon_spec();
        spec.ingress = Some(IngressConfig {
            class_name: Some("nginx".to_string()),
            hosts: vec![IngressHost {
                host: "horizon.example.com".to_string(),
                paths: vec![
                    IngressPath {
                        path: "/api".to_string(),
                        path_type: Some("Prefix".to_string()),
                    },
                    IngressPath {
                        path: "/health".to_string(),
                        path_type: Some("Exact".to_string()),
                    },
                ],
            }],
            tls_secret_name: None,
            cert_manager_issuer: None,
            cert_manager_cluster_issuer: None,
            annotations: None,
        });

        assert!(spec.validate().is_ok());
    }
}
