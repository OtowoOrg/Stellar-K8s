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

/// All recognised feature-flag keys.
///
/// Any key present in the ConfigMap that is *not* listed here is treated as
/// unknown and surfaced as a [`FlagValidationWarning::UnknownKey`].
pub const KNOWN_FLAGS: &[&str] = &[
    "enable_cve_scanning",
    "enable_read_pool",
    "enable_dr",
    "enable_peer_discovery",
    "enable_archive_health",
    "enable_soroban_metrics",
];

/// A warning produced by [`validate_config_map_data`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagValidationWarning {
    /// The ConfigMap contains a key that is not in [`KNOWN_FLAGS`].
    UnknownKey(String),
    /// A recognised key has a value that is not a recognised boolean string.
    InvalidValue { key: String, value: String },
}

impl std::fmt::Display for FlagValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey(key) => {
                write!(f, "unknown feature flag '{key}' — will be ignored")
            }
            Self::InvalidValue { key, value } => {
                write!(
                    f,
                    "invalid value '{value}' for flag '{key}' — expected true/false/1/0/yes/no; reverting to default"
                )
            }
        }
    }
}

/// Validate `data` from a feature-flags ConfigMap against the registry.
///
/// Returns a (possibly empty) list of warnings for unknown keys and
/// unrecognised boolean values. The warnings are purely advisory: callers
/// can choose to log them and proceed, or surface them as status conditions.
/// This function never mutates or rejects entries — that is left to
/// [`FeatureFlags::from_config_map_data`].
pub fn validate_config_map_data(data: &BTreeMap<String, String>) -> Vec<FlagValidationWarning> {
    let valid_bool_values = ["true", "false", "1", "0", "yes", "no"];
    let mut warnings = Vec::new();

    for (key, value) in data {
        if !KNOWN_FLAGS.contains(&key.as_str()) {
            warnings.push(FlagValidationWarning::UnknownKey(key.clone()));
        } else if !valid_bool_values.contains(&value.to_lowercase().as_str()) {
            warnings.push(FlagValidationWarning::InvalidValue {
                key: key.clone(),
                value: value.clone(),
            });
        }
    }

    warnings
}

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
pub async fn watch_feature_flags(
    client: Client,
    namespace: String,
    flags: SharedFeatureFlags,
    audit_recorder: Option<Arc<crate::controller::audit_recorder::AuditRecorder>>,
) {
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

                for warning in validate_config_map_data(&data) {
                    warn!(
                        configmap = FEATURE_FLAGS_CONFIGMAP,
                        warning = %warning,
                        "Feature-flag ConfigMap validation warning"
                    );
                }

                let new_flags = FeatureFlags::from_config_map_data(&data);

                let mut current = flags.write().await;
                if *current != new_flags {
                    log_flag_changes(&current, &new_flags, cm.name_any().as_str());

                    if let Some(recorder) = &audit_recorder {
                        use crate::controller::audit_log::{AdminAction, AuditEntry};
                        let actor = extract_actor(&cm);
                        let entry = AuditEntry::new(
                            AdminAction::ConfigUpdate,
                            actor,
                            FEATURE_FLAGS_CONFIGMAP,
                            namespace.clone(),
                            Some("Feature flags updated in ConfigMap"),
                        )
                        .with_diff(serde_json::to_value(&data).unwrap_or_default());

                        recorder.record(entry).await;
                    }

                    *current = new_flags;
                }
            }
            Ok(Event::Delete(cm)) => {
                warn!(
                    configmap = FEATURE_FLAGS_CONFIGMAP,
                    "Feature-flags ConfigMap deleted; reverting to defaults"
                );

                if let Some(recorder) = &audit_recorder {
                    use crate::controller::audit_log::{AdminAction, AuditEntry};
                    let actor = extract_actor(&cm);
                    let entry = AuditEntry::new(
                        AdminAction::ConfigDelete,
                        actor,
                        FEATURE_FLAGS_CONFIGMAP,
                        namespace.clone(),
                        Some("Feature flags ConfigMap deleted"),
                    );
                    recorder.record(entry).await;
                }

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

fn extract_actor(cm: &ConfigMap) -> String {
    if let Some(managed) = &cm.metadata.managed_fields {
        for field in managed.iter().rev() {
            if let Some(manager) = &field.manager {
                if manager != "stellar-operator" && manager != "kube-controller-manager" {
                    return manager.clone();
                }
            }
        }
    }
    "system:unknown".to_string()
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

    // ── Registry and validation tests ─────────────────────────────────────────

    #[test]
    fn known_flags_covers_all_struct_fields() {
        // Every field on FeatureFlags must appear in KNOWN_FLAGS.
        let all_keys: Vec<&str> = vec![
            "enable_cve_scanning",
            "enable_read_pool",
            "enable_dr",
            "enable_peer_discovery",
            "enable_archive_health",
            "enable_soroban_metrics",
        ];
        for key in &all_keys {
            assert!(
                KNOWN_FLAGS.contains(key),
                "'{key}' is missing from KNOWN_FLAGS"
            );
        }
    }

    #[test]
    fn validate_returns_no_warnings_for_known_keys() {
        let d = data(&[
            ("enable_cve_scanning", "true"),
            ("enable_read_pool", "false"),
            ("enable_dr", "yes"),
            ("enable_peer_discovery", "1"),
            ("enable_archive_health", "0"),
            ("enable_soroban_metrics", "no"),
        ]);
        let warnings = validate_config_map_data(&d);
        assert!(warnings.is_empty(), "expected no warnings, got: {warnings:?}");
    }

    #[test]
    fn validate_warns_on_unknown_key() {
        let d = data(&[("enable_dr", "true"), ("unknown_feature", "true")]);
        let warnings = validate_config_map_data(&d);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0],
            FlagValidationWarning::UnknownKey("unknown_feature".to_string())
        );
    }

    #[test]
    fn validate_warns_on_multiple_unknown_keys() {
        let d = data(&[
            ("enable_dr", "true"),
            ("foo_flag", "true"),
            ("bar_flag", "false"),
        ]);
        let mut warnings = validate_config_map_data(&d);
        warnings.sort_by_key(|w| format!("{w:?}"));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn validate_warns_on_invalid_bool_value() {
        let d = data(&[("enable_dr", "enabled")]);
        let warnings = validate_config_map_data(&d);
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            &warnings[0],
            FlagValidationWarning::InvalidValue { key, .. } if key == "enable_dr"
        ));
    }

    #[test]
    fn validate_empty_data_returns_no_warnings() {
        let warnings = validate_config_map_data(&BTreeMap::new());
        assert!(warnings.is_empty());
    }

    #[test]
    fn flag_validation_warning_display_includes_key_name() {
        let w = FlagValidationWarning::UnknownKey("mystery_flag".to_string());
        assert!(w.to_string().contains("mystery_flag"));

        let w2 = FlagValidationWarning::InvalidValue {
            key: "enable_dr".to_string(),
            value: "enabled".to_string(),
        };
        let s = w2.to_string();
        assert!(s.contains("enable_dr"));
        assert!(s.contains("enabled"));
    }
}
