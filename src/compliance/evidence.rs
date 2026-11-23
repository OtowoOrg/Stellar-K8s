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
//! Evidence collection subsystem for compliance audits.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A declarative control probe (CEL/Kyverno-style configuration).
///
/// Adding coverage for a new control is a matter of configuration, not code:
/// the probe names the framework control it satisfies, the expression to
/// evaluate, and the data sources the expression draws from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlProbe {
    /// Framework control identifier, e.g. `SOC2-CC6.1`.
    pub control_id: String,
    /// Human readable title of the control.
    pub title: String,
    /// CEL/Kyverno-style expression evaluated by the probe runner.
    pub expression: String,
    /// Data sources referenced by the expression (e.g. `policy-engine`, `metrics-store`).
    #[serde(default)]
    pub data_sources: Vec<String>,
    /// Maximum age of evidence for it to be considered in-window.
    #[serde(default = "default_max_age_secs")]
    pub max_age_secs: i64,
}

fn default_max_age_secs() -> i64 {
    24 * 60 * 60
}

/// Outcome of evaluating a single probe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOutcome {
    pub control_id: String,
    pub passed: bool,
    pub detail: String,
}

/// A finding raised when evidence collection fails for a control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionFinding {
    pub control_id: String,
    pub severity: String,
    pub message: String,
    pub raised_at: DateTime<Utc>,
}

/// Snapshot of the policy engine and metrics store used to evaluate probes.
#[derive(Debug, Clone, Default)]
pub struct ProbeContext {
    /// Policy engine facts, keyed by fact name.
    pub policy_facts: BTreeMap<String, String>,
    /// Metrics store samples, keyed by metric name.
    pub metrics: BTreeMap<String, f64>,
}

impl ProbeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fact(mut self, key: &str, value: &str) -> Self {
        self.policy_facts.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_metric(mut self, key: &str, value: f64) -> Self {
        self.metrics.insert(key.to_string(), value);
        self
    }
}

/// Evaluates a probe expression against a [`ProbeContext`].
///
/// The supported expression grammar is intentionally small and declarative:
/// `fact("name") == "value"`, `metric("name") >= 0.5`, combined with `&&`.
/// This mirrors the CEL/Kyverno style while remaining dependency free.
pub fn evaluate_probe(probe: &ControlProbe, ctx: &ProbeContext) -> ProbeOutcome {
    let mut passed = true;
    let mut details = Vec::new();
    for clause in probe.expression.split("&&").map(str::trim) {
        if clause.is_empty() {
            continue;
        }
        match evaluate_clause(clause, ctx) {
            Ok(ok) => {
                details.push(format!("{} => {}", clause, ok));
                passed &= ok;
            }
            Err(err) => {
                details.push(format!("{} => error: {}", clause, err));
                passed = false;
            }
        }
    }
    ProbeOutcome {
        control_id: probe.control_id.clone(),
        passed,
        detail: details.join("; "),
    }
}

fn evaluate_clause(clause: &str, ctx: &ProbeContext) -> Result<bool, String> {
    for op in ["==", "!=", ">=", "<=", ">", "<"] {
        if let Some(idx) = clause.find(op) {
            let lhs = clause[..idx].trim();
            let rhs = clause[idx + op.len()..].trim();
            return compare(lhs, op, rhs, ctx);
        }
    }
    Err(format!("unsupported clause: {}", clause))
}

fn compare(lhs: &str, op: &str, rhs: &str, ctx: &ProbeContext) -> Result<bool, String> {
    if let Some(name) = extract_call(lhs, "fact") {
        let actual = ctx
            .policy_facts
            .get(&name)
            .ok_or_else(|| format!("missing fact {}", name))?;
        let expected = rhs.trim_matches('"');
        return Ok(match op {
            "==" => actual == expected,
            "!=" => actual != expected,
            _ => return Err(format!("operator {} not valid for facts", op)),
        });
    }
    if let Some(name) = extract_call(lhs, "metric") {
        let actual = *ctx
            .metrics
            .get(&name)
            .ok_or_else(|| format!("missing metric {}", name))?;
        let expected: f64 = rhs
            .parse()
            .map_err(|_| format!("invalid numeric literal {}", rhs))?;
        return Ok(match op {
            "==" => (actual - expected).abs() < f64::EPSILON,
            "!=" => (actual - expected).abs() >= f64::EPSILON,
            ">=" => actual >= expected,
            "<=" => actual <= expected,
            ">" => actual > expected,
            "<" => actual < expected,
            _ => return Err(format!("unsupported operator {}", op)),
        });
    }
    Err(format!("unsupported left-hand side: {}", lhs))
}

fn extract_call(expr: &str, fname: &str) -> Option<String> {
    let prefix = format!("{}(", fname);
    let rest = expr.strip_prefix(&prefix)?;
    let inner = rest.strip_suffix(')')?;
    Some(inner.trim().trim_matches('"').to_string())
}

/// A signed evidence package for a single framework control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePackage {
    pub control_id: String,
    pub generated_at: DateTime<Utc>,
    pub items: Vec<EvidenceItem>,
    /// HMAC-SHA256 signature over the canonical package payload.
    pub signature: String,
}

impl EvidencePackage {
    /// Canonical payload used for signing and verification.
    fn payload(&self) -> String {
        let mut buf = String::new();
        buf.push_str(&self.control_id);
        buf.push('|');
        buf.push_str(&self.generated_at.to_rfc3339());
        for item in &self.items {
            buf.push('|');
            buf.push_str(&item.id);
            buf.push(':');
            buf.push_str(&item.content_hash);
        }
        buf
    }

    /// Verify the package signature offline using the supplied key.
    pub fn verify_offline(&self, key: &[u8]) -> bool {
        let expected = hmac_sha256(key, self.payload().as_bytes());
        constant_time_eq(expected.as_bytes(), self.signature.as_bytes())
            && self.items.iter().all(|item| {
                let hash = hex::encode(Sha256::digest(item.content.as_bytes()));
                hash == item.content_hash
            })
    }
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let d = Sha256::digest(key);
        k[..d.len()].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    hex::encode(outer.finalize())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Tracks evidence coverage across in-scope controls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    pub in_scope: BTreeSet<String>,
    pub covered: BTreeSet<String>,
    pub missing: BTreeSet<String>,
}

impl CoverageReport {
    pub fn coverage_ratio(&self) -> f64 {
        if self.in_scope.is_empty() {
            return 1.0;
        }
        self.covered.len() as f64 / self.in_scope.len() as f64
    }

    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// A piece of compliance evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceItem {
    pub id: String,
    pub rule_id: String,
    pub title: String,
    pub collected_at: DateTime<Utc>,
    pub source: String,
    pub content_hash: String,
    pub content: String,
}

/// Collects and stores compliance evidence artifacts.
pub struct EvidenceCollector {
    items: Vec<EvidenceItem>,
    probes: Vec<ControlProbe>,
    in_scope: BTreeSet<String>,
    findings: Vec<CollectionFinding>,
}

impl EvidenceCollector {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            probes: Vec::new(),
            in_scope: BTreeSet::new(),
            findings: Vec::new(),
        }
    }

    /// Register a declarative control probe.
    pub fn register_probe(&mut self, probe: ControlProbe) {
        self.in_scope.insert(probe.control_id.clone());
        self.probes.push(probe);
    }

    /// Declare a control as in-scope for coverage tracking.
    pub fn add_in_scope(&mut self, control_id: &str) {
        self.in_scope.insert(control_id.to_string());
    }

    pub fn probes(&self) -> &[ControlProbe] {
        &self.probes
    }

    pub fn findings(&self) -> &[CollectionFinding] {
        &self.findings
    }

    /// Run all registered probes against the supplied context.
    ///
    /// Any probe that fails to evaluate (or evaluates to false) raises a
    /// finding within the same collection cycle.
    pub fn run_probes(&mut self, ctx: &ProbeContext) -> Vec<ProbeOutcome> {
        let mut outcomes = Vec::new();
        for probe in &self.probes {
            let outcome = evaluate_probe(probe, ctx);
            if outcome.passed {
                self.collect(
                    &probe.control_id,
                    &probe.title,
                    "probe-runner",
                    &format!("expression={} detail={}", probe.expression, outcome.detail),
                );
            } else {
                self.findings.push(CollectionFinding {
                    control_id: probe.control_id.clone(),
                    severity: "high".to_string(),
                    message: format!(
                        "evidence collection failed for {}: {}",
                        probe.control_id, outcome.detail
                    ),
                    raised_at: Utc::now(),
                });
            }
            outcomes.push(outcome);
        }
        outcomes
    }

    /// Compute coverage of in-scope controls by collected evidence.
    pub fn coverage(&self) -> CoverageReport {
        let covered: BTreeSet<String> = self
            .items
            .iter()
            .map(|item| item.rule_id.clone())
            .filter(|id| self.in_scope.contains(id))
            .collect();
        let missing: BTreeSet<String> = self
            .in_scope
            .difference(&covered)
            .cloned()
            .collect();
        CoverageReport {
            in_scope: self.in_scope.clone(),
            covered,
            missing,
        }
    }

    /// Build a signed evidence package for a single framework control.
    pub fn package_control(&self, control_id: &str, key: &[u8]) -> Option<EvidencePackage> {
        let items: Vec<EvidenceItem> = self
            .items
            .iter()
            .filter(|item| item.rule_id == control_id)
            .cloned()
            .collect();
        if items.is_empty() {
            return None;
        }
        let mut package = EvidencePackage {
            control_id: control_id.to_string(),
            generated_at: Utc::now(),
            items,
            signature: String::new(),
        };
        package.signature = hmac_sha256(key, package.payload().as_bytes());
        Some(package)
    }

    /// Verify that an item is in-window relative to `now` for the given control.
    pub fn is_in_window(&self, item: &EvidenceItem, now: DateTime<Utc>) -> bool {
        let max_age = self
            .probes
            .iter()
            .find(|p| p.control_id == item.rule_id)
            .map(|p| p.max_age_secs)
            .unwrap_or_else(default_max_age_secs);
        now.signed_duration_since(item.collected_at) <= Duration::seconds(max_age)
    }

    pub fn collect(
        &mut self,
        rule_id: &str,
        title: &str,
        source: &str,
        content: &str,
    ) -> EvidenceItem {
        let hash = hex::encode(Sha256::digest(content.as_bytes()));
        let item = EvidenceItem {
            id: format!("ev-{}-{}", rule_id, self.items.len()),
            rule_id: rule_id.to_string(),
            title: title.to_string(),
            collected_at: Utc::now(),
            source: source.to_string(),
            content_hash: hash,
            content: content.to_string(),
        };
        self.items.push(item.clone());
        item
    }

    pub fn items(&self) -> &[EvidenceItem] {
        &self.items
    }

    pub fn verify_integrity(&self) -> bool {
        self.items.iter().all(|item| {
            let hash = hex::encode(Sha256::digest(item.content.as_bytes()));
            hash == item.content_hash
        })
    }

    /// Auto-collect evidence from rule validation results.
    pub fn collect_from_validation(
        &mut self,
        results: &[super::frameworks::RuleResult],
    ) -> Vec<EvidenceItem> {
        results
            .iter()
            .map(|r| {
                self.collect(
                    &r.rule.id,
                    &r.rule.title,
                    "validation-pipeline",
                    &format!(
                        "passed={} evidence={} remediation={:?}",
                        r.passed, r.evidence, r.remediation
                    ),
                )
            })
            .collect()
    }
}

impl Default for EvidenceCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_and_verify_integrity() {
        let mut collector = EvidenceCollector::new();
        collector.collect("SOC2-CC6.1", "RBAC Check", "k8s-api", "rbac=enabled");
        assert!(collector.verify_integrity());
        assert_eq!(collector.items().len(), 1);
    }

    fn sample_probes() -> Vec<ControlProbe> {
        vec![
            ControlProbe {
                control_id: "SOC2-CC6.1".to_string(),
                title: "RBAC enabled".to_string(),
                expression: "fact(\"rbac\") == \"enabled\"".to_string(),
                data_sources: vec!["policy-engine".to_string()],
                max_age_secs: 3600,
            },
            ControlProbe {
                control_id: "SOC2-CC7.2".to_string(),
                title: "Audit logging coverage".to_string(),
                expression: "metric(\"audit_coverage\") >= 0.95".to_string(),
                data_sources: vec!["metrics-store".to_string()],
                max_age_secs: 3600,
            },
        ]
    }

    #[test]
    fn probe_runner_collects_evidence_and_raises_findings() {
        let mut collector = EvidenceCollector::new();
        for probe in sample_probes() {
            collector.register_probe(probe);
        }
        let ctx = ProbeContext::new()
            .with_fact("rbac", "enabled")
            .with_metric("audit_coverage", 0.5);
        let outcomes = collector.run_probes(&ctx);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].passed);
        assert!(!outcomes[1].passed);
        // Failure raised a finding within the same cycle.
        assert_eq!(collector.findings().len(), 1);
        assert_eq!(collector.findings()[0].control_id, "SOC2-CC7.2");
        let coverage = collector.coverage();
        assert!(!coverage.is_complete());
        assert!(coverage.missing.contains("SOC2-CC7.2"));
    }

    #[test]
    fn signed_package_validates_offline() {
        let mut collector = EvidenceCollector::new();
        collector.collect("SOC2-CC6.1", "RBAC Check", "k8s-api", "rbac=enabled");
        let key = b"auditor-secret";
        let package = collector.package_control("SOC2-CC6.1", key).unwrap();
        assert!(package.verify_offline(key));
        assert!(!package.verify_offline(b"wrong-key"));
        let mut tampered = package.clone();
        tampered.items[0].content = "tampered".to_string();
        assert!(!tampered.verify_offline(key));
    }

    #[test]
    fn auditor_review_samples_controls() {
        let mut collector = EvidenceCollector::new();
        for probe in sample_probes() {
            collector.register_probe(probe);
        }
        let ctx = ProbeContext::new()
            .with_fact("rbac", "enabled")
            .with_metric("audit_coverage", 0.99);
        collector.run_probes(&ctx);

        let coverage = collector.coverage();
        assert!(coverage.is_complete());
        assert_eq!(coverage.coverage_ratio(), 1.0);

        let key = b"auditor-secret";
        let now = Utc::now();
        // Auditor samples every in-scope control.
        for control_id in &coverage.in_scope {
            let package = collector
                .package_control(control_id, key)
                .expect("sampled control must have evidence");
            assert!(package.verify_offline(key), "package must verify offline");
            assert!(!package.items.is_empty());
            for item in &package.items {
                assert!(
                    collector.is_in_window(item, now),
                    "evidence for {} must be in-window",
                    control_id
                );
                let hash = hex::encode(Sha256::digest(item.content.as_bytes()));
                assert_eq!(hash, item.content_hash, "evidence must be independently verifiable");
            }
        }
    }
}
