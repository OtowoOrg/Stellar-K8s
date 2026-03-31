//! Label propagation subsystem for StellarNode child resources.
//!
//! This module computes and applies a consistent set of Kubernetes labels to all
//! child resources owned by a `StellarNode`. Labels are derived from two sources:
//! - **Standard_Labels**: a fixed set of `app.kubernetes.io/*` labels computed from
//!   the `StellarNodeSpec` fields (always injected, operator-controlled).
//! - **User_Labels**: a filtered subset of `StellarNode.metadata.labels` controlled
//!   by the `LabelPropagationConfig` allow/deny policy.
//!
//! All functions are pure (no I/O) to enable property-based testing.

use std::collections::BTreeMap;

use tracing::warn;

use crate::crd::StellarNode;

/// Encapsulates label propagation logic for a single `StellarNode`.
pub struct LabelPropagator<'a> {
    node: &'a StellarNode,
}

impl<'a> LabelPropagator<'a> {
    /// Create a new `LabelPropagator` for the given node.
    pub fn new(node: &'a StellarNode) -> Self {
        Self { node }
    }

    /// Compute the five Standard_Labels derived from the node's metadata and spec.
    ///
    /// | Key | Value |
    /// |-----|-------|
    /// | `app.kubernetes.io/name` | `"stellar-node"` (constant) |
    /// | `app.kubernetes.io/instance` | `StellarNode.metadata.name` (falls back to `"unknown"`) |
    /// | `app.kubernetes.io/component` | `spec.nodeType` lowercased |
    /// | `app.kubernetes.io/managed-by` | `"stellar-operator"` (constant) |
    /// | `app.kubernetes.io/version` | `spec.version` |
    pub fn standard_labels(&self) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();

        // app.kubernetes.io/name — constant
        labels.insert(
            "app.kubernetes.io/name".to_string(),
            "stellar-node".to_string(),
        );

        // app.kubernetes.io/instance — from metadata.name, fall back to "unknown"
        let instance = match self.node.metadata.name.as_deref() {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => {
                warn!(
                    "StellarNode has no metadata.name; using \"unknown\" for \
                     app.kubernetes.io/instance"
                );
                "unknown".to_string()
            }
        };
        labels.insert("app.kubernetes.io/instance".to_string(), instance);

        // app.kubernetes.io/component — spec.nodeType lowercased
        labels.insert(
            "app.kubernetes.io/component".to_string(),
            self.node.spec.node_type.to_string().to_lowercase(),
        );

        // app.kubernetes.io/managed-by — constant
        labels.insert(
            "app.kubernetes.io/managed-by".to_string(),
            "stellar-operator".to_string(),
        );

        // app.kubernetes.io/version — spec.version
        labels.insert(
            "app.kubernetes.io/version".to_string(),
            self.node.spec.version.clone(),
        );

        labels
    }

    /// Filter `StellarNode.metadata.labels` through the `LabelPropagationConfig`.
    ///
    /// Filter evaluation order:
    /// 1. Reject keys with `kubernetes.io/` or `k8s.io/` prefix (implicit denyList)
    /// 2. If `denyList` is non-empty, reject keys matching any denyList pattern
    /// 3. If `allowList` is non-empty, accept only keys matching at least one allowList pattern
    /// 4. If `allowList` is empty, accept all remaining keys
    ///
    /// Invalid glob patterns are logged as warnings and skipped (treated as absent).
    pub fn filtered_user_labels(&self) -> BTreeMap<String, String> {
        let user_labels = match self.node.metadata.labels.as_ref() {
            Some(m) => m,
            None => return BTreeMap::new(),
        };

        let config = self
            .node
            .spec
            .label_propagation
            .as_ref()
            .cloned()
            .unwrap_or_default();

        let mut result = BTreeMap::new();

        'outer: for (key, value) in user_labels {
            // Step 1: implicit system-prefix denyList
            if key.contains("kubernetes.io/") || key.contains("k8s.io/") {
                continue;
            }

            // Step 2: user denyList
            for pattern_str in &config.deny_list {
                match glob::Pattern::new(pattern_str) {
                    Ok(pattern) => {
                        if pattern.matches(key) {
                            continue 'outer;
                        }
                    }
                    Err(e) => {
                        warn!(
                            pattern = %pattern_str,
                            error = %e,
                            "Invalid glob pattern in denyList; skipping"
                        );
                    }
                }
            }

            // Step 3: user allowList (if non-empty, key must match at least one pattern)
            if !config.allow_list.is_empty() {
                let mut matched = false;
                for pattern_str in &config.allow_list {
                    match glob::Pattern::new(pattern_str) {
                        Ok(pattern) => {
                            if pattern.matches(key) {
                                matched = true;
                                break;
                            }
                        }
                        Err(e) => {
                            warn!(
                                pattern = %pattern_str,
                                error = %e,
                                "Invalid glob pattern in allowList; skipping"
                            );
                        }
                    }
                }
                if !matched {
                    continue;
                }
            }

            // Step 4: accept
            result.insert(key.clone(), value.clone());
        }

        result
    }

    /// Compute the full Propagated_Labels map (Standard_Labels ∪ filtered User_Labels).
    /// Standard_Labels always win over User_Labels on key conflicts.
    pub fn compute(&self) -> BTreeMap<String, String> {
        let mut labels = self.filtered_user_labels();
        // Standard_Labels overwrite any conflicting user labels
        for (k, v) in self.standard_labels() {
            labels.insert(k, v);
        }
        labels
    }

    /// Merge `propagated` labels onto an existing label map, preserving keys not
    /// present in `propagated`.
    ///
    /// - Starts with a clone of `existing`.
    /// - Inserts/overwrites all keys from `propagated`.
    /// - Keys in `existing` that are NOT in `propagated` are preserved unchanged.
    ///
    /// _Requirements: 3.4, 3.5_
    pub fn merge_onto(
        existing: &BTreeMap<String, String>,
        propagated: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut result = existing.clone();
        for (k, v) in propagated {
            result.insert(k.clone(), v.clone());
        }
        result
    }

    /// Remove keys that were previously propagated but are no longer in `propagated`.
    ///
    /// - Starts with a clone of `existing`.
    /// - For each key in `previously_propagated`: if that key is NOT in `propagated`,
    ///   removes it from the result.
    /// - Keys in `existing` that were never in `previously_propagated` are preserved
    ///   (unmanaged labels).
    ///
    /// _Requirements: 4.3, 4.4_
    pub fn remove_stale_labels(
        existing: &BTreeMap<String, String>,
        propagated: &BTreeMap<String, String>,
        previously_propagated: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut result = existing.clone();
        for key in previously_propagated.keys() {
            if !propagated.contains_key(key) {
                result.remove(key);
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::types::{NodeType, StellarNetwork};
    use crate::crd::StellarNodeSpec;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn make_node(name: Option<&str>, node_type: NodeType, version: &str) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: name.map(|s| s.to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type,
                network: StellarNetwork::Testnet,
                version: version.to_string(),
                history_mode: Default::default(),
                resources: Default::default(),
                storage: Default::default(),
                validator_config: None,
                read_pool_endpoint: None,
                horizon_config: None,
                soroban_config: None,
                replicas: 1,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                pod_anti_affinity: Default::default(),
                topology_spread_constraints: None,
                cve_handling: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
                read_replica_config: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                forensic_snapshot: None,
                label_propagation: None,
                resource_meta: None,
                placement: Default::default(),
                sidecars: None,
                nat_traversal: None,
                custom_network_passphrase: None,
            },
            status: None,
        }
    }

    #[test]
    fn standard_labels_contains_all_five_keys() {
        let node = make_node(Some("my-node"), NodeType::Validator, "v21.0.0");
        let labels = LabelPropagator::new(&node).standard_labels();

        assert_eq!(
            labels.get("app.kubernetes.io/name").map(|s| s.as_str()),
            Some("stellar-node")
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(|s| s.as_str()),
            Some("my-node")
        );
        assert_eq!(
            labels
                .get("app.kubernetes.io/component")
                .map(|s| s.as_str()),
            Some("validator")
        );
        assert_eq!(
            labels
                .get("app.kubernetes.io/managed-by")
                .map(|s| s.as_str()),
            Some("stellar-operator")
        );
        assert_eq!(
            labels.get("app.kubernetes.io/version").map(|s| s.as_str()),
            Some("v21.0.0")
        );
        assert_eq!(labels.len(), 5);
    }

    #[test]
    fn standard_labels_component_is_lowercased() {
        let node = make_node(Some("horizon-1"), NodeType::Horizon, "v2.28.0");
        let labels = LabelPropagator::new(&node).standard_labels();
        assert_eq!(
            labels
                .get("app.kubernetes.io/component")
                .map(|s| s.as_str()),
            Some("horizon")
        );

        let node2 = make_node(Some("soroban-1"), NodeType::SorobanRpc, "v0.9.0");
        let labels2 = LabelPropagator::new(&node2).standard_labels();
        assert_eq!(
            labels2
                .get("app.kubernetes.io/component")
                .map(|s| s.as_str()),
            Some("sorobanrpc")
        );
    }

    #[test]
    fn standard_labels_falls_back_to_unknown_when_name_is_none() {
        let node = make_node(None, NodeType::Validator, "v21.0.0");
        let labels = LabelPropagator::new(&node).standard_labels();
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(|s| s.as_str()),
            Some("unknown")
        );
    }

    #[test]
    fn standard_labels_falls_back_to_unknown_when_name_is_empty() {
        let node = make_node(Some(""), NodeType::Validator, "v21.0.0");
        let labels = LabelPropagator::new(&node).standard_labels();
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(|s| s.as_str()),
            Some("unknown")
        );
    }

    // --- filtered_user_labels tests ---

    fn make_node_with_labels(
        labels: BTreeMap<String, String>,
        config: Option<crate::crd::types::LabelPropagationConfig>,
    ) -> StellarNode {
        let mut node = make_node(Some("test-node"), NodeType::Validator, "v21.0.0");
        node.metadata.labels = Some(labels);
        node.spec.label_propagation = config;
        node
    }

    #[test]
    fn filtered_user_labels_no_config_passes_all_non_system_labels() {
        let mut user_labels = BTreeMap::new();
        user_labels.insert("team".to_string(), "platform".to_string());
        user_labels.insert("billing/project".to_string(), "stellar".to_string());
        let node = make_node_with_labels(user_labels, None);
        let result = LabelPropagator::new(&node).filtered_user_labels();
        assert_eq!(result.get("team").map(|s| s.as_str()), Some("platform"));
        assert_eq!(
            result.get("billing/project").map(|s| s.as_str()),
            Some("stellar")
        );
    }

    #[test]
    fn filtered_user_labels_blocks_kubernetes_io_prefix() {
        let mut user_labels = BTreeMap::new();
        user_labels.insert("kubernetes.io/hostname".to_string(), "node1".to_string());
        user_labels.insert("beta.kubernetes.io/arch".to_string(), "amd64".to_string());
        user_labels.insert("team".to_string(), "platform".to_string());
        let node = make_node_with_labels(user_labels, None);
        let result = LabelPropagator::new(&node).filtered_user_labels();
        assert!(!result.contains_key("kubernetes.io/hostname"));
        assert!(!result.contains_key("beta.kubernetes.io/arch"));
        assert!(result.contains_key("team"));
    }

    #[test]
    fn filtered_user_labels_blocks_k8s_io_prefix() {
        let mut user_labels = BTreeMap::new();
        user_labels.insert("node.k8s.io/type".to_string(), "worker".to_string());
        user_labels.insert("team".to_string(), "platform".to_string());
        let node = make_node_with_labels(user_labels, None);
        let result = LabelPropagator::new(&node).filtered_user_labels();
        assert!(!result.contains_key("node.k8s.io/type"));
        assert!(result.contains_key("team"));
    }

    #[test]
    fn filtered_user_labels_deny_list_excludes_matching_keys() {
        use crate::crd::types::LabelPropagationConfig;
        let mut user_labels = BTreeMap::new();
        user_labels.insert("team".to_string(), "platform".to_string());
        user_labels.insert("billing/project".to_string(), "stellar".to_string());
        let config = LabelPropagationConfig {
            deny_list: vec!["billing/*".to_string()],
            allow_list: vec![],
        };
        let node = make_node_with_labels(user_labels, Some(config));
        let result = LabelPropagator::new(&node).filtered_user_labels();
        assert!(!result.contains_key("billing/project"));
        assert!(result.contains_key("team"));
    }

    #[test]
    fn filtered_user_labels_allow_list_restricts_to_matching_keys() {
        use crate::crd::types::LabelPropagationConfig;
        let mut user_labels = BTreeMap::new();
        user_labels.insert("team".to_string(), "platform".to_string());
        user_labels.insert("billing/project".to_string(), "stellar".to_string());
        user_labels.insert("env".to_string(), "prod".to_string());
        let config = LabelPropagationConfig {
            allow_list: vec!["billing/*".to_string()],
            deny_list: vec![],
        };
        let node = make_node_with_labels(user_labels, Some(config));
        let result = LabelPropagator::new(&node).filtered_user_labels();
        assert!(result.contains_key("billing/project"));
        assert!(!result.contains_key("team"));
        assert!(!result.contains_key("env"));
    }

    #[test]
    fn filtered_user_labels_deny_list_takes_precedence_over_allow_list() {
        use crate::crd::types::LabelPropagationConfig;
        let mut user_labels = BTreeMap::new();
        user_labels.insert("billing/project".to_string(), "stellar".to_string());
        let config = LabelPropagationConfig {
            allow_list: vec!["billing/*".to_string()],
            deny_list: vec!["billing/*".to_string()],
        };
        let node = make_node_with_labels(user_labels, Some(config));
        let result = LabelPropagator::new(&node).filtered_user_labels();
        // denyList wins — key must be absent
        assert!(!result.contains_key("billing/project"));
    }

    #[test]
    fn filtered_user_labels_invalid_glob_pattern_is_skipped() {
        use crate::crd::types::LabelPropagationConfig;
        let mut user_labels = BTreeMap::new();
        user_labels.insert("team".to_string(), "platform".to_string());
        // "[invalid" is not a valid glob pattern
        let config = LabelPropagationConfig {
            deny_list: vec!["[invalid".to_string()],
            allow_list: vec![],
        };
        let node = make_node_with_labels(user_labels, Some(config));
        // Invalid pattern is skipped — label should still pass through
        let result = LabelPropagator::new(&node).filtered_user_labels();
        assert!(result.contains_key("team"));
    }

    // --- merge_onto tests ---

    #[test]
    fn merge_onto_inserts_propagated_keys() {
        let existing: BTreeMap<String, String> = BTreeMap::new();
        let mut propagated = BTreeMap::new();
        propagated.insert(
            "app.kubernetes.io/name".to_string(),
            "stellar-node".to_string(),
        );
        let result = LabelPropagator::merge_onto(&existing, &propagated);
        assert_eq!(
            result.get("app.kubernetes.io/name").map(|s| s.as_str()),
            Some("stellar-node")
        );
    }

    #[test]
    fn merge_onto_preserves_unmanaged_keys() {
        let mut existing = BTreeMap::new();
        existing.insert("unmanaged".to_string(), "keep-me".to_string());
        let mut propagated = BTreeMap::new();
        propagated.insert(
            "app.kubernetes.io/name".to_string(),
            "stellar-node".to_string(),
        );
        let result = LabelPropagator::merge_onto(&existing, &propagated);
        assert_eq!(result.get("unmanaged").map(|s| s.as_str()), Some("keep-me"));
        assert_eq!(
            result.get("app.kubernetes.io/name").map(|s| s.as_str()),
            Some("stellar-node")
        );
    }

    #[test]
    fn merge_onto_overwrites_existing_key_with_propagated_value() {
        let mut existing = BTreeMap::new();
        existing.insert("team".to_string(), "old-value".to_string());
        let mut propagated = BTreeMap::new();
        propagated.insert("team".to_string(), "new-value".to_string());
        let result = LabelPropagator::merge_onto(&existing, &propagated);
        assert_eq!(result.get("team").map(|s| s.as_str()), Some("new-value"));
    }

    #[test]
    fn merge_onto_empty_propagated_returns_clone_of_existing() {
        let mut existing = BTreeMap::new();
        existing.insert("a".to_string(), "1".to_string());
        let propagated = BTreeMap::new();
        let result = LabelPropagator::merge_onto(&existing, &propagated);
        assert_eq!(result, existing);
    }

    // --- remove_stale_labels tests ---

    #[test]
    fn remove_stale_labels_removes_key_no_longer_in_propagated() {
        let mut existing = BTreeMap::new();
        existing.insert("team".to_string(), "platform".to_string());
        existing.insert("unmanaged".to_string(), "keep-me".to_string());

        let propagated: BTreeMap<String, String> = BTreeMap::new(); // "team" dropped

        let mut previously_propagated = BTreeMap::new();
        previously_propagated.insert("team".to_string(), "platform".to_string());

        let result =
            LabelPropagator::remove_stale_labels(&existing, &propagated, &previously_propagated);
        assert!(!result.contains_key("team"), "stale key should be removed");
        assert_eq!(
            result.get("unmanaged").map(|s| s.as_str()),
            Some("keep-me"),
            "unmanaged key must be preserved"
        );
    }

    #[test]
    fn remove_stale_labels_keeps_key_still_in_propagated() {
        let mut existing = BTreeMap::new();
        existing.insert("team".to_string(), "platform".to_string());

        let mut propagated = BTreeMap::new();
        propagated.insert("team".to_string(), "platform".to_string());

        let mut previously_propagated = BTreeMap::new();
        previously_propagated.insert("team".to_string(), "platform".to_string());

        let result =
            LabelPropagator::remove_stale_labels(&existing, &propagated, &previously_propagated);
        assert!(
            result.contains_key("team"),
            "key still in propagated must be kept"
        );
    }

    #[test]
    fn remove_stale_labels_preserves_unmanaged_keys() {
        let mut existing = BTreeMap::new();
        existing.insert("unmanaged".to_string(), "keep-me".to_string());

        let propagated: BTreeMap<String, String> = BTreeMap::new();
        let previously_propagated: BTreeMap<String, String> = BTreeMap::new();

        let result =
            LabelPropagator::remove_stale_labels(&existing, &propagated, &previously_propagated);
        assert_eq!(result.get("unmanaged").map(|s| s.as_str()), Some("keep-me"));
    }

    #[test]
    fn remove_stale_labels_empty_inputs_returns_empty() {
        let existing: BTreeMap<String, String> = BTreeMap::new();
        let propagated: BTreeMap<String, String> = BTreeMap::new();
        let previously_propagated: BTreeMap<String, String> = BTreeMap::new();
        let result =
            LabelPropagator::remove_stale_labels(&existing, &propagated, &previously_propagated);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_standard_labels_win_over_user_labels_on_conflict() {
        let mut user_labels = BTreeMap::new();
        // User tries to override a standard label
        user_labels.insert(
            "app.kubernetes.io/name".to_string(),
            "my-custom-name".to_string(),
        );
        user_labels.insert("team".to_string(), "platform".to_string());
        let node = make_node_with_labels(user_labels, None);
        let result = LabelPropagator::new(&node).compute();
        // Standard label wins
        assert_eq!(
            result.get("app.kubernetes.io/name").map(|s| s.as_str()),
            Some("stellar-node")
        );
        assert!(result.contains_key("team"));
    }
}
