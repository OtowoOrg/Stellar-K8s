//! Evidence collection subsystem for compliance audits.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
}

impl EvidenceCollector {
    pub fn new() -> Self {
        Self { items: Vec::new() }
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
}
