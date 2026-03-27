//! Runtime feature flags loaded from the `stellar-operator-config` ConfigMap.
//!
//! The operator watches this ConfigMap and reloads flags without restart.
//!
//! # Available Feature Flags
//!
//! | Flag | Default | Description |
//! |------|---------|-------------|
//! | `enable_cve_scanning` | `true` | Enable automatic CVE patch reconciliation |
//! | `enable_read_pool` | `false` | Enable read-replica pool management |
//! | `enable_dr` | `false` | Enable disaster-recovery drill scheduling |
//! | `enable_peer_discovery` | `true` | Enable automatic peer discovery |
//! | `enable_archive_health` | `true` | Enable history archive health checks |
//! | `enable_soroban_metrics` | `true` | Enable Soroban-specific Prometheus metrics |
//!
//! # ConfigMap Example
//!
//! ```yaml
//! apiVersion: v1
//! kind: ConfigMap
//! metadata:
//!   name: stellar-operator-config
//!   namespace: stellar-system
//! data:
//!   enable_cve_scanning: "true"
//!   enable_read_pool: "false"
//!   enable_dr: "false"
//!   enable_peer_discovery: "true"
//!   enable_archive_health: "true"
//!   enable_soroban_metrics: "true"
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    api::Api,
    runtime::watcher::{self, Event},
    Client, ResourceExt,
};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Name of the feature-flags ConfigMap the operator watches.
pub const FEATURE_FLAGS_CONFIGMAP: &str = "stellar-operator-config";

/// Runtime feature flags. All fields default to safe production values.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureFlags {
    /// Enable automatic CVE patch reconciliation.
    pub enable_cve_scanning: bool,
    /// Enable read-replica pool management.
    pub enable_read_pool: bool,
    /// Enable disaster-recovery drill scheduling.
    pub enable_dr: bool,
    /// Enable automatic peer discovery.
    pub enable_peer_discovery: bool,
    /// Enable history archive health checks.
    pub enable_archive_health: bool,
    /// Enable Soroban-specific Prometheus metrics collection.
    pub enable_soroban_metrics: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            enable_cve_scanning: true,
            enable_read_pool: false,
            enable_dr: false,
            enable_peer_discovery: true,
            enable_archive_health: true,
            enable_soroban_metrics: true,
        }
    }
}

impl FeatureFlags {
    /// Parse flags from a ConfigMap's `data` field.
    /// Unknown keys are silently ignored; missing keys fall back to defaults.
    pub fn from_config_map_data(data: &BTreeMap<String, String>) -> Self {
        let defaults = Self::default();
        let parse = |key: &str, default: bool| -> bool {
            data.get(key)
                .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
                .unwrap_or(default)
        };

        Self {
            enable_cve_scanning: parse("enable_cve_scanning", defaults.enable_cve_scanning),
            enable_read_pool: parse("enable_read_pool", defaults.enable_read_pool),
            enable_dr: parse("enable_dr", defaults.enable_dr),
            enable_peer_discovery: parse("enable_peer_discovery", defaults.enable_peer_discovery),
            enable_archive_health: parse("enable_archive_health", defaults.enable_archive_health),
            enable_soroban_metrics: parse(
                "enable_soroban_metrics",
                defaults.enable_soroban_metrics,
            ),
        }
    }
}

/// Shared, live-reloadable feature flags handle.
pub type SharedFeatureFlags = Arc<RwLock<FeatureFlags>>;

/// Create a new `SharedFeatureFlags` initialised with defaults.
pub fn new_shared() -> SharedFeatureFlags {
    Arc::new(RwLock::new(FeatureFlags::default()))
}

/// Watch the `stellar-operator-config` ConfigMap in `namespace` and update
/// `flags` whenever it changes. Runs until the task is cancelled.
///
/// This function is intended to be spawned as a background `tokio::task`.
pub async fn watch_feature_flags(client: Client, namespace: String, flags: SharedFeatureFlags) {
    let api: Api<ConfigMap> = Api::namespaced(client, &namespace);

    let watcher_config =
        watcher::Config::default().fields(&format!("metadata.name={FEATURE_FLAGS_CONFIGMAP}"));

    let mut stream = watcher::watcher(api, watcher_config).boxed();

    info!(
        namespace = %namespace,
        configmap = FEATURE_FLAGS_CONFIGMAP,
        "Starting feature-flag watcher"
    );

    while let Some(event) = stream.next().await {
        match event {
            Ok(Event::Apply(cm)) | Ok(Event::InitApply(cm)) => {
                let data = cm.data.clone().unwrap_or_default();
                let new_flags = FeatureFlags::from_config_map_data(&data);

                let mut current = flags.write().await;
                if *current != new_flags {
                    log_flag_changes(&current, &new_flags, cm.name_any().as_str());
                    *current = new_flags;
                }
            }
            Ok(Event::Delete(_)) => {
                warn!(
                    configmap = FEATURE_FLAGS_CONFIGMAP,
                    "Feature-flags ConfigMap deleted; reverting to defaults"
                );
                let mut current = flags.write().await;
                *current = FeatureFlags::default();
            }
            Ok(Event::Init) | Ok(Event::InitDone) => {}
            Err(e) => {
                warn!(
                    error = %e,
                    configmap = FEATURE_FLAGS_CONFIGMAP,
                    "Feature-flag watcher error; will retry"
                );
            }
        }
    }
}

/// Log each flag that changed at INFO level.
fn log_flag_changes(old: &FeatureFlags, new: &FeatureFlags, configmap_name: &str) {
    macro_rules! log_if_changed {
        ($field:ident) => {
            if old.$field != new.$field {
                info!(
                    configmap = configmap_name,
                    flag = stringify!($field),
                    old = old.$field,
                    new = new.$field,
                    "Feature flag changed"
                );
            }
        };
    }

    log_if_changed!(enable_cve_scanning);
    log_if_changed!(enable_read_pool);
    log_if_changed!(enable_dr);
    log_if_changed!(enable_peer_discovery);
    log_if_changed!(enable_archive_health);
    log_if_changed!(enable_soroban_metrics);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_defaults() {
        let flags = FeatureFlags::default();
        assert!(flags.enable_cve_scanning);
        assert!(!flags.enable_read_pool);
        assert!(!flags.enable_dr);
        assert!(flags.enable_peer_discovery);
        assert!(flags.enable_archive_health);
        assert!(flags.enable_soroban_metrics);
    }

    #[test]
    fn test_parse_all_true() {
        let d = data(&[
            ("enable_cve_scanning", "true"),
            ("enable_read_pool", "true"),
            ("enable_dr", "true"),
            ("enable_peer_discovery", "true"),
            ("enable_archive_health", "true"),
            ("enable_soroban_metrics", "true"),
        ]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_cve_scanning);
        assert!(flags.enable_read_pool);
        assert!(flags.enable_dr);
        assert!(flags.enable_peer_discovery);
        assert!(flags.enable_archive_health);
        assert!(flags.enable_soroban_metrics);
    }

    #[test]
    fn test_parse_all_false() {
        let d = data(&[
            ("enable_cve_scanning", "false"),
            ("enable_read_pool", "false"),
            ("enable_dr", "false"),
            ("enable_peer_discovery", "false"),
            ("enable_archive_health", "false"),
            ("enable_soroban_metrics", "false"),
        ]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(!flags.enable_cve_scanning);
        assert!(!flags.enable_read_pool);
        assert!(!flags.enable_dr);
        assert!(!flags.enable_peer_discovery);
        assert!(!flags.enable_archive_health);
        assert!(!flags.enable_soroban_metrics);
    }

    #[test]
    fn test_parse_numeric_and_yes() {
        let d = data(&[("enable_read_pool", "1"), ("enable_dr", "yes")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_read_pool);
        assert!(flags.enable_dr);
    }

    #[test]
    fn test_missing_keys_use_defaults() {
        let d = data(&[("enable_read_pool", "true")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        // Only read_pool changed; everything else is default
        assert!(flags.enable_read_pool);
        assert!(flags.enable_cve_scanning); // default true
        assert!(!flags.enable_dr); // default false
    }

    #[test]
    fn test_unknown_keys_ignored() {
        let d = data(&[("unknown_flag", "true"), ("enable_dr", "true")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_dr);
        // Defaults preserved for everything else
        assert!(flags.enable_cve_scanning);
    }

    #[test]
    fn test_empty_data_returns_defaults() {
        let flags = FeatureFlags::from_config_map_data(&BTreeMap::new());
        assert_eq!(flags, FeatureFlags::default());
    }

    #[test]
    fn test_case_insensitive_true() {
        let d = data(&[("enable_dr", "TRUE"), ("enable_read_pool", "True")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_dr);
        assert!(flags.enable_read_pool);
    }

    #[tokio::test]
    async fn test_shared_flags_default() {
        let shared = new_shared();
        let flags = shared.read().await;
        assert_eq!(*flags, FeatureFlags::default());
    }

    #[tokio::test]
    async fn test_shared_flags_update() {
        let shared = new_shared();
        {
            let mut flags = shared.write().await;
            flags.enable_dr = true;
        }
        let flags = shared.read().await;
        assert!(flags.enable_dr);
    }
}
