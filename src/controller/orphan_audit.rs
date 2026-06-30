//! Orphaned Kubernetes resource auditor for post-uninstall cleanup verification.
//!
//! After uninstalling the Stellar operator, resources that were managed by it
//! (ConfigMaps, Services, PersistentVolumeClaims) may be left behind if finalizers
//! were not processed correctly. This module audits namespaces for such orphaned
//! resources and produces structured reports.
//!
//! # Usage
//!
//! ```rust,ignore
//! let auditor = OrphanAuditor::new(client);
//! let report = auditor.audit_namespace("stellar").await?;
//! println!("{}", format_report_table(&report));
//! ```

use chrono::Utc;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, PersistentVolumeClaim, Service};
use kube::{
    api::{Api, ListParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{debug, warn};

use crate::crd::StellarNode;
use crate::error::Result;

/// Label used to identify resources managed by the stellar-operator.
pub const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
/// Value of the managed-by label for stellar-operator resources.
pub const MANAGED_BY_VALUE: &str = "stellar-operator";

/// A single orphaned Kubernetes resource discovered during an audit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanedResource {
    /// Kubernetes resource kind (e.g. "ConfigMap", "Service", "PersistentVolumeClaim").
    pub kind: String,
    /// Name of the resource.
    pub name: String,
    /// Namespace the resource resides in.
    pub namespace: String,
    /// Labels attached to the resource at the time of audit.
    pub labels: BTreeMap<String, String>,
    /// Approximate age of the resource in seconds since creation.
    pub age_seconds: i64,
    /// Human-readable reason why this resource is considered orphaned.
    pub reason: String,
}

impl Default for OrphanedResource {
    fn default() -> Self {
        Self {
            kind: String::new(),
            name: String::new(),
            namespace: String::new(),
            labels: BTreeMap::new(),
            age_seconds: 0,
            reason: String::new(),
        }
    }
}

/// Summary statistics for an audit report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AuditSummary {
    /// Total number of orphaned resources found.
    pub total_orphaned: usize,
    /// Number of orphaned ConfigMaps.
    pub orphaned_config_maps: usize,
    /// Number of orphaned Services.
    pub orphaned_services: usize,
    /// Number of orphaned PersistentVolumeClaims.
    pub orphaned_pvcs: usize,
}

/// A complete audit report for a single namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanAuditReport {
    /// RFC 3339 timestamp when the audit was performed.
    pub timestamp: String,
    /// Name of the cluster (from the kube context, or "unknown").
    pub cluster_name: String,
    /// Namespace that was audited.
    pub namespace: String,
    /// All orphaned resources discovered in this namespace.
    pub orphaned_resources: Vec<OrphanedResource>,
    /// Summary statistics for quick consumption.
    pub summary: AuditSummary,
}

impl OrphanAuditReport {
    /// Create a new empty report for a namespace.
    fn new(namespace: &str, cluster_name: &str) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            cluster_name: cluster_name.to_string(),
            namespace: namespace.to_string(),
            orphaned_resources: Vec::new(),
            summary: AuditSummary::default(),
        }
    }

    /// Recompute summary from the current orphaned_resources list.
    fn recompute_summary(&mut self) {
        let mut summary = AuditSummary::default();
        for r in &self.orphaned_resources {
            summary.total_orphaned += 1;
            match r.kind.as_str() {
                "ConfigMap" => summary.orphaned_config_maps += 1,
                "Service" => summary.orphaned_services += 1,
                "PersistentVolumeClaim" => summary.orphaned_pvcs += 1,
                _ => {}
            }
        }
        self.summary = summary;
    }
}

/// Audits Kubernetes namespaces for resources orphaned after operator uninstall.
///
/// An orphaned resource is one that carries the `app.kubernetes.io/managed-by=stellar-operator`
/// label but whose owning `StellarNode` no longer exists in the same namespace.
pub struct OrphanAuditor {
    client: Client,
    /// Cluster name reported in audit results. Defaults to "unknown".
    cluster_name: String,
}

impl OrphanAuditor {
    /// Create a new auditor backed by the given Kubernetes client.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            cluster_name: "unknown".to_string(),
        }
    }

    /// Set the cluster name reported in audit results.
    pub fn with_cluster_name(mut self, name: impl Into<String>) -> Self {
        self.cluster_name = name.into();
        self
    }

    /// Audit a single namespace for orphaned resources.
    ///
    /// Lists ConfigMaps, Services, and PersistentVolumeClaims bearing the
    /// `app.kubernetes.io/managed-by=stellar-operator` label, then checks
    /// whether the owning `StellarNode` still exists.  Resources whose owner
    /// is absent are recorded as orphaned.
    pub async fn audit_namespace(&self, namespace: &str) -> Result<OrphanAuditReport> {
        let mut report = OrphanAuditReport::new(namespace, &self.cluster_name);

        // Fetch the set of StellarNode names still present in this namespace.
        let existing_nodes = self.list_existing_nodes(namespace).await?;
        debug!(
            namespace = namespace,
            node_count = existing_nodes.len(),
            "fetched existing StellarNodes"
        );

        // Audit each resource kind.
        let cm_orphans = self
            .audit_config_maps(namespace, &existing_nodes)
            .await?;
        let svc_orphans = self
            .audit_services(namespace, &existing_nodes)
            .await?;
        let pvc_orphans = self
            .audit_pvcs(namespace, &existing_nodes)
            .await?;

        report.orphaned_resources.extend(cm_orphans);
        report.orphaned_resources.extend(svc_orphans);
        report.orphaned_resources.extend(pvc_orphans);
        report.recompute_summary();

        Ok(report)
    }

    /// Audit every namespace in the cluster and return one report per namespace.
    ///
    /// Namespaces that produce errors are skipped with a warning rather than
    /// aborting the entire run, so a partial result is always returned.
    pub async fn audit_all_namespaces(&self) -> Result<Vec<OrphanAuditReport>> {
        let ns_api: Api<Namespace> = Api::all(self.client.clone());
        let namespaces = ns_api
            .list(&ListParams::default())
            .await
            .map_err(crate::error::Error::KubeError)?;

        let mut reports = Vec::new();
        for ns in &namespaces.items {
            let ns_name = ns.name_any();
            match self.audit_namespace(&ns_name).await {
                Ok(report) => reports.push(report),
                Err(e) => {
                    warn!(namespace = %ns_name, error = ?e, "skipping namespace due to audit error");
                }
            }
        }
        Ok(reports)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Return the set of StellarNode names currently present in `namespace`.
    async fn list_existing_nodes(
        &self,
        namespace: &str,
    ) -> Result<std::collections::HashSet<String>> {
        let node_api: Api<StellarNode> = Api::namespaced(self.client.clone(), namespace);
        let nodes = node_api
            .list(&ListParams::default())
            .await
            .map_err(crate::error::Error::KubeError)?;
        Ok(nodes.items.iter().map(|n| n.name_any()).collect())
    }

    /// Build `ListParams` that filter by the managed-by label.
    fn managed_by_params() -> ListParams {
        ListParams::default()
            .labels(&format!("{MANAGED_BY_LABEL}={MANAGED_BY_VALUE}"))
    }

    /// Compute age in seconds for a resource given its creation timestamp.
    fn age_seconds(creation: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>) -> i64 {
        creation
            .map(|t| {
                let now = Utc::now();
                (now - t.0).num_seconds().max(0)
            })
            .unwrap_or(0)
    }

    /// Determine which StellarNode this resource belongs to.
    ///
    /// Checks `ownerReferences` first, then falls back to the
    /// `app.kubernetes.io/instance` label.
    fn owner_node_name(
        owner_refs: Option<&Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference>>,
        labels: &BTreeMap<String, String>,
    ) -> Option<String> {
        // Prefer ownerReference pointing to a StellarNode.
        if let Some(refs) = owner_refs {
            for r in refs {
                if r.kind == "StellarNode" {
                    return Some(r.name.clone());
                }
            }
        }
        // Fall back to the instance label.
        labels
            .get("app.kubernetes.io/instance")
            .or_else(|| labels.get("stellar.org/node-name"))
            .cloned()
    }

    /// Audit ConfigMaps in `namespace` against `existing_nodes`.
    async fn audit_config_maps(
        &self,
        namespace: &str,
        existing_nodes: &std::collections::HashSet<String>,
    ) -> Result<Vec<OrphanedResource>> {
        let api: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);
        let cms = api
            .list(&Self::managed_by_params())
            .await
            .map_err(crate::error::Error::KubeError)?;

        let mut orphans = Vec::new();
        for cm in &cms.items {
            let labels: BTreeMap<String, String> = cm
                .metadata
                .labels
                .clone()
                .unwrap_or_default();
            let owner = Self::owner_node_name(
                cm.metadata.owner_references.as_ref(),
                &labels,
            );
            let is_orphaned = match &owner {
                Some(node_name) => !existing_nodes.contains(node_name.as_str()),
                None => true, // no owner identifiable → treat as orphaned
            };
            if is_orphaned {
                let reason = match &owner {
                    Some(name) => format!("owning StellarNode '{name}' no longer exists"),
                    None => "no owning StellarNode could be identified".to_string(),
                };
                orphans.push(OrphanedResource {
                    kind: "ConfigMap".to_string(),
                    name: cm.name_any(),
                    namespace: namespace.to_string(),
                    labels,
                    age_seconds: Self::age_seconds(cm.metadata.creation_timestamp.as_ref()),
                    reason,
                });
            }
        }
        Ok(orphans)
    }

    /// Audit Services in `namespace` against `existing_nodes`.
    async fn audit_services(
        &self,
        namespace: &str,
        existing_nodes: &std::collections::HashSet<String>,
    ) -> Result<Vec<OrphanedResource>> {
        let api: Api<Service> = Api::namespaced(self.client.clone(), namespace);
        let services = api
            .list(&Self::managed_by_params())
            .await
            .map_err(crate::error::Error::KubeError)?;

        let mut orphans = Vec::new();
        for svc in &services.items {
            let labels: BTreeMap<String, String> = svc
                .metadata
                .labels
                .clone()
                .unwrap_or_default();
            let owner = Self::owner_node_name(
                svc.metadata.owner_references.as_ref(),
                &labels,
            );
            let is_orphaned = match &owner {
                Some(node_name) => !existing_nodes.contains(node_name.as_str()),
                None => true,
            };
            if is_orphaned {
                let reason = match &owner {
                    Some(name) => format!("owning StellarNode '{name}' no longer exists"),
                    None => "no owning StellarNode could be identified".to_string(),
                };
                orphans.push(OrphanedResource {
                    kind: "Service".to_string(),
                    name: svc.name_any(),
                    namespace: namespace.to_string(),
                    labels,
                    age_seconds: Self::age_seconds(svc.metadata.creation_timestamp.as_ref()),
                    reason,
                });
            }
        }
        Ok(orphans)
    }

    /// Audit PersistentVolumeClaims in `namespace` against `existing_nodes`.
    async fn audit_pvcs(
        &self,
        namespace: &str,
        existing_nodes: &std::collections::HashSet<String>,
    ) -> Result<Vec<OrphanedResource>> {
        let api: Api<PersistentVolumeClaim> =
            Api::namespaced(self.client.clone(), namespace);
        let pvcs = api
            .list(&Self::managed_by_params())
            .await
            .map_err(crate::error::Error::KubeError)?;

        let mut orphans = Vec::new();
        for pvc in &pvcs.items {
            let labels: BTreeMap<String, String> = pvc
                .metadata
                .labels
                .clone()
                .unwrap_or_default();
            let owner = Self::owner_node_name(
                pvc.metadata.owner_references.as_ref(),
                &labels,
            );
            let is_orphaned = match &owner {
                Some(node_name) => !existing_nodes.contains(node_name.as_str()),
                None => true,
            };
            if is_orphaned {
                let reason = match &owner {
                    Some(name) => format!("owning StellarNode '{name}' no longer exists"),
                    None => "no owning StellarNode could be identified".to_string(),
                };
                orphans.push(OrphanedResource {
                    kind: "PersistentVolumeClaim".to_string(),
                    name: pvc.name_any(),
                    namespace: namespace.to_string(),
                    labels,
                    age_seconds: Self::age_seconds(pvc.metadata.creation_timestamp.as_ref()),
                    reason,
                });
            }
        }
        Ok(orphans)
    }
}

// ---------------------------------------------------------------------------
// Report formatting
// ---------------------------------------------------------------------------

/// Format an audit report as a human-readable table.
///
/// Columns: KIND | NAME | NAMESPACE | AGE | REASON
pub fn format_report_table(report: &OrphanAuditReport) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "Orphan Audit Report — namespace: {} | cluster: {} | at: {}\n",
        report.namespace, report.cluster_name, report.timestamp
    ));
    out.push_str(&format!(
        "Total orphaned: {}  (ConfigMaps: {}, Services: {}, PVCs: {})\n",
        report.summary.total_orphaned,
        report.summary.orphaned_config_maps,
        report.summary.orphaned_services,
        report.summary.orphaned_pvcs,
    ));

    if report.orphaned_resources.is_empty() {
        out.push_str("No orphaned resources found.\n");
        return out;
    }

    // Header
    out.push_str(&format!(
        "\n{:<26} {:<40} {:<20} {:>10}  {}\n",
        "KIND", "NAME", "NAMESPACE", "AGE(s)", "REASON"
    ));
    out.push_str(&"-".repeat(130));
    out.push('\n');

    for r in &report.orphaned_resources {
        out.push_str(&format!(
            "{:<26} {:<40} {:<20} {:>10}  {}\n",
            r.kind, r.name, r.namespace, r.age_seconds, r.reason
        ));
    }
    out
}

/// Serialize an audit report to a pretty-printed JSON string.
pub fn format_report_json(report: &OrphanAuditReport) -> Result<String> {
    serde_json::to_string_pretty(report).map_err(crate::error::Error::SerializationError)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_orphaned_resource(kind: &str, name: &str) -> OrphanedResource {
        let mut labels = BTreeMap::new();
        labels.insert(MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string());
        OrphanedResource {
            kind: kind.to_string(),
            name: name.to_string(),
            namespace: "stellar".to_string(),
            labels,
            age_seconds: 3600,
            reason: format!("owning StellarNode '{name}-node' no longer exists"),
        }
    }

    fn sample_report() -> OrphanAuditReport {
        let mut report = OrphanAuditReport::new("stellar", "test-cluster");
        report.orphaned_resources.push(sample_orphaned_resource(
            "ConfigMap",
            "my-validator-config",
        ));
        report.orphaned_resources.push(sample_orphaned_resource(
            "Service",
            "my-validator-svc",
        ));
        report
            .orphaned_resources
            .push(sample_orphaned_resource("PersistentVolumeClaim", "data-pvc"));
        report.recompute_summary();
        report
    }

    // ------------------------------------------------------------------
    // OrphanedResource construction and Default
    // ------------------------------------------------------------------

    #[test]
    fn orphaned_resource_default_has_empty_fields() {
        let r = OrphanedResource::default();
        assert!(r.kind.is_empty());
        assert!(r.name.is_empty());
        assert!(r.namespace.is_empty());
        assert!(r.labels.is_empty());
        assert_eq!(r.age_seconds, 0);
        assert!(r.reason.is_empty());
    }

    #[test]
    fn orphaned_resource_construction_stores_all_fields() {
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "stellar".to_string());

        let r = OrphanedResource {
            kind: "ConfigMap".to_string(),
            name: "my-cm".to_string(),
            namespace: "stellar".to_string(),
            labels: labels.clone(),
            age_seconds: 120,
            reason: "node gone".to_string(),
        };

        assert_eq!(r.kind, "ConfigMap");
        assert_eq!(r.name, "my-cm");
        assert_eq!(r.namespace, "stellar");
        assert_eq!(r.labels, labels);
        assert_eq!(r.age_seconds, 120);
        assert_eq!(r.reason, "node gone");
    }

    // ------------------------------------------------------------------
    // OrphanAuditReport serialization round-trip
    // ------------------------------------------------------------------

    #[test]
    fn report_serialization_roundtrip() {
        let report = sample_report();

        let json = serde_json::to_string(&report).expect("serialize");
        let restored: OrphanAuditReport = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(report, restored);
    }

    #[test]
    fn report_serialization_contains_expected_keys() {
        let report = sample_report();
        let json = serde_json::to_string(&report).expect("serialize");

        assert!(json.contains("\"timestamp\""));
        assert!(json.contains("\"cluster_name\""));
        assert!(json.contains("\"namespace\""));
        assert!(json.contains("\"orphaned_resources\""));
        assert!(json.contains("\"summary\""));
    }

    #[test]
    fn report_summary_counts_are_correct_after_recompute() {
        let report = sample_report();
        assert_eq!(report.summary.total_orphaned, 3);
        assert_eq!(report.summary.orphaned_config_maps, 1);
        assert_eq!(report.summary.orphaned_services, 1);
        assert_eq!(report.summary.orphaned_pvcs, 1);
    }

    #[test]
    fn empty_report_has_zero_summary() {
        let report = OrphanAuditReport::new("default", "my-cluster");
        assert_eq!(report.summary.total_orphaned, 0);
        assert_eq!(report.summary.orphaned_config_maps, 0);
        assert_eq!(report.summary.orphaned_services, 0);
        assert_eq!(report.summary.orphaned_pvcs, 0);
    }

    // ------------------------------------------------------------------
    // format_report_table column checks
    // ------------------------------------------------------------------

    #[test]
    fn format_report_table_contains_header_columns() {
        let report = sample_report();
        let table = format_report_table(&report);

        assert!(table.contains("KIND"), "table must contain KIND column");
        assert!(table.contains("NAME"), "table must contain NAME column");
        assert!(table.contains("NAMESPACE"), "table must contain NAMESPACE column");
        assert!(table.contains("AGE"), "table must contain AGE column");
        assert!(table.contains("REASON"), "table must contain REASON column");
    }

    #[test]
    fn format_report_table_contains_resource_data() {
        let report = sample_report();
        let table = format_report_table(&report);

        assert!(table.contains("ConfigMap"));
        assert!(table.contains("my-validator-config"));
        assert!(table.contains("Service"));
        assert!(table.contains("my-validator-svc"));
        assert!(table.contains("PersistentVolumeClaim"));
        assert!(table.contains("data-pvc"));
    }

    #[test]
    fn format_report_table_shows_namespace_and_cluster() {
        let report = sample_report();
        let table = format_report_table(&report);

        assert!(table.contains("stellar"));
        assert!(table.contains("test-cluster"));
    }

    #[test]
    fn format_report_table_empty_report_says_no_orphans() {
        let report = OrphanAuditReport::new("default", "cluster");
        let table = format_report_table(&report);

        assert!(table.contains("No orphaned resources found."));
    }

    #[test]
    fn format_report_table_shows_summary_counts() {
        let report = sample_report();
        let table = format_report_table(&report);

        assert!(table.contains("Total orphaned: 3"));
        assert!(table.contains("ConfigMaps: 1"));
        assert!(table.contains("Services: 1"));
        assert!(table.contains("PVCs: 1"));
    }

    // ------------------------------------------------------------------
    // format_report_json
    // ------------------------------------------------------------------

    #[test]
    fn format_report_json_returns_valid_json() {
        let report = sample_report();
        let json = format_report_json(&report).expect("json formatting should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("output must be valid JSON");
        assert!(parsed.is_object());
    }

    #[test]
    fn format_report_json_roundtrip_matches_original() {
        let report = sample_report();
        let json = format_report_json(&report).expect("format json");
        let restored: OrphanAuditReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, restored);
    }

    // ------------------------------------------------------------------
    // OrphanAuditor::owner_node_name
    // ------------------------------------------------------------------

    #[test]
    fn owner_node_name_prefers_owner_reference() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;

        let refs = vec![OwnerReference {
            api_version: "stellar.org/v1alpha1".to_string(),
            kind: "StellarNode".to_string(),
            name: "my-validator".to_string(),
            uid: "abc-123".to_string(),
            ..Default::default()
        }];
        let mut labels = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/instance".to_string(),
            "other-node".to_string(),
        );

        let name = OrphanAuditor::owner_node_name(Some(&refs), &labels);
        assert_eq!(name.as_deref(), Some("my-validator"));
    }

    #[test]
    fn owner_node_name_falls_back_to_instance_label() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/instance".to_string(),
            "fallback-node".to_string(),
        );

        let name = OrphanAuditor::owner_node_name(None, &labels);
        assert_eq!(name.as_deref(), Some("fallback-node"));
    }

    #[test]
    fn owner_node_name_returns_none_when_no_clue() {
        let labels = BTreeMap::new();
        let name = OrphanAuditor::owner_node_name(None, &labels);
        assert!(name.is_none());
    }

    // ------------------------------------------------------------------
    // MANAGED_BY constants
    // ------------------------------------------------------------------

    #[test]
    fn managed_by_label_and_value_are_correct() {
        assert_eq!(MANAGED_BY_LABEL, "app.kubernetes.io/managed-by");
        assert_eq!(MANAGED_BY_VALUE, "stellar-operator");
    }
}
