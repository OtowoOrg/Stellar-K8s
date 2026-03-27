//! TOML validation tests for ConfigMap generation
//! 
//! Verifies generated stellar-core.cfg and captive-core.cfg are valid TOML.

#[cfg(test)]
mod tests {
    use super::super::resources::{build_config_map_for_test, make_node};
    use crate::crd::stellar_node::NodeType;
    use crate::crd::ValidatorConfig;
    use crate::crd::SorobanConfig;
    use crate::crd::CaptiveCoreConfig;
    use toml::Value;
    use std::collections::BTreeMap;

    #[test]
    fn test_validator_stellar_core_config_is_valid_toml() {
        let mut node = make_node(NodeType::Validator);
        node.spec.validator_config = Some(ValidatorConfig {
            seed_secret_ref: "test-seed".to_string(),
            seed_secret_source: None,
            quorum_set: Some("[VALIDATORS]\nTHRESHOLD_PERCENT=66".to_string()),
            enable_history_archive: true,
            history_archive_urls: vec!["https://history.stellar.org".to_string()],
            catchup_complete: true,
            key_source: crate::crd::types::KeySource::Secret,
            kms_config: None,
            vl_source: None,
            hsm_config: None,
        });

        let cm = build_config_map_for_test(&node, None, false);
        
        if let Some(data) = cm.data {
            if let Some(core_cfg) = data.get("stellar-core.cfg") {
                // Should parse as valid TOML
                let parsed: Value = toml::from_str(core_cfg).expect("stellar-core.cfg is invalid TOML");
                assert!(parsed.is_table(), "root should be table");
                assert!(parsed.get("NETWORK_PASSPHRASE").is_some(), "missing NETWORK_PASSPHRASE");
            } else {
                panic!("no stellar-core.cfg in ConfigMap.data");
            }
        } else {
            panic!("ConfigMap.data is None");
        }
    }

    #[test]
    fn test_validator_mtls_config_is_valid_toml() {
        let mut node = make_node(NodeType::Validator);
        node.spec.validator_config = Some(ValidatorConfig {
            seed_secret_ref: "test-seed".to_string(),
            seed_secret_source: None,
            quorum_set: Some("[VALIDATORS]\nTHRESHOLD_PERCENT=66".to_string()),
            enable_history_archive: true,
            history_archive_urls: vec!["https://history.stellar.org".to_string()],
            catchup_complete: true,
            key_source: crate::crd::types::KeySource::Secret,
            kms_config: None,
            vl_source: None,
            hsm_config: None,
        });

        let cm = build_config_map_for_test(&node, None, true);  // enable_mtls = true
        
        if let Some(data) = cm.data {
            if let Some(core_cfg) = data.get("stellar-core.cfg") {
                let parsed: Value = toml::from_str(core_cfg).expect("mTLS stellar-core.cfg is invalid TOML");
                assert!(parsed.is_table());
                // mTLS should add HTTP_PORT_SECURE, TLS cert paths
                assert!(parsed.get("HTTP_PORT_SECURE").is_some(), "missing HTTP_PORT_SECURE for mTLS");
            } else {
                panic!("no stellar-core.cfg with mTLS");
            }
        }
    }

    #[test]
    fn test_soroban_captive_core_config_is_valid_toml() {
        let captive_config = CaptiveCoreConfig {
            network_passphrase: None,
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-testnet/core_testnet_001".to_string()
            ],
            peer_port: None,
            http_port: None,
            log_level: None,
            additional_config: None,
        };

        let mut node = make_node(NodeType::SorobanRpc);
        node.spec.soroban_config = Some(SorobanConfig {
            stellar_core_url: "http://core:11626".to_string(),
            captive_core_config: None,  // deprecated
            captive_core_structured_config: Some(captive_config),
            enable_preflight: false,
            max_events_per_request: 1000,
        });

        let cm = build_config_map_for_test(&node, None, false);

        if let Some(data) = cm.data {
            if let Some(captive_cfg) = data.get("captive-core.cfg") {
                let parsed: Value = toml::from_str(captive_cfg).expect("captive-core.cfg invalid TOML");
                assert!(parsed.is_table());
                assert!(parsed.get("NETWORK_PASSPHRASE").is_some());
                assert!(parsed.get("PEER_PORT").is_some());
                assert!(parsed.get("HTTP_PORT").is_some());
            } else {
                panic!("no captive-core.cfg in Soroban ConfigMap");
            }
        } else {
            panic!("ConfigMap.data is None for Soroban");
        }
    }

    #[test]
    #[should_panic(expected = "invalid TOML")]
    fn test_invalid_quorum_panics() {
        let mut node = make_node(NodeType::Validator);
        node.spec.validator_config = Some(ValidatorConfig {
            seed_secret_ref: "test".to_string(),
            seed_secret_source: None,
            // Invalid TOML to test panic on parse error
            quorum_set: Some("INVALID TOML [".to_string()),
            enable_history_archive: false,
            history_archive_urls: vec![],
            catchup_complete: false,
            key_source: crate::crd::types::KeySource::Secret,
            kms_config: None,
            vl_source: None,
            hsm_config: None,
        });

        let cm = build_config_map_for_test(&node, None, false);
        // This will panic if stellar-core.cfg exists (which it will) because of malformed TOML
        if let Some(data) = cm.data {
            if let Some(cfg) = data.get("stellar-core.cfg") {
                let _ = toml::from_str::<Value>(cfg).unwrap();  // PANIC here
            }
        }
    }
}

