//! Operator-driven, ConfigMap-backed configuration for selectively enabling the
//! Reentrancy Guard on specific namespaces or contract IDs.
//!
//! The Stellar-K8s operator can carry a Kubernetes `ConfigMap` whose data keys
//! select the scope in which the guard is *enforced* (an allow-list) and the
//! scope in which it is *explicitly disabled* (a deny/disable-list). Because a
//! Soroban sub-contract cannot read a cluster `ConfigMap` directly, this module
//! parses the *same* JSON document that the operator's admission webhook would
//! attach to the validation input, so the guard can be toggled per namespace or
//! per contract ID without rebuilding or re-deploying the operator.
//!
//! # Semantics
//!
//! - If both lists are empty, the guard is **enabled everywhere** (the safe
//!   default for high-value deployments).
//! - `enabled_namespaces` / `enabled_contracts`: if either is non-empty, only
//!   matching scopes are guarded (no-op elsewhere → zero false positives on
//!   deployments that opted out).
//! - `disabled_namespaces` / `disabled_contracts`: matching scopes are **never**
//!   guarded, regardless of the enabled lists (explicit opt-out).
//! - Explicit `disabled` rules take precedence over `enabled` rules.
//!
//! The operator webhook is expected to expose this configuration through a
//! `ConfigMap` data key such as `reentrancy-guard.json` and pass the parsed
//! [`ReentrancyGuardConfig`] into the middleware before a guarded invocation.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// Parsed runtime configuration for the reentrancy-guard middleware.
///
/// Wire format is JSON (suitable to live under a `ConfigMap` data key). Unknown
/// fields are ignored, so future operator fields are forward-compatible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReentrancyGuardConfig {
    /// Namespaces on which the guard is enforced. Empty means "all namespaces".
    pub enabled_namespaces: Vec<String>,
    /// Contract IDs (hex, without `C` prefix) on which the guard is enforced.
    pub enabled_contracts: Vec<String>,
    /// Namespaces on which the guard is explicitly disabled.
    pub disabled_namespaces: Vec<String>,
    /// Contract IDs on which the guard is explicitly disabled.
    pub disabled_contracts: Vec<String>,
}

impl ReentrancyGuardConfig {
    /// Parse from a JSON document (e.g. the `ConfigMap` data value).
    pub fn from_json(blob: &str) -> Result<Self, String> {
        serde_json::from_str(blob).map_err(|e| format!("invalid reentrancy-guard config: {e}"))
    }

    /// Whether the guard should be applied to a cross-contract call whose
    /// substrate is `namespace` / `contract_id`.
    ///
    /// Returns `false` (no guard) only when an explicit disable matches, or
    /// when enable-lists are non-empty and none of them match. Never returns
    /// `false` for a scope the operator wants protected.
    pub fn is_enabled_for(&self, namespace: &str, contract_id: &str) -> bool {
        // Explicit opt-outs always win.
        if self.disabled_namespaces.iter().any(|n| n == namespace) {
            return false;
        }
        if self.disabled_contracts.iter().any(|c| c == contract_id) {
            return false;
        }

        // If no scope was explicitly selected, the safe default is "enabled".
        let wants_namespace = !self.enabled_namespaces.is_empty();
        let wants_contract = !self.enabled_contracts.is_empty();
        if !wants_namespace && !wants_contract {
            return true;
        }

        wants_namespace && self.enabled_namespaces.iter().any(|n| n == namespace)
            || wants_contract && self.enabled_contracts.iter().any(|c| c == contract_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let cfg = ReentrancyGuardConfig {
            enabled_namespaces: vec!["payments".into()],
            enabled_contracts: vec!["a1b2".into()],
            disabled_namespaces: vec!["legacy".into()],
            disabled_contracts: vec!["cafe".into()],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed = ReentrancyGuardConfig::from_json(&json).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn default_is_enabled_everywhere() {
        let cfg = ReentrancyGuardConfig::default();
        assert!(cfg.is_enabled_for("any-ns", "any-contract"));
    }

    #[test]
    fn namespace_allowlist_scopes_guard() {
        let cfg: ReentrancyGuardConfig =
            serde_json::from_str(r#"{"enabled_namespaces":["payments","settlement"]}"#).unwrap();
        assert!(cfg.is_enabled_for("payments", "x"));
        assert!(cfg.is_enabled_for("settlement", "x"));
        // Not on the allow-list: no guard → the operator incurs zero overhead
        // and zero false positives for deployments that opted out.
        assert!(!cfg.is_enabled_for("unrelated", "x"));
    }

    #[test]
    fn contract_allowlist_scopes_guard() {
        let cfg: ReentrancyGuardConfig =
            serde_json::from_str(r#"{"enabled_contracts":["CAFEBABE"]}"#).unwrap();
        assert!(cfg.is_enabled_for("any", "CAFEBABE"));
        assert!(!cfg.is_enabled_for("any", "DEADBEEF"));
    }

    #[test]
    fn disabled_overrides_enabled() {
        let cfg: ReentrancyGuardConfig = serde_json::from_str(
            r#"{"enabled_namespaces":["payments"],"disabled_contracts":["trickyfx"]}"#,
        )
        .unwrap();
        assert!(cfg.is_enabled_for("payments", "aaa"));
        // Same namespace but the contract opted out: no guard.
        assert!(!cfg.is_enabled_for("payments", "trickyfx"));
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        // Forward compatibility: future operator fields must not break parsing.
        let cfg = ReentrancyGuardConfig::from_json(
            r#"{"enabled_namespaces":["payments"],"future_field":123}"#,
        )
        .unwrap();
        assert!(cfg.is_enabled_for("payments", "abc"));
        assert!(ReentrancyGuardConfig::from_json("not json").is_err());
    }
}
