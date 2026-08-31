// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Runtime feature flags loaded from the `stellar-operator-config` ConfigMap.
//!
//! The operator watches this ConfigMap and reloads flags without restart.
//!
//! Dead flags that no longer gated any code paths
//! (`enable_cve_scanning`, `enable_read_pool`, `enable_peer_discovery`,
//! `enable_archive_health`, `enable_soroban_metrics`, `enable_scp_topology`)
//! were removed. Only `enable_dr` remains — it gates Helm-rendered DR /
//! cross-region resources and is reserved for runtime DR drill scheduling.
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
//!   enable_dr: "false"
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
pub const KNOWN_FLAGS: &[&str] = &["enable_dr"];

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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FeatureFlags {
    /// Enable disaster-recovery drill scheduling / cross-region DR resources.
    pub enable_dr: bool,
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
            enable_dr: parse("enable_dr", defaults.enable_dr),
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
    if old.enable_dr != new.enable_dr {
        info!(
            configmap = configmap_name,
            flag = "enable_dr",
            old = old.enable_dr,
            new = new.enable_dr,
            "Feature flag changed"
        );
    }
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
        assert!(!flags.enable_dr);
    }

    #[test]
    fn test_parse_enable_dr_true() {
        let d = data(&[("enable_dr", "true")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_dr);
    }

    #[test]
    fn test_parse_enable_dr_false() {
        let d = data(&[("enable_dr", "false")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(!flags.enable_dr);
    }

    #[test]
    fn test_parse_numeric_and_yes() {
        let d = data(&[("enable_dr", "yes")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_dr);
    }

    #[test]
    fn test_missing_keys_use_defaults() {
        let flags = FeatureFlags::from_config_map_data(&BTreeMap::new());
        assert!(!flags.enable_dr);
    }

    #[test]
    fn test_unknown_keys_ignored() {
        let d = data(&[("unknown_flag", "true"), ("enable_dr", "true")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_dr);
    }

    #[test]
    fn test_empty_data_returns_defaults() {
        let flags = FeatureFlags::from_config_map_data(&BTreeMap::new());
        assert_eq!(flags, FeatureFlags::default());
    }

    #[test]
    fn test_case_insensitive_true() {
        let d = data(&[("enable_dr", "TRUE")]);
        let flags = FeatureFlags::from_config_map_data(&d);
        assert!(flags.enable_dr);
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

    #[test]
    fn known_flags_covers_all_struct_fields() {
        assert!(KNOWN_FLAGS.contains(&"enable_dr"));
        assert_eq!(KNOWN_FLAGS.len(), 1);
    }

    #[test]
    fn validate_returns_no_warnings_for_known_keys() {
        let d = data(&[("enable_dr", "yes")]);
        let warnings = validate_config_map_data(&d);
        assert!(
            warnings.is_empty(),
            "expected no warnings, got: {warnings:?}"
        );
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
    fn validate_warns_on_dead_flag_keys() {
        // Former flags must now surface as unknown so operators notice removals.
        let d = data(&[
            ("enable_cve_scanning", "true"),
            ("enable_read_pool", "true"),
            ("enable_peer_discovery", "true"),
            ("enable_archive_health", "true"),
            ("enable_soroban_metrics", "true"),
            ("enable_scp_topology", "true"),
        ]);
        let warnings = validate_config_map_data(&d);
        assert_eq!(warnings.len(), 6);
        assert!(warnings
            .iter()
            .all(|w| matches!(w, FlagValidationWarning::UnknownKey(_))));
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
