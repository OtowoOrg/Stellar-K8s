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
//! SDF Testnet Compliance Validator
//!
//! Validates that a testnet validator configuration matches SDF testnet
//! requirements (peers, quorum, history archives) and emits warnings on
//! non-compliance. This validation is purely local — no network calls are
//! made, making it cheap to run on every reconcile.
//!
//! # Design
//!
//! The validator compares the node's `ValidatorConfig` against the known SDF
//! testnet defaults:
//!
//! | Field | SDF Default |
//! |-------|-------------|
//! | History archive URL | `https://history.stellar.org/prd/core-testnet/core_testnet_001` |
//! | SDF testnet validators | GDKDZGJQJ2WQJ2WQJ2WQJ2WQJ2WQJ2WQJ2WQJ2, GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP, GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IX |
//!
//! When a node is configured for the `testnet` network and has a
//! `ValidatorConfig`, the reconciler calls this validator and emits a
//! `ComplianceWarning` condition listing any non-compliant fields.

use crate::crd::{StellarNetwork, StellarNode, ValidatorConfig};

/// The SDF testnet history archive URL.
pub const SDF_TESTNET_HISTORY_ARCHIVE: &str =
    "https://history.stellar.org/prd/core-testnet/core_testnet_001";

/// SDF testnet validator public keys that should be present in the quorum set.
///
/// These are the well-known SDF-operated testnet validators.
pub const SDF_TESTNET_VALIDATORS: &[&str] = &[
    "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA",
    "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP",
    "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z",
];

/// Known SDF testnet peers (public keys of nodes that should be in
/// `known_peers` for a compliant testnet configuration).
pub const SDF_TESTNET_PEERS: &[&str] = &[
    "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA",
    "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP",
    "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z",
    "GCQPG2M2Q6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6H6",
    "GAAZI4T3S6XNVW5RQFYNLJNHVBRFXRWUN5Q3NXPK6V4CNHRN7M7PX2YP",
];

/// A single compliance finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComplianceFinding {
    /// The field that is non-compliant.
    pub field: String,
    /// Human-readable description of the issue.
    pub message: String,
    /// The expected value (or "must include" for set-based checks).
    pub expected: String,
    /// The actual value found in the config.
    pub actual: String,
}

/// Result of a testnet compliance check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComplianceReport {
    /// Whether the configuration is fully compliant.
    pub compliant: bool,
    /// Individual findings for each non-compliant field.
    pub findings: Vec<ComplianceFinding>,
}

/// Validate a testnet validator configuration against SDF defaults.
///
/// Returns a [`ComplianceReport`] with findings for any non-compliant fields.
/// This function is pure and makes no network calls.
///
/// Only nodes on the `testnet` network with a `ValidatorConfig` are checked;
/// all other configurations pass silently.
pub fn validate_testnet_compliance(node: &StellarNode) -> ComplianceReport {
    // Only validate testnet nodes with a ValidatorConfig
    if node.spec.network != StellarNetwork::Testnet {
        return ComplianceReport::default();
    }

    let validator_config = match &node.spec.validator_config {
        Some(vc) => vc,
        None => return ComplianceReport::default(),
    };

    let mut findings = Vec::new();

    // Check history archive URLs
    check_history_archives(validator_config, &mut findings);

    // Check known peers
    check_known_peers(validator_config, &mut findings);

    // Check quorum set includes SDF testnet validators
    check_quorum_set(validator_config, &mut findings);

    ComplianceReport {
        compliant: findings.is_empty(),
        findings,
    }
}

fn check_history_archives(config: &ValidatorConfig, findings: &mut Vec<ComplianceFinding>) {
    let expected_archive = SDF_TESTNET_HISTORY_ARCHIVE;

    if config.history_archive_urls.is_empty() {
        findings.push(ComplianceFinding {
            field: "historyArchiveUrls".to_string(),
            message: "No history archive URLs configured; SDF testnet requires a history archive"
                .to_string(),
            expected: expected_archive.to_string(),
            actual: "(none)".to_string(),
        });
        return;
    }

    // Check if the SDF default archive is present
    let has_sdf_archive = config
        .history_archive_urls
        .iter()
        .any(|url| url == expected_archive);

    if !has_sdf_archive {
        findings.push(ComplianceFinding {
            field: "historyArchiveUrls".to_string(),
            message: format!(
                "History archive URL differs from SDF testnet default; expected {}",
                expected_archive
            ),
            expected: expected_archive.to_string(),
            actual: config.history_archive_urls.join(", "),
        });
    }
}

fn check_known_peers(config: &ValidatorConfig, findings: &mut Vec<ComplianceFinding>) {
    // If known_peers is not set, we cannot validate — skip with a finding
    // since a compliant testnet config should include SDF peers.
    let known_peers = match &config.known_peers {
        Some(peers) => peers,
        None => {
            findings.push(ComplianceFinding {
                field: "knownPeers".to_string(),
                message: "knownPeers is not set; SDF testnet requires known peers for connectivity"
                    .to_string(),
                expected: SDF_TESTNET_PEERS.join(", "),
                actual: "(not set)".to_string(),
            });
            return;
        }
    };

    // Check that at least some SDF testnet peers are present
    let missing_peers: Vec<&str> = SDF_TESTNET_PEERS
        .iter()
        .filter(|peer| {
            let needle: &str = peer;
            !known_peers
                .split(',')
                .flat_map(|segment| segment.split(' '))
                .map(str::trim)
                .any(|k| k == needle)
        })
        .copied()
        .collect();

    if !missing_peers.is_empty() {
        findings.push(ComplianceFinding {
            field: "knownPeers".to_string(),
            message: format!(
                "knownPeers is missing SDF testnet validators: {}",
                missing_peers.join(", ")
            ),
            expected: SDF_TESTNET_PEERS.join(", "),
            actual: known_peers.clone(),
        });
    }
}

fn check_quorum_set(config: &ValidatorConfig, findings: &mut Vec<ComplianceFinding>) {
    let quorum_set = match &config.quorum_set {
        Some(qs) => qs,
        None => {
            findings.push(ComplianceFinding {
                field: "quorumSet".to_string(),
                message: "quorumSet is not set; SDF testnet requires a quorum set that includes SDF validators".to_string(),
                expected: SDF_TESTNET_VALIDATORS.join(", "),
                actual: "(not set)".to_string(),
            });
            return;
        }
    };

    // Check that at least one SDF testnet validator is in the quorum set
    let has_sdf_validator = SDF_TESTNET_VALIDATORS
        .iter()
        .any(|validator| quorum_set.contains(validator));

    if !has_sdf_validator {
        findings.push(ComplianceFinding {
            field: "quorumSet".to_string(),
            message: "quorumSet does not include any SDF testnet validators".to_string(),
            expected: SDF_TESTNET_VALIDATORS.join(", "),
            actual: quorum_set.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{StellarNetwork, StellarNode, StellarNodeSpec, ValidatorConfig};

    fn make_testnet_node(validator_config: Option<ValidatorConfig>) -> StellarNode {
        StellarNode {
            spec: StellarNodeSpec {
                network: StellarNetwork::Testnet,
                validator_config,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn compliant_testnet_config_passes_silently() {
        let config = ValidatorConfig {
            history_archive_urls: vec![SDF_TESTNET_HISTORY_ARCHIVE.to_string()],
            known_peers: Some(SDF_TESTNET_PEERS.join(",")),
            quorum_set: Some(SDF_TESTNET_VALIDATORS[0].to_string()),
            ..Default::default()
        };
        let node = make_testnet_node(Some(config));
        let report = validate_testnet_compliance(&node);
        assert!(report.compliant, "Compliant config should pass: {report:?}");
        assert!(report.findings.is_empty());
    }

    #[test]
    fn non_compliant_testnet_config_produces_findings() {
        let config = ValidatorConfig {
            history_archive_urls: vec!["https://custom.archive.org".to_string()],
            known_peers: Some("GXYZ123".to_string()),
            quorum_set: Some("GXYZ123".to_string()),
            ..Default::default()
        };
        let node = make_testnet_node(Some(config));
        let report = validate_testnet_compliance(&node);
        assert!(!report.compliant, "Non-compliant config should fail");
        assert!(!report.findings.is_empty());
        // Should have findings for all three fields
        let fields: Vec<&str> = report.findings.iter().map(|f| f.field.as_str()).collect();
        assert!(fields.contains(&"historyArchiveUrls"));
        assert!(fields.contains(&"knownPeers"));
        assert!(fields.contains(&"quorumSet"));
    }

    #[test]
    fn mainnet_node_passes_silently() {
        let config = ValidatorConfig {
            history_archive_urls: vec!["https://custom.archive.org".to_string()],
            known_peers: Some("GXYZ123".to_string()),
            quorum_set: Some("GXYZ123".to_string()),
            ..Default::default()
        };
        let mut node = make_testnet_node(Some(config));
        node.spec.network = StellarNetwork::Mainnet;
        let report = validate_testnet_compliance(&node);
        assert!(report.compliant, "Mainnet should pass silently");
        assert!(report.findings.is_empty());
    }

    #[test]
    fn node_without_validator_config_passes_silently() {
        let node = make_testnet_node(None);
        let report = validate_testnet_compliance(&node);
        assert!(
            report.compliant,
            "Node without validator_config should pass silently"
        );
        assert!(report.findings.is_empty());
    }

    #[test]
    fn missing_history_archive_is_detected() {
        let config = ValidatorConfig {
            history_archive_urls: vec![],
            known_peers: Some(SDF_TESTNET_PEERS.join(",")),
            quorum_set: Some(SDF_TESTNET_VALIDATORS[0].to_string()),
            ..Default::default()
        };
        let node = make_testnet_node(Some(config));
        let report = validate_testnet_compliance(&node);
        let history_finding = report
            .findings
            .iter()
            .find(|f| f.field == "historyArchiveUrls");
        assert!(
            history_finding.is_some(),
            "Missing history archive should be detected"
        );
    }

    #[test]
    fn missing_known_peers_is_detected() {
        let config = ValidatorConfig {
            history_archive_urls: vec![SDF_TESTNET_HISTORY_ARCHIVE.to_string()],
            known_peers: None,
            quorum_set: Some(SDF_TESTNET_VALIDATORS[0].to_string()),
            ..Default::default()
        };
        let node = make_testnet_node(Some(config));
        let report = validate_testnet_compliance(&node);
        let peers_finding = report.findings.iter().find(|f| f.field == "knownPeers");
        assert!(
            peers_finding.is_some(),
            "Missing known peers should be detected"
        );
    }

    #[test]
    fn missing_quorum_set_is_detected() {
        let config = ValidatorConfig {
            history_archive_urls: vec![SDF_TESTNET_HISTORY_ARCHIVE.to_string()],
            known_peers: Some(SDF_TESTNET_PEERS.join(",")),
            quorum_set: None,
            ..Default::default()
        };
        let node = make_testnet_node(Some(config));
        let report = validate_testnet_compliance(&node);
        let quorum_finding = report.findings.iter().find(|f| f.field == "quorumSet");
        assert!(
            quorum_finding.is_some(),
            "Missing quorum set should be detected"
        );
    }

    #[test]
    fn report_lists_specific_non_compliant_fields() {
        let config = ValidatorConfig {
            history_archive_urls: vec!["https://wrong.archive.org".to_string()],
            known_peers: Some("GWRONG1,GWRONG2".to_string()),
            quorum_set: Some("GWRONG3".to_string()),
            ..Default::default()
        };
        let node = make_testnet_node(Some(config));
        let report = validate_testnet_compliance(&node);
        assert!(!report.compliant);
        // Each finding should list both expected and actual values
        for finding in &report.findings {
            assert!(
                !finding.expected.is_empty(),
                "Finding should list expected value for {}",
                finding.field
            );
            assert!(
                !finding.actual.is_empty(),
                "Finding should list actual value for {}",
                finding.field
            );
        }
    }
}
