//! Operator runtime configuration loaded from a mounted ConfigMap.
//!
//! Loaded from the file specified by the `STELLAR_OPERATOR_CONFIG` env var
//! (default: `/etc/stellar-operator/config.yaml`).
//!
//! # Precedence
//! StellarNode `spec.resources` > Helm defaults (this file) > hardcoded fallback.

use crate::crd::{NodeType, ResourceRequirements, ResourceSpec};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

/// Per-node-type default resources from Helm `defaultResources.*`
///
/// `Default` uses **empty** cpu/memory strings so that `defaults_for()`
/// can detect "no Helm value provided" and fall through to hardcoded fallbacks.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperatorConfig {
    pub default_resources: DefaultResources,
    #[serde(default)]
    pub reconciler: ReconcilerConfig,
}

/// Reconciler configuration for requeue intervals and backoff
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcilerConfig {
    /// Base requeue interval for healthy reconciliation loops (seconds)
    #[serde(default = "default_requeue_interval")]
    pub requeue_interval: u64,

    /// Base backoff duration for error retries (seconds)
    #[serde(default = "default_error_backoff_base")]
    pub error_backoff_base: u64,

    /// Maximum backoff duration (seconds)
    #[serde(default = "default_max_backoff")]
    pub max_backoff: u64,

    /// Enable jitter for backoff calculations
    #[serde(default = "default_enable_jitter")]
    pub enable_jitter: bool,
}

fn default_requeue_interval() -> u64 {
    60
}

fn default_error_backoff_base() -> u64 {
    15
}

fn default_max_backoff() -> u64 {
    300
}

fn default_enable_jitter() -> bool {
    true
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            requeue_interval: default_requeue_interval(),
            error_backoff_base: default_error_backoff_base(),
            max_backoff: default_max_backoff(),
            enable_jitter: default_enable_jitter(),
        }
    }
}

impl ReconcilerConfig {
    /// Calculate exponential backoff with optional jitter
    ///
    /// # Arguments
    /// * `retry_count` - Number of retries attempted (0-indexed)
    ///
    /// # Returns
    /// Duration to wait before next retry
    pub fn calculate_backoff(&self, retry_count: u32) -> Duration {
        // Exponential backoff: base * 2^retry_count
        let backoff_secs = self
            .error_backoff_base
            .saturating_mul(2u64.saturating_pow(retry_count))
            .min(self.max_backoff);

        let backoff_secs = if self.enable_jitter {
            // Add jitter: random value between 0.5x and 1.5x of calculated backoff
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let jitter_factor = rng.gen_range(0.5..=1.5);
            ((backoff_secs as f64) * jitter_factor) as u64
        } else {
            backoff_secs
        };

        Duration::from_secs(backoff_secs)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
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

    #[test]
    fn test_reconciler_config_defaults() {
        let config = ReconcilerConfig::default();
        assert_eq!(config.requeue_interval, 60);
        assert_eq!(config.error_backoff_base, 15);
        assert_eq!(config.max_backoff, 300);
        assert!(config.enable_jitter);
    }

    #[test]
    fn test_calculate_backoff_exponential() {
        let config = ReconcilerConfig {
            requeue_interval: 60,
            error_backoff_base: 10,
            max_backoff: 300,
            enable_jitter: false,
        };

        // Test exponential growth: base * 2^retry_count
        assert_eq!(config.calculate_backoff(0).as_secs(), 10); // 10 * 2^0 = 10
        assert_eq!(config.calculate_backoff(1).as_secs(), 20); // 10 * 2^1 = 20
        assert_eq!(config.calculate_backoff(2).as_secs(), 40); // 10 * 2^2 = 40
        assert_eq!(config.calculate_backoff(3).as_secs(), 80); // 10 * 2^3 = 80
        assert_eq!(config.calculate_backoff(4).as_secs(), 160); // 10 * 2^4 = 160
    }

    #[test]
    fn test_calculate_backoff_max_cap() {
        let config = ReconcilerConfig {
            requeue_interval: 60,
            error_backoff_base: 10,
            max_backoff: 100,
            enable_jitter: false,
        };

        // Should cap at max_backoff
        assert_eq!(config.calculate_backoff(5).as_secs(), 100); // Would be 320, capped at 100
        assert_eq!(config.calculate_backoff(10).as_secs(), 100); // Would be 10240, capped at 100
    }

    #[test]
    fn test_calculate_backoff_with_jitter() {
        let config = ReconcilerConfig {
            requeue_interval: 60,
            error_backoff_base: 10,
            max_backoff: 300,
            enable_jitter: true,
        };

        // With jitter, result should be between 0.5x and 1.5x of base calculation
        let backoff = config.calculate_backoff(2).as_secs(); // Base would be 40
        assert!(
            (20..=60).contains(&backoff),
            "Backoff {backoff} not in range [20, 60]"
        );
    }

    #[test]
    fn test_calculate_backoff_overflow_protection() {
        let config = ReconcilerConfig {
            requeue_interval: 60,
            error_backoff_base: u64::MAX / 2,
            max_backoff: 300,
            enable_jitter: false,
        };

        // Should handle overflow gracefully and cap at max_backoff
        assert_eq!(config.calculate_backoff(10).as_secs(), 300);
    }

    #[test]
    fn test_load_config_with_reconciler_settings() {
        let yaml = r#"
defaultResources:
  validator:
    requests: { cpu: "500m", memory: "1Gi" }
    limits: { cpu: "2", memory: "4Gi" }
  horizon:
    requests: { cpu: "250m", memory: "512Mi" }
    limits: { cpu: "1", memory: "2Gi" }
  sorobanRpc:
    requests: { cpu: "500m", memory: "2Gi" }
    limits: { cpu: "4", memory: "8Gi" }
reconciler:
  requeueInterval: 120
  errorBackoffBase: 30
  maxBackoff: 600
  enableJitter: false
"#;
        let cfg: OperatorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.reconciler.requeue_interval, 120);
        assert_eq!(cfg.reconciler.error_backoff_base, 30);
        assert_eq!(cfg.reconciler.max_backoff, 600);
        assert!(!cfg.reconciler.enable_jitter);
    }
}
