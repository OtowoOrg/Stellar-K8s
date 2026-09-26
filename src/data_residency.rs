// Copyright 2026 Stellar-K8s Contributors
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
//! Data residency enforcement for cross-region stateful workloads (#1495).
//!
//! Residency rules are expressed per dataset and per tenant. The scheduler
//! and volume placement honor them through node-affinity-shaped constraints
//! (no scheduler fork), violation attempts are blocked with a clear denial
//! reason plus a signed audit event, and a coverage report plus evidence
//! export serve compliance review.
//!
//! This module complements the existing jurisdiction node-affinity injector
//! (`crate::controller::jurisdiction`) with the missing pieces: per-tenant
//! rules, volume topology, denial, signed audit, coverage, and export.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer, SigningKey};
use k8s_openapi::api::core::v1::{NodeAffinity, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

/// Default label key used to match regions.
pub const DEFAULT_REGION_LABEL_KEY: &str = "topology.kubernetes.io/region";

/// Residency rule for one dataset/tenant pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidencyRule {
    /// Dataset identifier (e.g. `ledger`, `history-archive`).
    pub dataset: String,
    /// Tenant identifier (e.g. team or customer ID).
    pub tenant: String,
    /// Permitted regions. Empty means "deny everywhere" is a misconfig —
    /// use [`ResidencyPolicy::validate`] to catch it.
    pub allowed_regions: Vec<String>,
    /// Node label key carrying the region.
    pub label_key: String,
}

impl ResidencyRule {
    pub fn new(dataset: &str, tenant: &str, allowed_regions: Vec<String>) -> Self {
        Self {
            dataset: dataset.to_string(),
            tenant: tenant.to_string(),
            allowed_regions,
            label_key: DEFAULT_REGION_LABEL_KEY.to_string(),
        }
    }

    /// Node-affinity-shaped enforcement consumed by scheduling machinery.
    /// Returns `None` when the rule permits nothing (misconfigured) — the
    /// caller must deny rather than schedule unconstrained.
    pub fn node_affinity(&self) -> Option<NodeAffinity> {
        if self.allowed_regions.is_empty() {
            return None;
        }
        Some(NodeAffinity {
            required_during_scheduling_ignored_during_execution: Some(NodeSelector {
                node_selector_terms: vec![NodeSelectorTerm {
                    match_expressions: Some(vec![NodeSelectorRequirement {
                        key: self.label_key.clone(),
                        operator: "In".to_string(),
                        values: Some(self.allowed_regions.clone()),
                    }]),
                    ..Default::default()
                }],
            }),
            ..Default::default()
        })
    }

    /// Allowed topologies for PVC `allowedTopologies` / volume placement.
    pub fn pvc_allowed_topologies(&self) -> Vec<HashMap<String, Vec<String>>> {
        if self.allowed_regions.is_empty() {
            return Vec::new();
        }
        vec![HashMap::from([(
            self.label_key.clone(),
            self.allowed_regions.clone(),
        )])]
    }

    /// Check whether a node in `region` satisfies this rule.
    pub fn allows_region(&self, region: &str) -> bool {
        self.allowed_regions.iter().any(|r| r == region)
    }
}

/// In-memory residency policy: per-dataset and per-tenant rules.
#[derive(Debug, Default)]
pub struct ResidencyPolicy {
    /// Key: `(dataset, tenant)`.
    rules: HashMap<(String, String), ResidencyRule>,
}

impl ResidencyPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, rule: ResidencyRule) {
        self.rules.insert((rule.dataset.clone(), rule.tenant.clone()), rule);
    }

    pub fn rule_for(&self, dataset: &str, tenant: &str) -> Option<&ResidencyRule> {
        self.rules.get(&(dataset.to_string(), tenant.to_string()))
    }

    /// Validate all rules; returns human-readable problems (empty = valid).
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for rule in self.rules.values() {
            if rule.dataset.is_empty() || rule.tenant.is_empty() {
                problems.push(format!("rule has empty dataset/tenant: {rule:?}"));
            }
            if rule.allowed_regions.is_empty() {
                problems.push(format!(
                    "rule {}/{} permits no regions; scheduling would deny everywhere",
                    rule.dataset, rule.tenant
                ));
            }
        }
        problems
    }

    /// Scheduler predicate: is a node in `region` feasible for this workload?
    /// Unknown (dataset, tenant) pairs are denied closed (fail-safe).
    pub fn node_feasible(&self, dataset: &str, tenant: &str, region: &str) -> bool {
        self.rule_for(dataset, tenant).map(|r| r.allows_region(region)).unwrap_or(false)
    }

    /// Block a violating placement with a clear, auditable denial reason.
    /// Returns `Ok(())` when allowed, `Err(reason)` when blocked.
    pub fn admit(&self, dataset: &str, tenant: &str, region: &str, workload: &str) -> Result<(), String> {
        match self.rule_for(dataset, tenant) {
            None => Err(format!(
                "residency denied: workload {workload} dataset {dataset} tenant {tenant} has no residency rule; default-deny (fail-safe)"
            )),
            Some(rule) if rule.allows_region(region) => Ok(()),
            Some(rule) => {
                let reason = format!(
                    "residency denied: workload {workload} dataset {dataset} tenant {tenant} may only run in [{}] (label {}), requested region {region}",
                    rule.allowed_regions.join(", "),
                    rule.label_key,
                );
                warn!(reason = %reason, "data-residency violation blocked");
                Err(reason)
            }
        }
    }

    /// Coverage report: every stateful workload (dataset/tenant) and whether
    /// it is covered by a rule.
    pub fn coverage_report(&self, workloads: &[(String, String, String)]) -> CoverageReport {
        let entries = workloads
            .iter()
            .map(|(workload, dataset, tenant)| CoverageEntry {
                workload: workload.clone(),
                dataset: dataset.clone(),
                tenant: tenant.clone(),
                covered: self.rule_for(dataset, tenant).is_some(),
                allowed_regions: self
                    .rule_for(dataset, tenant)
                    .map(|r| r.allowed_regions.clone())
                    .unwrap_or_default(),
            })
            .collect();
        CoverageReport { generated_at: Utc::now(), entries }
    }
}

/// One row of the policy coverage report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageEntry {
    pub workload: String,
    pub dataset: String,
    pub tenant: String,
    pub covered: bool,
    pub allowed_regions: Vec<String>,
}

/// Coverage report listing every stateful workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<CoverageEntry>,
}

impl CoverageReport {
    pub fn uncovered(&self) -> Vec<&CoverageEntry> {
        self.entries.iter().filter(|e| !e.covered).collect()
    }
}

/// Signed audit event for a blocked violation attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidencyViolationEvent {
    pub at: DateTime<Utc>,
    pub workload: String,
    pub dataset: String,
    pub tenant: String,
    pub requested_region: String,
    pub allowed_regions: Vec<String>,
    pub reason: String,
}

/// Signed envelope (same shape as the compliance exporter: sha256 + ed25519).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedResidencyEvent {
    pub payload: ResidencyViolationEvent,
    pub sha256: String,
    pub signature: String,
    pub public_key: String,
}

/// Build a signed audit event for a violation attempt.
pub fn sign_violation(
    workload: &str,
    dataset: &str,
    tenant: &str,
    requested_region: &str,
    allowed_regions: &[String],
    reason: &str,
) -> SignedResidencyEvent {
    let payload = ResidencyViolationEvent {
        at: Utc::now(),
        workload: workload.to_string(),
        dataset: dataset.to_string(),
        tenant: tenant.to_string(),
        requested_region: requested_region.to_string(),
        allowed_regions: allowed_regions.to_vec(),
        reason: reason.to_string(),
    };
    let payload_bytes = serde_json::to_vec(&payload).expect("violation payload serialises");
    let digest = Sha256::digest(&payload_bytes);
    let signing_key = SigningKey::generate(&mut OsRng);
    let sig = signing_key.sign(digest.as_slice());
    SignedResidencyEvent {
        payload,
        sha256: hex::encode(digest.as_slice()),
        signature: hex::encode(sig.to_bytes()),
        public_key: hex::encode(signing_key.verifying_key().to_bytes()),
    }
}

/// Evidence export for compliance review: coverage + signed violations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidencyEvidence {
    pub exported_at: DateTime<Utc>,
    pub operator_version: String,
    pub coverage: CoverageReport,
    pub violations: Vec<SignedResidencyEvent>,
    pub zero_violations_30d: bool,
}

pub fn export_evidence(coverage: CoverageReport, violations: Vec<SignedResidencyEvent>) -> ResidencyEvidence {
    // Zero-violation claim holds when no violations are recorded in the
    // export window; callers pass the trailing-30d violation list.
    let zero = violations.is_empty();
    ResidencyEvidence {
        exported_at: Utc::now(),
        operator_version: env!("CARGO_PKG_VERSION").to_string(),
        coverage,
        violations,
        zero_violations_30d: zero,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ResidencyPolicy {
        let mut p = ResidencyPolicy::new();
        p.insert(ResidencyRule::new("ledger", "tenant-a", vec!["eu-west-1".to_string(), "eu-central-1".to_string()]));
        p
    }

    #[test]
    fn allowed_region_passes_forbidden_blocked_with_reason() {
        let p = policy();
        assert!(p.admit("ledger", "tenant-a", "eu-west-1", "stellar-node-0").is_ok());
        let err = p.admit("ledger", "tenant-a", "us-east-1", "stellar-node-0").unwrap_err();
        assert!(err.contains("residency denied"));
        assert!(err.contains("eu-west-1"));
    }

    #[test]
    fn unknown_workload_default_denies() {
        let p = policy();
        assert!(p.admit("unknown", "tenant-a", "eu-west-1", "w").is_err());
    }

    #[test]
    fn node_affinity_and_pvc_topology_honor_rule() {
        let rule = ResidencyRule::new("ledger", "t", vec!["eu-west-1".to_string()]);
        let affinity = rule.node_affinity().unwrap();
        let expr = &affinity
            .required_during_scheduling_ignored_during_execution
            .unwrap()
            .node_selector_terms[0]
            .match_expressions
            .as_ref()
            .unwrap()[0];
        assert_eq!(expr.key, DEFAULT_REGION_LABEL_KEY);
        assert_eq!(expr.values.as_ref().unwrap(), &vec!["eu-west-1".to_string()]);
        let topo = rule.pvc_allowed_topologies();
        assert_eq!(topo.len(), 1);
        assert_eq!(topo[0][DEFAULT_REGION_LABEL_KEY], vec!["eu-west-1".to_string()]);
    }

    #[test]
    fn signed_violation_event_verifies_shape() {
        let signed = sign_violation("w", "ledger", "tenant-a", "us-east-1", &["eu-west-1".to_string()], "denied");
        assert_eq!(signed.payload.requested_region, "us-east-1");
        assert_eq!(signed.sha256.len(), 64);
        assert!(!signed.signature.is_empty());
        assert!(!signed.public_key.is_empty());
    }

    #[test]
    fn coverage_lists_every_workload_and_flags_uncovered() {
        let p = policy();
        let report = p.coverage_report(&[
            ("node-0".to_string(), "ledger".to_string(), "tenant-a".to_string()),
            ("node-1".to_string(), "ledger".to_string(), "tenant-b".to_string()),
        ]);
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.uncovered().len(), 1);
        let evidence = export_evidence(report, Vec::new());
        assert!(evidence.zero_violations_30d);
    }
}
