//! Captive Core configuration builder for Soroban RPC
//!
//! This module provides utilities to generate TOML configuration for Captive Core
//! from structured Rust types, replacing the error-prone raw TOML string approach.

use crate::crd::{CaptiveCoreConfig, StellarNode};
use crate::error::{Error, Result};

/// Default Stellar Core peer port
const DEFAULT_PEER_PORT: u16 = 11625;

/// Default Stellar Core HTTP port
const DEFAULT_HTTP_PORT: u16 = 11626;

/// Default log level
const DEFAULT_LOG_LEVEL: &str = "info";

/// Builder for generating Captive Core TOML configuration
///
/// This builder extracts configuration from a StellarNode and generates
/// a properly formatted TOML file for Captive Core.
#[derive(Debug, Clone)]
pub struct CaptiveCoreConfigBuilder {
    network_passphrase: String,
    history_archive_urls: Vec<String>,
    peer_port: u16,
    http_port: u16,
    log_level: String,
    additional_config: Option<String>,
}

impl CaptiveCoreConfigBuilder {
    /// Create a builder from a StellarNode's configuration
    ///
    /// # Arguments
    ///
    /// * `node` - The StellarNode resource containing configuration
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * No Soroban configuration is provided
    /// * No history archive URLs are configured (and no structured config exists)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use stellar_k8s::controller::captive_core::CaptiveCoreConfigBuilder;
    /// use stellar_k8s::crd::StellarNode;
    ///
    /// // Assuming you have a StellarNode resource
    /// let builder = CaptiveCoreConfigBuilder::from_node_config(&node)?;
    /// # Ok::<(), stellar_k8s::error::Error>(())
    /// ```
    pub fn from_node_config(node: &StellarNode) -> Result<Self> {
        let soroban_config = node.spec.soroban_config.as_ref().ok_or_else(|| {
            Error::ConfigError(
                "SorobanConfig is required for Captive Core configuration".to_string(),
            )
        })?;

        // Check if structured config is provided (preferred)
        if let Some(structured_config) = &soroban_config.captive_core_structured_config {
            Self::from_structured_config(node, structured_config)
        } else {
            // Fallback to deprecated raw TOML (backward compatibility)
            Err(Error::ConfigError(
                "No structured Captive Core configuration provided. Please use captive_core_structured_config field.".to_string(),
            ))
        }
    }

    /// Create builder from structured configuration
    fn from_structured_config(node: &StellarNode, config: &CaptiveCoreConfig) -> Result<Self> {
        // Get network passphrase (use override or default from network)
        let network_passphrase = config
            .network_passphrase
            .clone()
            .unwrap_or_else(|| node.spec.network.passphrase().to_string());

        // Validate history archive URLs
        if config.history_archive_urls.is_empty() {
            return Err(Error::ConfigError(
                "At least one history archive URL is required for Captive Core".to_string(),
            ));
        }

        Ok(Self {
            network_passphrase,
            history_archive_urls: config.history_archive_urls.clone(),
            peer_port: config.peer_port.unwrap_or(DEFAULT_PEER_PORT),
            http_port: config.http_port.unwrap_or(DEFAULT_HTTP_PORT),
            log_level: config
                .log_level
                .clone()
                .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string()),
            additional_config: config.additional_config.clone(),
        })
    }

    /// Generate TOML configuration string
    ///
    /// Creates a properly formatted Stellar Core TOML configuration
    /// suitable for Captive Core.
    ///
    /// # Returns
    ///
    /// Returns a complete TOML configuration string
    ///
    /// # Examples
    ///
    /// ```ignore
    /// # use stellar_k8s::controller::captive_core::CaptiveCoreConfigBuilder;
    /// # use stellar_k8s::crd::StellarNode;
    /// // Assuming you have a StellarNode resource
    /// let builder = CaptiveCoreConfigBuilder::from_node_config(&node)?;
    /// let toml = builder.build_toml()?;
    /// println!("{}", toml);
    /// # Ok::<(), stellar_k8s::error::Error>(())
    /// ```
    pub fn build_toml(&self) -> Result<String> {
        self.validate()?;

        let mut toml = String::new();

        // Network passphrase
        toml.push_str(&format!(
            "NETWORK_PASSPHRASE=\"{}\"\n\n",
            self.network_passphrase
        ));

        // History archives
        // Stellar Core expects each archive to have a unique name
        for (idx, url) in self.history_archive_urls.iter().enumerate() {
            let archive_name = format!("archive{}", idx + 1);
            toml.push_str(&format!("[HISTORY.{archive_name}]\n"));
            // Use curl to fetch history archives (standard Stellar pattern)
            toml.push_str(&format!("get=\"curl -sf {url}/{{0}} -o {{1}}\"\n\n"));
        }

        // Peer and HTTP ports
        toml.push_str(&format!("PEER_PORT={}\n", self.peer_port));
        toml.push_str(&format!("HTTP_PORT={}\n", self.http_port));

        // Log level
        toml.push_str(&format!("LOG_LEVEL=\"{}\"\n", self.log_level));

        // Append additional custom configuration if provided
        if let Some(additional) = &self.additional_config {
            toml.push_str("\n# Additional custom configuration\n");
            toml.push_str(additional);
            if !additional.ends_with('\n') {
                toml.push('\n');
            }
        }

        Ok(toml)
    }

    /// Validate the configuration
    ///
    /// Ensures all required fields are present and valid.
    fn validate(&self) -> Result<()> {
        if self.network_passphrase.is_empty() {
            return Err(Error::ConfigError(
                "Network passphrase cannot be empty".to_string(),
            ));
        }

        if self.history_archive_urls.is_empty() {
            return Err(Error::ConfigError(
                "At least one history archive URL is required".to_string(),
            ));
        }

        // Validate log level
        let valid_log_levels = ["fatal", "error", "warning", "info", "debug", "trace"];
        if !valid_log_levels.contains(&self.log_level.as_str()) {
            return Err(Error::ConfigError(format!(
                "Invalid log level '{}'. Valid values: {:?}",
                self.log_level, valid_log_levels
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{CaptiveCoreConfig, NodeType, SorobanConfig, StellarNetwork, StellarNodeSpec};

    /// Helper to create a test StellarNode with Soroban config
    fn create_test_node(captive_config: CaptiveCoreConfig) -> StellarNode {
        StellarNode {
            metadata: kube::api::ObjectMeta {
                name: Some("test-soroban".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::SorobanRpc,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                history_mode: Default::default(),
                resources: crate::crd::ResourceRequirements {
                    requests: crate::crd::ResourceSpec {
                        cpu: "500m".to_string(),
                        memory: "1Gi".to_string(),
                    },
                    limits: crate::crd::ResourceSpec {
                        cpu: "2".to_string(),
                        memory: "4Gi".to_string(),
                    },
                },
                storage: crate::crd::StorageConfig {
                    storage_class: "standard".to_string(),
                    size: "100Gi".to_string(),
                    retention_policy: Default::default(),
                    annotations: None,
                },
                validator_config: None,
                horizon_config: None,
                soroban_config: Some(SorobanConfig {
                    stellar_core_url: "http://core:11626".to_string(),
                    #[allow(deprecated)]
                    captive_core_config: None,
                    captive_core_structured_config: Some(captive_config),
                    enable_preflight: true,
                    max_events_per_request: 10000,
                }),
                replicas: 2,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                // Added this field to resolve the E0063 error
                managed_database: None,
                autoscaling: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
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
                vpa_config: None,
                read_pool_endpoint: None,
            },
            status: None,
        }
    }

    #[test]
    fn test_builder_from_valid_config() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-testnet/core_testnet_001".to_string(),
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node);

        assert!(builder.is_ok());
        let builder = builder.unwrap();
        assert_eq!(
            builder.network_passphrase,
            "Test SDF Network ; September 2015"
        );
        assert_eq!(builder.history_archive_urls.len(), 1);
        assert_eq!(builder.peer_port, DEFAULT_PEER_PORT);
        assert_eq!(builder.http_port, DEFAULT_HTTP_PORT);
        assert_eq!(builder.log_level, DEFAULT_LOG_LEVEL);
    }

    #[test]
    fn test_toml_generation_testnet() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-testnet/core_testnet_001".to_string(),
                "https://history.stellar.org/prd/core-testnet/core_testnet_002".to_string(),
            ],
            peer_port: None,
            http_port: None,
            log_level: Some("debug".to_string()),
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        assert!(toml.contains("NETWORK_PASSPHRASE=\"Test SDF Network ; September 2015\""));
        assert!(toml.contains("[HISTORY.archive1]"));
        assert!(toml.contains("[HISTORY.archive2]"));
        assert!(toml.contains("PEER_PORT=11625"));
        assert!(toml.contains("HTTP_PORT=11626"));
        assert!(toml.contains("LOG_LEVEL=\"debug\""));
    }

    #[test]
    fn test_toml_generation_mainnet() {
        let config = CaptiveCoreConfig {
            network_passphrase: Some("Public Global Stellar Network ; September 2015".to_string()),
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-live/core_live_001".to_string()
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        assert!(
            toml.contains("NETWORK_PASSPHRASE=\"Public Global Stellar Network ; September 2015\"")
        );
        assert!(toml.contains("[HISTORY.archive1]"));
    }

    #[test]
    fn test_toml_generation_with_custom_ports() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: Some(11700),
            http_port: Some(11701),
            log_level: None,
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        assert!(toml.contains("PEER_PORT=11700"));
        assert!(toml.contains("HTTP_PORT=11701"));
    }

    #[test]
    fn test_validation_missing_history_archives() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![], // Empty!
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let node = create_test_node(config);
        let result = CaptiveCoreConfigBuilder::from_node_config(&node);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("At least one history archive URL is required"));
    }

    #[test]
    fn test_validation_invalid_log_level() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: Some("invalid".to_string()),
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let result = builder.build_toml();

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Invalid log level"));
    }

    #[test]
    fn test_additional_config_appending() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: Some("# Custom config\nMAX_CONCURRENT_SUBPROCESSES=10".to_string()),
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        assert!(toml.contains("# Additional custom configuration"));
        assert!(toml.contains("MAX_CONCURRENT_SUBPROCESSES=10"));
    }

    /// Test that updating the config spec produces different TOML output
    #[test]
    fn test_config_update_produces_different_output() {
        // Initial configuration
        let config1 = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive1.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: Some("info".to_string()),
            additional_config: None,
        };

        let node1 = create_test_node(config1);
        let builder1 = CaptiveCoreConfigBuilder::from_node_config(&node1).unwrap();
        let toml1 = builder1.build_toml().unwrap();

        // Updated configuration with different values
        let config2 = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![
                "https://archive1.example.com".to_string(),
                "https://archive2.example.com".to_string(),
            ],
            peer_port: Some(11700),
            http_port: Some(11701),
            log_level: Some("debug".to_string()),
            additional_config: Some("NODE_SEED=\"SXYZ\"".to_string()),
        };

        let node2 = create_test_node(config2);
        let builder2 = CaptiveCoreConfigBuilder::from_node_config(&node2).unwrap();
        let toml2 = builder2.build_toml().unwrap();

        // Verify the outputs are different
        assert_ne!(toml1, toml2);

        // Verify specific changes
        assert!(!toml1.contains("archive2"));
        assert!(toml2.contains("archive2"));

        assert!(toml1.contains("PEER_PORT=11625"));
        assert!(toml2.contains("PEER_PORT=11700"));

        assert!(toml1.contains("LOG_LEVEL=\"info\""));
        assert!(toml2.contains("LOG_LEVEL=\"debug\""));

        assert!(!toml1.contains("NODE_SEED"));
        assert!(toml2.contains("NODE_SEED=\"SXYZ\""));
    }

    /// Test that Mainnet and Testnet produce different network passphrases
    #[test]
    fn test_network_specific_settings_mainnet_vs_testnet() {
        // Testnet configuration
        let testnet_config = CaptiveCoreConfig {
            network_passphrase: None, // Will use default from network
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-testnet/core_testnet_001".to_string(),
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };
        let mut testnet_node = create_test_node(testnet_config);
        testnet_node.spec.network = StellarNetwork::Testnet;

        let testnet_builder = CaptiveCoreConfigBuilder::from_node_config(&testnet_node).unwrap();
        let testnet_toml = testnet_builder.build_toml().unwrap();

        // Mainnet configuration
        let mainnet_config = CaptiveCoreConfig {
            network_passphrase: None, // Will use default from network
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-live/core_live_001".to_string()
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };
        let mut mainnet_node = create_test_node(mainnet_config);
        mainnet_node.spec.network = StellarNetwork::Mainnet;

        let mainnet_builder = CaptiveCoreConfigBuilder::from_node_config(&mainnet_node).unwrap();
        let mainnet_toml = mainnet_builder.build_toml().unwrap();

        // Verify different network passphrases
        assert!(testnet_toml.contains("NETWORK_PASSPHRASE=\"Test SDF Network ; September 2015\""));
        assert!(mainnet_toml
            .contains("NETWORK_PASSPHRASE=\"Public Global Stellar Network ; September 2015\""));

        // Verify different archive URLs
        assert!(testnet_toml.contains("core-testnet"));
        assert!(mainnet_toml.contains("core-live"));

        // Ensure they're not the same
        assert_ne!(testnet_toml, mainnet_toml);
    }

    /// Test Futurenet network configuration
    #[test]
    fn test_futurenet_network_configuration() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-futurenet/core_futurenet_001".to_string(),
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let mut node = create_test_node(config);
        node.spec.network = StellarNetwork::Futurenet;

        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        assert!(toml.contains("NETWORK_PASSPHRASE=\"Test SDF Future Network ; October 2022\""));
        assert!(toml.contains("core-futurenet"));
    }

    /// Test custom network configuration
    #[test]
    fn test_custom_network_configuration() {
        let custom_passphrase = "My Custom Network ; January 2026";

        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://custom-archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let mut node = create_test_node(config);
        node.spec.network = StellarNetwork::Custom(custom_passphrase.to_string());

        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        assert!(toml.contains(&format!("NETWORK_PASSPHRASE=\"{}\"", custom_passphrase)));
    }

    /// Test handling missing optional fields (all defaults)
    #[test]
    fn test_missing_optional_fields_use_defaults() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,         // Should default to 11625
            http_port: None,         // Should default to 11626
            log_level: None,         // Should default to "info"
            additional_config: None, // Should be omitted
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();

        // Verify defaults are applied
        assert_eq!(builder.peer_port, DEFAULT_PEER_PORT);
        assert_eq!(builder.http_port, DEFAULT_HTTP_PORT);
        assert_eq!(builder.log_level, DEFAULT_LOG_LEVEL);
        assert!(builder.additional_config.is_none());

        // Verify the TOML contains defaults
        let toml = builder.build_toml().unwrap();
        assert!(toml.contains("PEER_PORT=11625"));
        assert!(toml.contains("HTTP_PORT=11626"));
        assert!(toml.contains("LOG_LEVEL=\"info\""));
        assert!(!toml.contains("# Additional custom configuration"));
    }

    /// Test handling missing soroban_config entirely
    #[test]
    fn test_missing_soroban_config_returns_error() {
        let mut node = create_test_node(CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        });

        // Remove soroban config entirely
        node.spec.soroban_config = None;

        let result = CaptiveCoreConfigBuilder::from_node_config(&node);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("SorobanConfig is required"));
    }

    /// Test handling missing captive_core_structured_config
    #[test]
    fn test_missing_captive_core_structured_config_returns_error() {
        let mut node = create_test_node(CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        });

        // Remove structured config
        if let Some(ref mut soroban) = node.spec.soroban_config {
            soroban.captive_core_structured_config = None;
        }

        let result = CaptiveCoreConfigBuilder::from_node_config(&node);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("No structured Captive Core configuration provided"));
    }

    /// Test network passphrase override takes precedence
    #[test]
    fn test_network_passphrase_override() {
        let custom_passphrase = "Override Passphrase ; Test 2026";

        let config = CaptiveCoreConfig {
            network_passphrase: Some(custom_passphrase.to_string()),
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let mut node = create_test_node(config);
        node.spec.network = StellarNetwork::Testnet; // This should be overridden

        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        // Should use override, not Testnet passphrase
        assert!(toml.contains(&format!("NETWORK_PASSPHRASE=\"{}\"", custom_passphrase)));
        assert!(!toml.contains("Test SDF Network"));
    }

    /// Test TOML format with multiple archives
    #[test]
    fn test_toml_format_multiple_archives() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![
                "https://archive1.example.com".to_string(),
                "https://archive2.example.com".to_string(),
                "https://archive3.example.com".to_string(),
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        // Verify all archives are present with correct naming
        assert!(toml.contains("[HISTORY.archive1]"));
        assert!(toml.contains("curl -sf https://archive1.example.com/{0} -o {1}"));
        assert!(toml.contains("[HISTORY.archive2]"));
        assert!(toml.contains("curl -sf https://archive2.example.com/{0} -o {1}"));
        assert!(toml.contains("[HISTORY.archive3]"));
        assert!(toml.contains("curl -sf https://archive3.example.com/{0} -o {1}"));
    }

    /// Test all valid log levels are accepted
    #[test]
    fn test_all_valid_log_levels() {
        let log_levels = vec!["fatal", "error", "warning", "info", "debug", "trace"];

        for log_level in log_levels {
            let config = CaptiveCoreConfig {
                network_passphrase: None,
                history_archive_urls: vec!["https://archive.example.com".to_string()],
                peer_port: None,
                http_port: None,
                log_level: Some(log_level.to_string()),
                additional_config: None,
            };

            let node = create_test_node(config);
            let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
            let toml = builder.build_toml();

            assert!(toml.is_ok(), "Log level '{}' should be valid", log_level);
            let toml = toml.unwrap();
            assert!(toml.contains(&format!("LOG_LEVEL=\"{}\"", log_level)));
        }
    }

    /// Test validation catches empty network passphrase
    #[test]
    fn test_validation_empty_passphrase() {
        let builder = CaptiveCoreConfigBuilder {
            network_passphrase: String::new(), // Empty!
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: DEFAULT_PEER_PORT,
            http_port: DEFAULT_HTTP_PORT,
            log_level: DEFAULT_LOG_LEVEL.to_string(),
            additional_config: None,
        };

        let result = builder.build_toml();
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Network passphrase cannot be empty"));
    }

    /// Test TOML output is well-formed
    #[test]
    fn test_toml_output_well_formed() {
        let config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec!["https://archive.example.com".to_string()],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let node = create_test_node(config);
        let builder = CaptiveCoreConfigBuilder::from_node_config(&node).unwrap();
        let toml = builder.build_toml().unwrap();

        // Verify basic TOML structure
        assert!(toml.contains("NETWORK_PASSPHRASE="));
        assert!(toml.contains("[HISTORY.archive1]"));
        assert!(toml.contains("get="));
        assert!(toml.contains("PEER_PORT="));
        assert!(toml.contains("HTTP_PORT="));
        assert!(toml.contains("LOG_LEVEL="));

        // Verify no trailing issues
        assert!(!toml.is_empty());
        assert!(toml.len() > 100); // Should be reasonably sized
    }
}
