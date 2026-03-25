//! Operator runtime configuration loaded from a mounted ConfigMap.
//!
//! Loaded from the file specified by the `STELLAR_OPERATOR_CONFIG` env var
//! (default: `/etc/stellar-operator/config.yaml`).
//!
//! # Precedence
//! StellarNode `spec.resources` > Helm defaults (this file) > hardcoded fallback.

use crate::crd::{NodeType, ResourceRequirements, ResourceSpec};
use serde::Deserialize;
use tracing::warn;

/// Per-node-type default resources from Helm `defaultResources.*`
///
/// `Default` uses **empty** cpu/memory strings so that `defaults_for()`
/// can detect "no Helm value provided" and fall through to hardcoded fallbacks.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeResourceDefaults {
    pub requests: ResourceSpec,
    pub limits: ResourceSpec,
}

impl Default for NodeResourceDefaults {
    fn default() -> Self {
        Self {
            requests: ResourceSpec {
                cpu: String::new(),
                memory: String::new(),
            },
            limits: ResourceSpec {
                cpu: String::new(),
                memory: String::new(),
            },
        }
    }
}

/// Top-level operator config file schema
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperatorConfig {
    pub default_resources: DefaultResources,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DefaultResources {
    pub validator: NodeResourceDefaults,
    pub horizon: NodeResourceDefaults,
    pub soroban_rpc: NodeResourceDefaults,
}

/// The path checked when no env var is present.
const DEFAULT_CONFIG_PATH: &str = "/etc/stellar-operator/config.yaml";

impl OperatorConfig {
    /// Load config from the file at `STELLAR_OPERATOR_CONFIG` or the default path.
    /// Returns `Default::default()` if the file does not exist or cannot be parsed.
    pub fn load() -> Self {
        let path = std::env::var("STELLAR_OPERATOR_CONFIG")
            .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        Self::load_from_file(&path)
    }

    pub fn load_from_file(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_yaml::from_str::<OperatorConfig>(&contents) {
                Ok(cfg) => {
                    tracing::info!("Loaded operator config from {path}");
                    cfg
                }
                Err(e) => {
                    warn!("Failed to parse operator config at {path}: {e}. Using defaults.");
                    Self::default()
                }
            },
            Err(_) => {
                // File absent is expected when running without Helm (e.g. local dev).
                tracing::debug!(
                    "No operator config file found at {path}. Using hardcoded defaults."
                );
                Self::default()
            }
        }
    }

    /// Return Helm defaults for the given node type, or `None` if both
    /// `requests` and `limits` cpu strings are empty (i.e. `Default::default()`).
    pub fn defaults_for(&self, node_type: &NodeType) -> Option<&NodeResourceDefaults> {
        let d = match node_type {
            NodeType::Validator => &self.default_resources.validator,
            NodeType::Horizon => &self.default_resources.horizon,
            NodeType::SorobanRpc => &self.default_resources.soroban_rpc,
        };
        if d.requests.cpu.is_empty() && d.limits.cpu.is_empty() {
            None
        } else {
            Some(d)
        }
    }
}

/// Hardcoded last-resort defaults (used when no config file is mounted and
/// the StellarNode spec does not specify resources).
pub fn hardcoded_defaults(node_type: &NodeType) -> ResourceRequirements {
    match node_type {
        NodeType::Validator => ResourceRequirements {
            requests: ResourceSpec {
                cpu: "500m".to_string(),
                memory: "1Gi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "2".to_string(),
                memory: "4Gi".to_string(),
            },
        },
        NodeType::Horizon => ResourceRequirements {
            requests: ResourceSpec {
                cpu: "250m".to_string(),
                memory: "512Mi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "2".to_string(),
                memory: "4Gi".to_string(),
            },
        },
        NodeType::SorobanRpc => ResourceRequirements {
            requests: ResourceSpec {
                cpu: "500m".to_string(),
                memory: "2Gi".to_string(),
            },
            limits: ResourceSpec {
                cpu: "4".to_string(),
                memory: "8Gi".to_string(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::NodeType;
    use std::io::Write;

    #[test]
    fn test_hardcoded_defaults_non_empty() {
        for nt in [NodeType::Validator, NodeType::Horizon, NodeType::SorobanRpc] {
            let d = hardcoded_defaults(&nt);
            assert!(
                !d.requests.cpu.is_empty(),
                "requests.cpu must not be empty for {nt:?}"
            );
            assert!(
                !d.limits.memory.is_empty(),
                "limits.memory must not be empty for {nt:?}"
            );
        }
    }

    #[test]
    fn test_load_from_file_valid_yaml() {
        let yaml = r#"
defaultResources:
  validator:
    requests:
      cpu: "750m"
      memory: "2Gi"
    limits:
      cpu: "3"
      memory: "6Gi"
  horizon:
    requests:
      cpu: "300m"
      memory: "1Gi"
    limits:
      cpu: "2"
      memory: "4Gi"
  sorobanRpc:
    requests:
      cpu: "1"
      memory: "4Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let cfg = OperatorConfig::load_from_file(f.path().to_str().unwrap());
        assert_eq!(cfg.default_resources.validator.requests.cpu, "750m");
        assert_eq!(cfg.default_resources.horizon.limits.memory, "4Gi");
    }

    #[test]
    fn test_load_from_missing_file_returns_default() {
        let cfg = OperatorConfig::load_from_file("/nonexistent/path/config.yaml");
        // Default has empty strings — defaults_for returns None
        assert!(cfg.defaults_for(&NodeType::Validator).is_none());
    }

    #[test]
    fn test_defaults_for_returns_none_on_empty() {
        let cfg = OperatorConfig::default();
        assert!(cfg.defaults_for(&NodeType::Horizon).is_none());
    }

    #[test]
    fn test_default_resource_precedence() {
        // When spec.resources is non-empty it should win over Helm defaults.
        // This is verified at the call-site in reconciler.rs; here we just
        // confirm `defaults_for` returns the Helm value when present.
        let yaml = r#"
defaultResources:
  validator:
    requests: { cpu: "999m", memory: "9Gi" }
    limits:   { cpu: "8",    memory: "16Gi" }
  horizon:
    requests: { cpu: "100m", memory: "256Mi" }
    limits:   { cpu: "1",    memory: "2Gi" }
  sorobanRpc:
    requests: { cpu: "200m", memory: "512Mi" }
    limits:   { cpu: "2",    memory: "4Gi" }
"#;
        let cfg: OperatorConfig = serde_yaml::from_str(yaml).unwrap();
        let d = cfg.defaults_for(&NodeType::Validator).unwrap();
        assert_eq!(d.requests.cpu, "999m");
    }
}
