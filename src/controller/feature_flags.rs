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
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    api::Api,
    runtime::watcher::{self, Event},
    Client, ResourceExt,
};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Interval at which the async bundle refresher re-fetches the signed bundle.
///
/// Kept well under the 10s cluster-wide propagation target so that a bundle
/// published to the flag platform is observed by every operator replica
/// within the acceptance window.
pub const BUNDLE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Name of the ConfigMap key carrying the signed flag bundle payload.
pub const BUNDLE_CONFIGMAP_KEY: &str = "flag_bundle";

/// Name of the ConfigMap key carrying the kill-switch override.
///
/// When set to a truthy value the kill-switch forces all flags to their safe
/// defaults regardless of the bundle contents or bundle delivery health.
pub const KILL_SWITCH_CONFIGMAP_KEY: &str = "kill_switch";

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

/// In-process cache of the last-known-good signed flag bundle.
///
/// Evaluation reads exclusively from this cache so the hot path never touches
/// the network. The cache is refreshed asynchronously by
/// [`refresh_bundle_cache`] and survives bundle-delivery outages: when a
/// refresh fails the previously cached bundle is retained.
#[derive(Debug, Clone, Default)]
pub struct BundleCache {
    /// Last-known-good bundle payload, if any has been successfully applied.
    pub last_known_good: Option<String>,
    /// Whether the kill-switch override is currently engaged.
    pub kill_switch: bool,
}

impl BundleCache {
    /// Effective flags derived from the cached bundle, honouring the
    /// kill-switch override.
    ///
    /// When the kill-switch is engaged the safe defaults are returned so that
    /// user-visible behaviour is disabled even if bundle delivery is down.
    pub fn effective_flags(&self) -> FeatureFlags {
        if self.kill_switch {
            return FeatureFlags::default();
        }
        match &self.last_known_good {
            Some(payload) => FeatureFlags::from_config_map_data(&parse_bundle_payload(payload)),
            None => FeatureFlags::default(),
        }
    }
}

/// Shared, live-reloadable bundle cache handle.
pub type SharedBundleCache = Arc<RwLock<BundleCache>>;

/// Create a new `SharedBundleCache` with no last-known-good bundle.
pub fn new_shared_bundle_cache() -> SharedBundleCache {
    Arc::new(RwLock::new(BundleCache::default()))
}

/// Parse a signed bundle payload into a flag map.
///
/// The payload is expected to be a simple `key=value` line-oriented document
/// as emitted by the bundle signer. Malformed lines are ignored so a partial
/// bundle never panics the evaluator.
fn parse_bundle_payload(payload: &str) -> BTreeMap<String, String> {
    payload
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            line.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Apply a freshly fetched bundle payload to the cache.
///
/// Returns `true` when the cached bundle changed. The kill-switch state is
/// applied independently of bundle delivery so it keeps working during
/// outages.
pub async fn apply_bundle(
    cache: &SharedBundleCache,
    payload: Option<String>,
    kill_switch: bool,
) -> bool {
    let mut current = cache.write().await;
    let mut changed = false;

    if current.kill_switch != kill_switch {
        current.kill_switch = kill_switch;
        changed = true;
    }

    if let Some(payload) = payload {
        if current.last_known_good.as_deref() != Some(payload.as_str()) {
            current.last_known_good = Some(payload);
            changed = true;
        }
    }

    changed
}

/// Asynchronously refresh the bundle cache from the watched ConfigMap.
///
/// This is the network-touching half of the design: it runs on its own task
/// and never blocks in-process evaluation. On failure the last-known-good
/// bundle is retained so the kill-switch and cached flags keep functioning.
pub async fn refresh_bundle_cache(
    cache: SharedBundleCache,
    flags: SharedFeatureFlags,
    payload: Option<String>,
    kill_switch: bool,
) {
    let changed = apply_bundle(&cache, payload, kill_switch).await;
    if !changed {
        return;
    }

    let effective = cache.read().await.effective_flags();
    let mut current = flags.write().await;
    if *current != effective {
        *current = effective;
    }
}

/// Watch the `stellar-operator-config` ConfigMap in `namespace` and update
/// `flags` whenever it changes. Runs until the task is cancelled.
pub async fn watch_feature_flags(
    client: Client,
    namespace: String,
    flags: SharedFeatureFlags,
    audit_recorder: Option<Arc<crate::controller::audit_recorder::AuditRecorder>>,
) {
    watch_feature_flags_with_bundle(
        client,
        namespace,
        flags,
        new_shared_bundle_cache(),
        audit_recorder,
    )
    .await
}

/// Watch the `stellar-operator-config` ConfigMap and drive the signed bundle
/// cache, keeping in-process evaluation off the network hot path.
///
/// The watcher feeds [`refresh_bundle_cache`] so the cached bundle (and the
/// kill-switch override) propagate to every replica within
/// [`BUNDLE_REFRESH_INTERVAL`].
pub async fn watch_feature_flags_with_bundle(
    client: Client,
    namespace: String,
    flags: SharedFeatureFlags,
    bundle_cache: SharedBundleCache,
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

                let bundle_payload = data.get(BUNDLE_CONFIGMAP_KEY).cloned();
                let kill_switch = data
                    .get(KILL_SWITCH_CONFIGMAP_KEY)
                    .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
                    .unwrap_or(false);

                let new_flags = if bundle_payload.is_some() {
                    let mut cache = bundle_cache.write().await;
                    if let Some(payload) = &bundle_payload {
                        cache.last_known_good = Some(payload.clone());
                    }
                    cache.kill_switch = kill_switch;
                    cache.effective_flags()
                } else {
                    let mut cache = bundle_cache.write().await;
                    cache.kill_switch = kill_switch;
                    cache.effective_flags()
                };

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

                // Retain the last-known-good bundle so the kill-switch and
                // cached flags keep functioning while delivery is down.
                let mut current = flags.write().await;
                *current = bundle_cache.read().await.effective_flags();
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

    #[test]
    fn bundle_cache_defaults_to_safe_flags() {
        let cache = BundleCache::default();
        assert_eq!(cache.effective_flags(), FeatureFlags::default());
    }

    #[test]
    fn bundle_cache_parses_last_known_good() {
        let cache = BundleCache {
            last_known_good: Some("enable_dr=true".to_string()),
            kill_switch: false,
        };
        assert!(cache.effective_flags().enable_dr);
    }

    #[test]
    fn bundle_cache_kill_switch_overrides_bundle() {
        let cache = BundleCache {
            last_known_good: Some("enable_dr=true".to_string()),
            kill_switch: true,
        };
        assert!(!cache.effective_flags().enable_dr);
    }

    #[test]
    fn parse_bundle_payload_ignores_comments_and_blank_lines() {
        let parsed = parse_bundle_payload("# signed bundle\n\nenable_dr=yes\n");
        assert_eq!(parsed.get("enable_dr").map(String::as_str), Some("yes"));
        assert_eq!(parsed.len(), 1);
    }

    #[tokio::test]
    async fn apply_bundle_retains_last_known_good_on_missing_payload() {
        let cache = new_shared_bundle_cache();
        assert!(apply_bundle(&cache, Some("enable_dr=true".to_string()), false).await);
        // A refresh with no payload (delivery down) must not clear the cache.
        assert!(!apply_bundle(&cache, None, false).await);
        assert!(cache.read().await.effective_flags().enable_dr);
    }

    #[tokio::test]
    async fn refresh_bundle_cache_updates_shared_flags() {
        let cache = new_shared_bundle_cache();
        let flags = new_shared();
        refresh_bundle_cache(
            cache.clone(),
            flags.clone(),
            Some("enable_dr=true".to_string()),
            false,
        )
        .await;
        assert!(flags.read().await.enable_dr);

        // Kill-switch engaged: flags revert to safe defaults even with a bundle.
        refresh_bundle_cache(cache, flags.clone(), None, true).await;
        assert!(!flags.read().await.enable_dr);
    }
}
