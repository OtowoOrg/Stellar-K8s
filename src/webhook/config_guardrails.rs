//! Guardrails for high-risk configuration combinations
//!
//! Detects combinations of `StellarNodeSpec` fields that are individually valid
//! but, when paired, create stability, security, or availability hazards in
//! production. Each violation carries a severity, a human-readable message,
//! and a remediation hint.
//!
//! # Integration
//!
//! Call [`check_config_guardrails`] from the admission webhook pipeline after
//! the per-field validators have already run. Guardrail violations at
//! [`Severity::Error`] should be treated as admission rejections; those at
//! [`Severity::Warning`] may be emitted as Kubernetes Events instead.
//!
//! # Guardrails enforced
//!
//! | # | Combination | Severity |
//! |---|-------------|----------|
//! | G01 | VPA + HPA both enabled | Error |
//! | G02 | HPA enabled on a Validator node (quorum-set membership makes horizontal scale unsafe) | Error |
//! | G03 | `maintenance_mode=true` on Mainnet validator | Warning |
//! | G04 | `suspended=true` on Mainnet | Warning |
//! | G05 | Validator replicas < 3 on Mainnet | Error |
//! | G06 | DR config absent on Mainnet validator | Warning |
//! | G07 | Network policy absent on Mainnet | Warning |
//! | G08 | Custom network with no passphrase source | Error |
//! | G09 | `restore_from_snapshot` set while `suspended=false` on Mainnet | Warning |
//! | G10 | Horizon on Mainnet with no external or managed database | Warning |

use crate::crd::{NodeType, StellarNetwork, StellarNode};

/// Severity of a guardrail violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Should block admission in production; must be fixed before deploy.
    Error,
    /// Surfaced as a Kubernetes Event / warning annotation; does not block.
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
        }
    }
}

/// A single guardrail violation.
#[derive(Debug, Clone)]
pub struct GuardrailViolation {
    /// Short identifier (e.g. "G01") for alert routing.
    pub code: &'static str,
    pub severity: Severity,
    /// Human-readable description of why this combination is risky.
    pub message: String,
    /// Actionable fix for the operator.
    pub hint: &'static str,
}

impl GuardrailViolation {
    fn new(
        code: &'static str,
        severity: Severity,
        message: impl Into<String>,
        hint: &'static str,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            hint,
        }
    }
}

/// Run all guardrail checks against a [`StellarNode`] and return every violation found.
///
/// An empty return value means the configuration is safe to admit.
pub fn check_config_guardrails(node: &StellarNode) -> Vec<GuardrailViolation> {
    let spec = &node.spec;
    let is_mainnet = matches!(spec.network, StellarNetwork::Mainnet);
    let is_validator = matches!(spec.node_type, NodeType::Validator);
    let is_horizon = matches!(spec.node_type, NodeType::Horizon);
    let has_vpa = spec.vpa_config.is_some();
    let has_hpa = spec.autoscaling.is_some();

    let mut violations = Vec::new();

    // G01 — VPA + HPA conflict
    // Both controllers compete to resize the same resource targets, causing
    // oscillation and unpredictable latency.
    if has_vpa && has_hpa {
        violations.push(GuardrailViolation::new(
            "G01",
            Severity::Error,
            "spec.vpaConfig and spec.autoscaling (HPA) are both enabled; \
             the two controllers will fight over resource targets",
            "Remove one controller: keep VPA for nodes with variable memory \
             pressure, HPA for nodes that need horizontal scale-out.",
        ));
    }

    // G02 — HPA on a Validator node
    // Validator nodes participate in Stellar consensus with a fixed quorum set.
    // Adding or removing replicas dynamically breaks quorum-set membership
    // and can halt consensus.
    if has_hpa && is_validator {
        violations.push(GuardrailViolation::new(
            "G02",
            Severity::Error,
            "spec.autoscaling (HPA) is enabled on a Validator node; \
             dynamic replica scaling breaks quorum-set membership and can halt consensus",
            "Remove spec.autoscaling from Validator nodes. \
             Scale Validators manually and update your quorum set accordingly.",
        ));
    }

    // G03 — Maintenance mode on Mainnet validator
    if spec.maintenance_mode && is_mainnet && is_validator {
        violations.push(GuardrailViolation::new(
            "G03",
            Severity::Warning,
            "spec.maintenanceMode=true on a Mainnet validator reduces quorum \
             redundancy while the node is offline",
            "Coordinate with your quorum-set peers before enabling maintenance \
             mode on a Mainnet validator to avoid losing consensus.",
        ));
    }

    // G04 — Suspended on Mainnet
    if spec.suspended && is_mainnet {
        violations.push(GuardrailViolation::new(
            "G04",
            Severity::Warning,
            "spec.suspended=true halts the node on Mainnet; \
             if this is a validator it will drop out of consensus",
            "Only suspend Mainnet nodes during planned maintenance windows \
             coordinated with your SDF Trust and Safety contact.",
        ));
    }

    // G05 — Validator replicas < 3 on Mainnet
    if is_mainnet && is_validator && spec.replicas < 3 {
        violations.push(GuardrailViolation::new(
            "G05",
            Severity::Error,
            format!(
                "spec.replicas={} for a Mainnet validator; \
                 fewer than 3 replicas leaves no room for a rolling upgrade \
                 without dropping below quorum",
                spec.replicas
            ),
            "Set spec.replicas >= 3 for Mainnet validator nodes.",
        ));
    }

    // G06 — No DR config on Mainnet validator
    if is_mainnet && is_validator && spec.dr_config.is_none() {
        violations.push(GuardrailViolation::new(
            "G06",
            Severity::Warning,
            "spec.drConfig is absent on a Mainnet validator; \
             there is no automated disaster-recovery posture for this node",
            "Configure spec.drConfig with at minimum a snapshot schedule \
             and a cross-region restore target.",
        ));
    }

    // G07 — No network policy on Mainnet
    if is_mainnet && spec.network_policy.is_none() {
        violations.push(GuardrailViolation::new(
            "G07",
            Severity::Warning,
            "spec.networkPolicy is absent on a Mainnet node; \
             all pod-to-pod traffic is unrestricted",
            "Define spec.networkPolicy to allow only the ports and peers \
             required by the node type (2625/tcp for Stellar core, \
             8000/tcp for Horizon).",
        ));
    }

    // G08 — Custom network with no passphrase source
    if matches!(&spec.network, StellarNetwork::Custom(_)) {
        let has_inline = spec
            .custom_network_passphrase
            .as_deref()
            .is_some_and(|p| !p.is_empty());
        let has_secret_ref = spec.passphrase_secret_ref.is_some();
        if !has_inline && !has_secret_ref {
            violations.push(GuardrailViolation::new(
                "G08",
                Severity::Error,
                "spec.network is Custom but neither spec.customNetworkPassphrase \
                 nor spec.passphraseSecretRef is set; the node cannot connect",
                "Set spec.customNetworkPassphrase or spec.passphraseSecretRef \
                 when using a Custom network.",
            ));
        }
    }

    // G09 — restore_from_snapshot set while node is running on Mainnet
    if is_mainnet && spec.restore_from_snapshot.is_some() && !spec.suspended {
        violations.push(GuardrailViolation::new(
            "G09",
            Severity::Warning,
            "spec.restoreFromSnapshot is set but spec.suspended=false; \
             restoring while the pod is live risks data corruption",
            "Set spec.suspended=true before enabling spec.restoreFromSnapshot \
             to ensure the node is stopped before the restore begins.",
        ));
    }

    // G10 — Horizon on Mainnet with no external or managed database
    if is_mainnet && is_horizon {
        let has_db = spec.database.is_some() || spec.managed_database.is_some();
        if !has_db {
            violations.push(GuardrailViolation::new(
                "G10",
                Severity::Warning,
                "Mainnet Horizon node has neither spec.database nor \
                 spec.managedDatabase set; the node will use its embedded \
                 database which is unsuitable for production traffic",
                "Configure spec.database or spec.managedDatabase with a \
                 production-grade PostgreSQL instance for Mainnet Horizon nodes.",
            ));
        }
    }

    violations
}

/// Returns only violations with severity [`Severity::Error`].
pub fn blocking_violations(violations: &[GuardrailViolation]) -> Vec<&GuardrailViolation> {
    violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        AutoscalingConfig, NodeType, StellarNetwork, StellarNode, StellarNodeSpec, VpaConfig,
        VpaUpdateMode,
    };

    fn base_spec(node_type: NodeType, network: StellarNetwork) -> StellarNodeSpec {
        StellarNodeSpec {
            node_type,
            network,
            version: "v21.0.0".into(),
            replicas: 3,
            ..Default::default()
        }
    }

    fn make_node(spec: StellarNodeSpec) -> StellarNode {
        StellarNode::new("test-node", spec)
    }

    fn vpa_config() -> VpaConfig {
        VpaConfig {
            update_mode: VpaUpdateMode::default(),
            container_policies: vec![],
        }
    }

    // ── G01 ──────────────────────────────────────────────────────────────

    #[test]
    fn g01_vpa_plus_hpa_is_error() {
        let mut spec = base_spec(NodeType::Horizon, StellarNetwork::Testnet);
        spec.vpa_config = Some(vpa_config());
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 1,
            max_replicas: 5,
            ..Default::default()
        });
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G01" && e.severity == Severity::Error),
            "expected G01 error, got: {v:?}"
        );
    }

    #[test]
    fn g01_vpa_only_is_clean() {
        let mut spec = base_spec(NodeType::Horizon, StellarNetwork::Testnet);
        spec.vpa_config = Some(vpa_config());
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G01"));
    }

    // ── G02 ──────────────────────────────────────────────────────────────

    #[test]
    fn g02_hpa_on_validator_is_error() {
        let mut spec = base_spec(NodeType::Validator, StellarNetwork::Testnet);
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 1,
            max_replicas: 5,
            ..Default::default()
        });
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G02" && e.severity == Severity::Error),
            "expected G02 error, got: {v:?}"
        );
    }

    #[test]
    fn g02_hpa_on_horizon_is_clean() {
        let mut spec = base_spec(NodeType::Horizon, StellarNetwork::Testnet);
        spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 1,
            max_replicas: 5,
            ..Default::default()
        });
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G02"));
    }

    // ── G03 ──────────────────────────────────────────────────────────────

    #[test]
    fn g03_maintenance_mainnet_validator_is_warning() {
        let mut spec = base_spec(NodeType::Validator, StellarNetwork::Mainnet);
        spec.maintenance_mode = true;
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G03" && e.severity == Severity::Warning),
            "expected G03 warning, got: {v:?}"
        );
    }

    #[test]
    fn g03_maintenance_testnet_validator_is_clean() {
        let mut spec = base_spec(NodeType::Validator, StellarNetwork::Testnet);
        spec.maintenance_mode = true;
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G03"));
    }

    // ── G04 ──────────────────────────────────────────────────────────────

    #[test]
    fn g04_suspended_mainnet_is_warning() {
        let mut spec = base_spec(NodeType::Horizon, StellarNetwork::Mainnet);
        spec.suspended = true;
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G04" && e.severity == Severity::Warning),
            "expected G04 warning, got: {v:?}"
        );
    }

    #[test]
    fn g04_suspended_testnet_is_clean() {
        let mut spec = base_spec(NodeType::Validator, StellarNetwork::Testnet);
        spec.suspended = true;
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G04"));
    }

    // ── G05 ──────────────────────────────────────────────────────────────

    #[test]
    fn g05_validator_mainnet_low_replicas_is_error() {
        let mut spec = base_spec(NodeType::Validator, StellarNetwork::Mainnet);
        spec.replicas = 1;
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G05" && e.severity == Severity::Error),
            "expected G05 error, got: {v:?}"
        );
    }

    #[test]
    fn g05_validator_mainnet_three_replicas_is_clean() {
        let spec = base_spec(NodeType::Validator, StellarNetwork::Mainnet);
        assert_eq!(spec.replicas, 3);
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G05"));
    }

    #[test]
    fn g05_horizon_mainnet_single_replica_is_clean() {
        let mut spec = base_spec(NodeType::Horizon, StellarNetwork::Mainnet);
        spec.replicas = 1;
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G05"));
    }

    // ── G06 ──────────────────────────────────────────────────────────────

    #[test]
    fn g06_no_dr_mainnet_validator_is_warning() {
        let spec = base_spec(NodeType::Validator, StellarNetwork::Mainnet);
        assert!(spec.dr_config.is_none());
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G06" && e.severity == Severity::Warning),
            "expected G06 warning, got: {v:?}"
        );
    }

    #[test]
    fn g06_horizon_mainnet_no_dr_is_clean() {
        let spec = base_spec(NodeType::Horizon, StellarNetwork::Mainnet);
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G06"));
    }

    // ── G08 ──────────────────────────────────────────────────────────────

    #[test]
    fn g08_custom_network_no_passphrase_is_error() {
        let spec = base_spec(
            NodeType::Validator,
            StellarNetwork::Custom("my-net".into()),
        );
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(
            v.iter().any(|e| e.code == "G08" && e.severity == Severity::Error),
            "expected G08 error, got: {v:?}"
        );
    }

    #[test]
    fn g08_custom_network_with_inline_passphrase_is_clean() {
        let mut spec = base_spec(
            NodeType::Validator,
            StellarNetwork::Custom("my-net".into()),
        );
        spec.custom_network_passphrase = Some("My Network Passphrase ; 2024".into());
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G08"));
    }

    #[test]
    fn g08_custom_network_with_secret_ref_is_clean() {
        let mut spec = base_spec(
            NodeType::Validator,
            StellarNetwork::Custom("my-net".into()),
        );
        spec.passphrase_secret_ref = Some("passphrase-secret".into());
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(!v.iter().any(|e| e.code == "G08"));
    }

    // ── blocking_violations ───────────────────────────────────────────────

    #[test]
    fn blocking_violations_filters_errors_only() {
        let mut spec = base_spec(NodeType::Validator, StellarNetwork::Mainnet);
        spec.replicas = 1; // G05 Error
        // G06 Warning (no DR) and G07 Warning (no network policy) are also present
        let node = make_node(spec);
        let all = check_config_guardrails(&node);
        let blocking = blocking_violations(&all);
        assert!(blocking.iter().all(|v| v.severity == Severity::Error));
        assert!(blocking.iter().any(|v| v.code == "G05"));
        // Warnings must not appear in blocking list
        assert!(!blocking.iter().any(|v| v.code == "G06"));
    }

    #[test]
    fn clean_testnet_config_has_no_violations() {
        let spec = base_spec(NodeType::Validator, StellarNetwork::Testnet);
        let node = make_node(spec);
        let v = check_config_guardrails(&node);
        assert!(v.is_empty(), "clean Testnet config should pass all guardrails, got: {v:?}");
    }
}
