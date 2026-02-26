//! Comprehensive tests for the remediation module (issue #172)
//!
//! Covers: health-check triggered remediation, idempotency, cooldown enforcement,
//! correct K8s resource selection per remediation type, and annotation constants.

#[cfg(test)]
mod tests {
    use super::super::remediation::*;
    use crate::crd::{NodeType, StellarNetwork, StellarNode, StellarNodeSpec};
    use chrono::{Duration, Utc};
    use kube::api::ObjectMeta;
    use std::collections::BTreeMap;

    fn make_node(annotations: BTreeMap<String, String>) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("default".to_string()),
                annotations: Some(annotations),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                history_mode: Default::default(),
                resources: Default::default(),
                storage: Default::default(),
                validator_config: None,
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
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
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

    fn make_bare_node() -> StellarNode {
        make_node(BTreeMap::new())
    }

    // ── 1. Triggering remediation when health check fails ───────────────

    #[test]
    fn test_stale_node_triggers_restart_on_first_failure() {
        let thirty_min_ago = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), thirty_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(result.is_stale);
        assert_eq!(result.recommended_action, RemediationLevel::Restart);
        assert_eq!(result.current_ledger, Some(100));
        assert_eq!(result.last_observed_ledger, Some(100));
        assert!(result.minutes_since_progress.unwrap() >= 30);
    }

    #[test]
    fn test_stale_node_triggers_clear_and_resync_after_restart() {
        let thirty_min_ago = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), thirty_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "1".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(result.is_stale);
        assert_eq!(result.recommended_action, RemediationLevel::ClearAndResync);
    }

    #[test]
    fn test_no_remediation_when_ledger_progressing() {
        let five_min_ago = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), five_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        // current_ledger (200) > last_ledger (100) — ledger progressed
        let result = check_stale_node(&node, Some(200));

        assert!(!result.is_stale);
        assert_eq!(result.recommended_action, RemediationLevel::None);
        assert_eq!(result.current_ledger, Some(200));
    }

    #[test]
    fn test_no_remediation_when_no_current_ledger() {
        let thirty_min_ago = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), thirty_min_ago);

        let node = make_node(ann);
        let result = check_stale_node(&node, None);

        assert!(!result.is_stale);
        assert_eq!(result.recommended_action, RemediationLevel::None);
        assert_eq!(result.current_ledger, None);
    }

    // ── 2. Idempotency ─────────────────────────────────────────────────

    #[test]
    fn test_healthy_result_for_node_within_threshold() {
        // Stuck for 5 minutes — below the 15-minute stale threshold
        let five_min_ago = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), five_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(!result.is_stale, "node stuck <15 min must not be stale");
        assert_eq!(result.recommended_action, RemediationLevel::None);
    }

    #[test]
    fn test_stale_check_on_already_remediated_node_escalates() {
        // Node was already restarted (level=1) but is still stuck → escalate
        let twenty_min_ago = (Utc::now() - Duration::minutes(20)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), twenty_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "1".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(result.is_stale);
        assert_eq!(
            result.recommended_action,
            RemediationLevel::ClearAndResync,
            "must escalate to ClearAndResync, not circle back to Restart"
        );
    }

    // ── 3. Cooldown ────────────────────────────────────────────────────

    #[test]
    fn test_can_remediate_when_no_previous_remediation() {
        let node = make_bare_node();
        assert!(
            can_remediate(&node),
            "no prior remediation → must allow remediation"
        );
    }

    #[test]
    fn test_cannot_remediate_within_cooldown() {
        let five_min_ago = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(REMEDIATION_TIME_ANNOTATION.to_string(), five_min_ago);

        let node = make_node(ann);
        assert!(
            !can_remediate(&node),
            "remediation 5 min ago is within the 10 min cooldown"
        );
    }

    #[test]
    fn test_can_remediate_after_cooldown_expires() {
        let fifteen_min_ago = (Utc::now() - Duration::minutes(15)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(REMEDIATION_TIME_ANNOTATION.to_string(), fifteen_min_ago);

        let node = make_node(ann);
        assert!(
            can_remediate(&node),
            "15 min since last remediation exceeds the 10 min cooldown"
        );
    }

    // ── 4. Correct K8s resource selection per remediation type ──────────

    #[test]
    fn test_remediation_level_ordering() {
        assert!(RemediationLevel::None < RemediationLevel::Restart);
        assert!(RemediationLevel::Restart < RemediationLevel::ClearAndResync);
        assert!(RemediationLevel::None < RemediationLevel::ClearAndResync);
    }

    #[test]
    fn test_level_none_selected_for_healthy_node() {
        let now_str = Utc::now().to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "50".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), now_str);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        // Ledger advanced from 50 → 60
        let result = check_stale_node(&node, Some(60));

        assert!(!result.is_stale);
        assert_eq!(result.recommended_action, RemediationLevel::None);
    }

    #[test]
    fn test_level_restart_is_first_response() {
        let thirty_min_ago = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), thirty_min_ago);
        // level 0 — no prior remediation
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert_eq!(
            result.recommended_action,
            RemediationLevel::Restart,
            "first failure must trigger pod restart (Restart), not deployment rollout"
        );
    }

    #[test]
    fn test_level_clear_and_resync_for_persistent_failure() {
        let thirty_min_ago = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(
            LAST_LEDGER_TIME_ANNOTATION.to_string(),
            thirty_min_ago.clone(),
        );
        // level 1 — already restarted once
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "1".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert_eq!(
            result.recommended_action,
            RemediationLevel::ClearAndResync,
            "persistent failure after restart must escalate to ClearAndResync (deployment rollout)"
        );

        // Also verify level 2+ stays at ClearAndResync
        let mut ann2 = BTreeMap::new();
        ann2.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann2.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), thirty_min_ago);
        ann2.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "2".to_string());

        let node2 = make_node(ann2);
        let result2 = check_stale_node(&node2, Some(100));
        assert_eq!(result2.recommended_action, RemediationLevel::ClearAndResync);
    }

    #[test]
    fn test_remediation_level_as_str() {
        assert_eq!(RemediationLevel::None.as_str(), "None");
        assert_eq!(RemediationLevel::Restart.as_str(), "Restart");
        assert_eq!(RemediationLevel::ClearAndResync.as_str(), "ClearAndResync");
    }

    // ── 5. Annotation constants ────────────────────────────────────────

    #[test]
    fn test_annotation_keys_are_consistent() {
        assert_eq!(LAST_LEDGER_ANNOTATION, "stellar.org/last-observed-ledger");
        assert_eq!(
            LAST_LEDGER_TIME_ANNOTATION,
            "stellar.org/last-ledger-update-time"
        );
        assert_eq!(
            REMEDIATION_LEVEL_ANNOTATION,
            "stellar.org/remediation-level"
        );
        assert_eq!(
            REMEDIATION_TIME_ANNOTATION,
            "stellar.org/last-remediation-time"
        );

        // All must live under the stellar.org/ prefix
        for key in [
            LAST_LEDGER_ANNOTATION,
            LAST_LEDGER_TIME_ANNOTATION,
            REMEDIATION_LEVEL_ANNOTATION,
            REMEDIATION_TIME_ANNOTATION,
        ] {
            assert!(
                key.starts_with("stellar.org/"),
                "annotation {key} must use the stellar.org/ prefix"
            );
        }
    }

    // ── Additional edge-case coverage ──────────────────────────────────

    #[test]
    fn test_remediation_level_from_u8_roundtrip() {
        assert_eq!(RemediationLevel::from_u8(0), RemediationLevel::None);
        assert_eq!(RemediationLevel::from_u8(1), RemediationLevel::Restart);
        assert_eq!(
            RemediationLevel::from_u8(2),
            RemediationLevel::ClearAndResync
        );
        // Out-of-range values saturate at ClearAndResync
        assert_eq!(
            RemediationLevel::from_u8(255),
            RemediationLevel::ClearAndResync
        );
    }

    #[test]
    fn test_stale_check_healthy_constructor() {
        let h = StaleCheckResult::healthy(Some(42));
        assert!(!h.is_stale);
        assert_eq!(h.current_ledger, Some(42));
        assert_eq!(h.last_observed_ledger, Some(42));
        assert_eq!(h.minutes_since_progress, Some(0));
        assert_eq!(h.recommended_action, RemediationLevel::None);

        let h_none = StaleCheckResult::healthy(None);
        assert_eq!(h_none.current_ledger, None);
    }

    #[test]
    fn test_stale_check_stale_constructor() {
        let s = StaleCheckResult::stale(Some(100), Some(100), 30, RemediationLevel::Restart);
        assert!(s.is_stale);
        assert_eq!(s.current_ledger, Some(100));
        assert_eq!(s.last_observed_ledger, Some(100));
        assert_eq!(s.minutes_since_progress, Some(30));
        assert_eq!(s.recommended_action, RemediationLevel::Restart);
    }

    #[test]
    fn test_node_with_no_annotations_is_healthy() {
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("bare-node".to_string()),
                namespace: Some("default".to_string()),
                annotations: None,
                ..Default::default()
            },
            spec: make_bare_node().spec,
            status: None,
        };

        let result = check_stale_node(&node, Some(100));
        assert!(
            !result.is_stale,
            "first observation with no prior ledger → healthy"
        );
        assert_eq!(result.recommended_action, RemediationLevel::None);
    }

    #[test]
    fn test_stale_node_at_exact_threshold_boundary() {
        // Exactly 15 minutes — should trigger remediation (>=)
        let exactly_15_min_ago = (Utc::now() - Duration::minutes(15)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), exactly_15_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(
            result.is_stale,
            "exactly at threshold must be stale (>= check)"
        );
        assert_eq!(result.recommended_action, RemediationLevel::Restart);
    }

    #[test]
    fn test_cooldown_at_exact_boundary() {
        // Exactly 10 minutes ago — should allow remediation (>=)
        let exactly_10_min_ago = (Utc::now() - Duration::minutes(10)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(REMEDIATION_TIME_ANNOTATION.to_string(), exactly_10_min_ago);

        let node = make_node(ann);
        assert!(
            can_remediate(&node),
            "exactly at cooldown boundary must allow remediation (>= check)"
        );
    }

    // ── Edge-case and boundary tests ───────────────────────────────────

    #[test]
    fn test_stale_check_with_empty_annotations() {
        let node = make_node(BTreeMap::new());
        let result = check_stale_node(&node, Some(100));

        assert!(!result.is_stale);
        assert_eq!(result.recommended_action, RemediationLevel::None);
        assert_eq!(result.current_ledger, Some(100));
    }

    #[test]
    fn test_stale_check_with_malformed_ledger_annotation() {
        let twenty_min_ago = (Utc::now() - Duration::minutes(20)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(
            LAST_LEDGER_ANNOTATION.to_string(),
            "not-a-number".to_string(),
        );
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), twenty_min_ago);

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(
            !result.is_stale,
            "malformed ledger annotation should parse as None, falling through to healthy"
        );
        assert_eq!(result.recommended_action, RemediationLevel::None);
    }

    #[test]
    fn test_stale_check_with_malformed_time_annotation() {
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(
            LAST_LEDGER_TIME_ANNOTATION.to_string(),
            "garbage-timestamp".to_string(),
        );

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(
            !result.is_stale,
            "invalid time annotation should parse as None, falling through to healthy"
        );
        assert_eq!(result.recommended_action, RemediationLevel::None);
    }

    #[test]
    fn test_stale_check_one_minute_below_threshold() {
        let fourteen_min_ago = (Utc::now() - Duration::minutes(14)).to_rfc3339();
        let mut ann = BTreeMap::new();
        ann.insert(LAST_LEDGER_ANNOTATION.to_string(), "100".to_string());
        ann.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), fourteen_min_ago);
        ann.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());

        let node = make_node(ann);
        let result = check_stale_node(&node, Some(100));

        assert!(
            !result.is_stale,
            "14 minutes is below the 15-minute threshold"
        );
        assert_eq!(result.recommended_action, RemediationLevel::None);
    }

    #[test]
    fn test_can_remediate_with_malformed_time() {
        let mut ann = BTreeMap::new();
        ann.insert(
            REMEDIATION_TIME_ANNOTATION.to_string(),
            "garbage".to_string(),
        );

        let node = make_node(ann);
        assert!(
            can_remediate(&node),
            "unparseable remediation time should be treated as no previous remediation"
        );
    }

    #[test]
    fn test_remediation_level_from_u8_boundary() {
        assert_eq!(
            RemediationLevel::from_u8(3),
            RemediationLevel::ClearAndResync,
            "first value above ClearAndResync(2) must saturate"
        );
        assert_eq!(
            RemediationLevel::from_u8(255),
            RemediationLevel::ClearAndResync,
            "u8::MAX must saturate to ClearAndResync"
        );
    }
}
