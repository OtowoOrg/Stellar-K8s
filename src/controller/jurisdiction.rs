//! Jurisdictional Compliance Orchestrator
//!
//! Ensures Stellar nodes are physically placed in specific geographical
//! jurisdictions to comply with local financial regulations.
//!
//! # How it works
//!
//! When a `StellarNode` has `spec.placement.jurisdiction` set, the operator:
//!
//! 1. Maps the jurisdiction code to Kubernetes node labels
//!    (`topology.kubernetes.io/region` by default).
//! 2. Injects a `requiredDuringSchedulingIgnoredDuringExecution` `nodeAffinity`
//!    rule that restricts scheduling to nodes in the allowed regions.
//! 3. Appends any jurisdiction-specific `tolerations` to the pod spec.
//!
//! # Compliance Report
//!
//! The `compliance_report` function produces a per-node summary of the
//! physical location of all fleet assets, suitable for regulatory audits.

use k8s_openapi::api::core::v1::{
    NodeAffinity, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm, Toleration,
};
use kube::{Api, Client, ResourceExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::crd::{JurisdictionConfig, StellarNode};

/// Build a `NodeAffinity` that restricts scheduling to the regions allowed
/// by the given jurisdiction configuration.
///
/// Returns `None` if the jurisdiction has no regions configured (no-op).
pub fn build_jurisdiction_node_affinity(config: &JurisdictionConfig) -> Option<NodeAffinity> {
    if config.regions.is_empty() {
        warn!(
            jurisdiction = %config.code,
            "Jurisdiction has no regions configured; nodeAffinity will not be injected"
        );
        return None;
    }

    debug!(
        jurisdiction = %config.code,
        regions = ?config.regions,
        label_key = %config.label_key,
        "Injecting jurisdiction nodeAffinity"
    );

    Some(NodeAffinity {
        required_during_scheduling_ignored_during_execution: Some(NodeSelector {
            node_selector_terms: vec![NodeSelectorTerm {
                match_expressions: Some(vec![NodeSelectorRequirement {
                    key: config.label_key.clone(),
                    operator: "In".to_string(),
                    values: Some(config.regions.clone()),
                }]),
                ..Default::default()
            }],
        }),
        ..Default::default()
    })
}

/// Merge jurisdiction tolerations into an existing toleration list.
pub fn merge_jurisdiction_tolerations(existing: &mut Vec<Toleration>, config: &JurisdictionConfig) {
    for tol in &config.tolerations {
        // Avoid duplicates
        if !existing
            .iter()
            .any(|t| t.key == tol.key && t.value == tol.value)
        {
            existing.push(tol.clone());
        }
    }
}

/// A single entry in the compliance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceReportEntry {
    /// Kubernetes namespace of the StellarNode
    pub namespace: String,
    /// Name of the StellarNode resource
    pub name: String,
    /// Node type (Validator, Horizon, SorobanRpc)
    pub node_type: String,
    /// Jurisdiction code (e.g. "EU", "US") or "unconstrained" if none set
    pub jurisdiction: String,
    /// Allowed regions for this jurisdiction
    pub allowed_regions: Vec<String>,
    /// Label key used for region matching
    pub label_key: String,
    /// Whether jurisdiction enforcement is active
    pub enforced: bool,
}

/// Generate a compliance report for all StellarNode resources in the cluster.
///
/// Lists every node and reports its jurisdiction configuration, providing
/// operators with a fleet-wide view of physical location constraints.
pub async fn compliance_report(client: Client) -> Result<Vec<ComplianceReportEntry>, kube::Error> {
    let nodes: Api<StellarNode> = Api::all(client);
    let node_list = nodes.list(&Default::default()).await?;

    let entries = node_list
        .items
        .into_iter()
        .map(|node| {
            let jurisdiction = node.spec.placement.jurisdiction.as_ref();
            ComplianceReportEntry {
                namespace: node.namespace().unwrap_or_default(),
                name: node.name_any(),
                node_type: node.spec.node_type.to_string(),
                jurisdiction: jurisdiction
                    .map(|j| j.code.clone())
                    .unwrap_or_else(|| "unconstrained".to_string()),
                allowed_regions: jurisdiction.map(|j| j.regions.clone()).unwrap_or_default(),
                label_key: jurisdiction
                    .map(|j| j.label_key.clone())
                    .unwrap_or_else(|| "topology.kubernetes.io/region".to_string()),
                enforced: jurisdiction.is_some(),
            }
        })
        .collect();

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jurisdiction(code: &str, regions: &[&str]) -> JurisdictionConfig {
        JurisdictionConfig {
            code: code.to_string(),
            regions: regions.iter().map(|s| s.to_string()).collect(),
            label_key: "topology.kubernetes.io/region".to_string(),
            tolerations: vec![],
        }
    }

    #[test]
    fn test_build_jurisdiction_node_affinity_with_regions() {
        let config = make_jurisdiction("EU", &["eu-west-1", "eu-central-1"]);
        let affinity = build_jurisdiction_node_affinity(&config).unwrap();

        let selector = affinity
            .required_during_scheduling_ignored_during_execution
            .unwrap();
        assert_eq!(selector.node_selector_terms.len(), 1);

        let expr = &selector.node_selector_terms[0]
            .match_expressions
            .as_ref()
            .unwrap()[0];
        assert_eq!(expr.key, "topology.kubernetes.io/region");
        assert_eq!(expr.operator, "In");
        assert_eq!(
            expr.values.as_ref().unwrap(),
            &vec!["eu-west-1".to_string(), "eu-central-1".to_string()]
        );
    }

    #[test]
    fn test_build_jurisdiction_node_affinity_no_regions() {
        let config = make_jurisdiction("EU", &[]);
        let affinity = build_jurisdiction_node_affinity(&config);
        assert!(affinity.is_none());
    }

    #[test]
    fn test_merge_jurisdiction_tolerations_no_duplicates() {
        let mut existing = vec![Toleration {
            key: Some("jurisdiction".to_string()),
            value: Some("EU".to_string()),
            ..Default::default()
        }];
        let config = JurisdictionConfig {
            code: "EU".to_string(),
            regions: vec![],
            label_key: "topology.kubernetes.io/region".to_string(),
            tolerations: vec![Toleration {
                key: Some("jurisdiction".to_string()),
                value: Some("EU".to_string()),
                ..Default::default()
            }],
        };
        merge_jurisdiction_tolerations(&mut existing, &config);
        // Should not duplicate
        assert_eq!(existing.len(), 1);
    }

    #[test]
    fn test_merge_jurisdiction_tolerations_adds_new() {
        let mut existing: Vec<Toleration> = vec![];
        let config = JurisdictionConfig {
            code: "US".to_string(),
            regions: vec!["us-east-1".to_string()],
            label_key: "topology.kubernetes.io/region".to_string(),
            tolerations: vec![Toleration {
                key: Some("jurisdiction".to_string()),
                value: Some("US".to_string()),
                effect: Some("NoSchedule".to_string()),
                ..Default::default()
            }],
        };
        merge_jurisdiction_tolerations(&mut existing, &config);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].value, Some("US".to_string()));
    }

    #[test]
    fn test_custom_label_key() {
        let config = JurisdictionConfig {
            code: "SG".to_string(),
            regions: vec!["ap-southeast-1".to_string()],
            label_key: "cloud.example.com/jurisdiction-region".to_string(),
            tolerations: vec![],
        };
        let affinity = build_jurisdiction_node_affinity(&config).unwrap();
        let selector = affinity
            .required_during_scheduling_ignored_during_execution
            .unwrap();
        let expr = &selector.node_selector_terms[0]
            .match_expressions
            .as_ref()
            .unwrap()[0];
        assert_eq!(expr.key, "cloud.example.com/jurisdiction-region");
    }
}
