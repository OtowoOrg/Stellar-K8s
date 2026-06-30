//! Compatibility matrix tests across Kubernetes minor versions.
//!
//! This module defines a static compatibility matrix for the Stellar-K8s operator,
//! covering the supported Kubernetes version range (1.27 – 1.30).  Tests run
//! entirely offline — no cluster connection is required.
//!
//! # Running
//!
//! ```bash
//! # Run all compatibility matrix tests
//! cargo test --test compat_matrix
//!
//! # Run with verbose output to see the ASCII table
//! cargo test --test compat_matrix -- --nocapture
//! ```
//!
//! See `docs/compat-matrix.md` for the full documentation.

use serde::{Deserialize, Serialize};
use serde_json;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A specific Kubernetes minor version (e.g., 1.30).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct K8sVersion {
    pub major: u32,
    pub minor: u32,
}

impl K8sVersion {
    /// Construct a new version.
    pub fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }
}

impl std::fmt::Display for K8sVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

// ---------------------------------------------------------------------------
// Feature enumeration
// ---------------------------------------------------------------------------

/// Features of the Stellar-K8s operator that are tested against each K8s version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompatibilityFeature {
    /// Installation of the `StellarNode` CRD and related CRDs.
    CrdInstallation,
    /// Basic reconciler loop (create / update / delete `StellarNode` resources).
    ReconcilerBasic,
    /// Finalizer registration and removal (preventing orphaned PVCs).
    FinalizerHandling,
    /// `/status` subresource support on the `StellarNode` CRD.
    StatusSubresource,
    /// Validating and mutating admission webhooks via the operator's webhook server.
    AdmissionWebhook,
    /// Dynamic PVC volume expansion (disk auto-scaling feature).
    PvcExpansion,
    /// Service-mesh sidecar injection (Istio / Linkerd mTLS overlay).
    ServiceMesh,
}

impl CompatibilityFeature {
    /// Return all variants in a deterministic order.
    pub fn all() -> Vec<CompatibilityFeature> {
        vec![
            CompatibilityFeature::CrdInstallation,
            CompatibilityFeature::ReconcilerBasic,
            CompatibilityFeature::FinalizerHandling,
            CompatibilityFeature::StatusSubresource,
            CompatibilityFeature::AdmissionWebhook,
            CompatibilityFeature::PvcExpansion,
            CompatibilityFeature::ServiceMesh,
        ]
    }

    /// Human-readable short label used in the ASCII table header.
    pub fn label(&self) -> &'static str {
        match self {
            CompatibilityFeature::CrdInstallation => "CRD Install",
            CompatibilityFeature::ReconcilerBasic => "Reconciler",
            CompatibilityFeature::FinalizerHandling => "Finalizers",
            CompatibilityFeature::StatusSubresource => "Status Sub.",
            CompatibilityFeature::AdmissionWebhook => "Adm. Webhook",
            CompatibilityFeature::PvcExpansion => "PVC Expand",
            CompatibilityFeature::ServiceMesh => "Svc Mesh",
        }
    }
}

impl std::fmt::Display for CompatibilityFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// Test result
// ---------------------------------------------------------------------------

/// Result of testing one feature against one K8s version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatTestResult {
    /// The Kubernetes version under test.
    pub version: K8sVersion,
    /// A short name identifying the test.
    pub test_name: String,
    /// Whether the feature is supported / passes on this version.
    pub passed: bool,
    /// Optional notes (e.g. known limitations, required flags, beta APIs).
    pub notes: Option<String>,
}

// ---------------------------------------------------------------------------
// Compatibility matrix
// ---------------------------------------------------------------------------

/// Aggregated compatibility results for all tested K8s versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityMatrix {
    /// Ordered list of K8s versions covered by the matrix.
    pub k8s_versions: Vec<K8sVersion>,
    /// Flat list of all individual test results.
    pub results: Vec<CompatTestResult>,
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Returns the list of Kubernetes minor versions that Stellar-K8s officially
/// supports, ordered from oldest to newest.
///
/// Per the README prerequisites: "Kubernetes cluster (1.28+)" — however the
/// operator also supports 1.27 for legacy compatibility.  The current CI
/// target is 1.30 (k8s-openapi `v1_30` feature).
pub fn supported_k8s_versions() -> Vec<K8sVersion> {
    vec![
        K8sVersion::new(1, 27),
        K8sVersion::new(1, 28),
        K8sVersion::new(1, 29),
        K8sVersion::new(1, 30),
    ]
}

/// Evaluate feature compatibility for a single K8s version.
///
/// The rules encoded here reflect the stable / beta graduation status of each
/// API feature across minor versions:
///
/// | Feature              | Stable since |
/// |----------------------|-------------|
/// | CRD v1               | 1.16        |
/// | Status subresource   | 1.16        |
/// | Admission webhooks   | 1.16        |
/// | PVC expansion (GA)   | 1.24        |
/// | Finalizers           | 1.7         |
/// | Service mesh sidecar | external; 1.27+ recommended |
pub fn check_api_version_compatibility(version: &K8sVersion) -> Vec<CompatTestResult> {
    let mut results = Vec::new();

    for feature in CompatibilityFeature::all() {
        let (passed, notes) = evaluate_feature(&feature, version);
        results.push(CompatTestResult {
            version: version.clone(),
            test_name: format!("k8s_{}_{}_compat", version, feature.label().to_lowercase().replace(' ', "_").replace('.', "")),
            passed,
            notes,
        });
    }

    results
}

/// Internal helper: determine pass/fail and optional notes for a feature on a
/// given K8s version.
fn evaluate_feature(
    feature: &CompatibilityFeature,
    version: &K8sVersion,
) -> (bool, Option<String>) {
    // Only major == 1 is in the supported range for now.
    if version.major != 1 {
        return (false, Some(format!("Major version {} not supported", version.major)));
    }

    match feature {
        // CRD v1 is GA since 1.16 — always passes for 1.27+.
        CompatibilityFeature::CrdInstallation => (true, None),

        // kube-rs reconciler requires at minimum 1.20 — always passes for 1.27+.
        CompatibilityFeature::ReconcilerBasic => (true, None),

        // Finalizers are GA since 1.7 — always passes.
        CompatibilityFeature::FinalizerHandling => (true, None),

        // Status subresource on CRD v1 is GA since 1.16.
        CompatibilityFeature::StatusSubresource => (true, None),

        // Admission webhooks are GA since 1.16.
        CompatibilityFeature::AdmissionWebhook => (true, None),

        // PVC volume expansion promoted to GA in 1.24.
        // For 1.27+ it is always available without any feature gate.
        CompatibilityFeature::PvcExpansion => {
            if version.minor >= 24 {
                (true, None)
            } else {
                (
                    false,
                    Some("PVC expansion (ExpandPersistentVolumes) not GA before 1.24".to_string()),
                )
            }
        }

        // Service-mesh sidecar injection is not a core K8s API; it depends on
        // the CNI and mesh controller installed in the cluster.  We mark it as
        // passing on 1.27+ since that is the minimum recommended version for
        // ambient/sidecar mode of both Istio and Linkerd.
        CompatibilityFeature::ServiceMesh => {
            if version.minor >= 27 {
                (true, Some("Requires Istio ≥ 1.17 or Linkerd ≥ 2.14 installed separately".to_string()))
            } else {
                (false, Some("Service-mesh sidecar not validated below 1.27".to_string()))
            }
        }
    }
}

/// Assemble the full `CompatibilityMatrix` by running every feature check
/// against every supported version.
pub fn build_compatibility_matrix() -> CompatibilityMatrix {
    let versions = supported_k8s_versions();
    let mut all_results = Vec::new();

    for version in &versions {
        let mut version_results = check_api_version_compatibility(version);
        all_results.append(&mut version_results);
    }

    CompatibilityMatrix {
        k8s_versions: versions,
        results: all_results,
    }
}

// ---------------------------------------------------------------------------
// Reporting helpers
// ---------------------------------------------------------------------------

/// Format the compatibility matrix as a human-readable ASCII table.
///
/// ```text
/// Stellar-K8s Compatibility Matrix
/// =================================
/// Version  | CRD Install | Reconciler | Finalizers | Status Sub. | Adm. Webhook | PVC Expand | Svc Mesh
/// ---------+-------------+------------+------------+-------------+--------------+------------+---------
/// 1.27     | ✓           | ✓          | ✓          | ✓           | ✓            | ✓          | ✓
/// 1.28     | ✓           | ✓          | ✓          | ✓           | ✓            | ✓          | ✓
/// ...
/// ```
pub fn print_matrix_table(matrix: &CompatibilityMatrix) -> String {
    let features = CompatibilityFeature::all();
    let col_width = 13usize;

    // --- Header ---
    let title = "Stellar-K8s Compatibility Matrix";
    let separator: String = "=".repeat(title.len());

    let mut header_cells = vec!["Version ".to_string()];
    for f in &features {
        let label = f.label();
        // Right-pad to col_width
        header_cells.push(format!(" {:<width$}", label, width = col_width - 1));
    }
    let header_row = header_cells.join("|");

    let divider: String = header_cells
        .iter()
        .map(|c| "-".repeat(c.len()))
        .collect::<Vec<_>>()
        .join("+");

    // --- Data rows ---
    let mut data_rows: Vec<String> = Vec::new();
    for version in &matrix.k8s_versions {
        let mut cells = vec![format!("{:<8}", version.to_string())];
        for feature in &features {
            // Find the result for this (version, feature) pair
            let passed = matrix
                .results
                .iter()
                .find(|r| &r.version == version && r.test_name.contains(&feature.label().to_lowercase().replace(' ', "_").replace('.', "")))
                .map(|r| r.passed)
                .unwrap_or(false);

            let symbol = if passed { "✓" } else { "✗" };
            cells.push(format!(" {:<width$}", symbol, width = col_width - 1));
        }
        data_rows.push(cells.join("|"));
    }

    // --- Assemble ---
    let mut lines = vec![
        title.to_string(),
        separator,
        header_row,
        divider,
    ];
    lines.extend(data_rows);
    lines.join("\n")
}

/// Export the compatibility matrix as a `serde_json::Value`.
///
/// The returned object contains:
/// - `"versions"`: array of version strings
/// - `"features"`: array of feature labels
/// - `"results"`: array of result objects with `version`, `feature`, `passed`, `notes`
/// - `"summary"`: object with `total`, `passed`, `failed` counts
pub fn matrix_to_json(matrix: &CompatibilityMatrix) -> serde_json::Value {
    let version_strings: Vec<String> = matrix.k8s_versions.iter().map(|v| v.to_string()).collect();
    let feature_labels: Vec<&str> = CompatibilityFeature::all().iter().map(|f| f.label()).collect();

    let result_entries: Vec<serde_json::Value> = matrix
        .results
        .iter()
        .map(|r| {
            let mut obj = serde_json::json!({
                "version": r.version.to_string(),
                "test_name": r.test_name,
                "passed": r.passed,
            });
            if let Some(note) = &r.notes {
                obj["notes"] = serde_json::Value::String(note.clone());
            } else {
                obj["notes"] = serde_json::Value::Null;
            }
            obj
        })
        .collect();

    let total = matrix.results.len();
    let passed_count = matrix.results.iter().filter(|r| r.passed).count();
    let failed_count = total - passed_count;

    serde_json::json!({
        "versions": version_strings,
        "features": feature_labels,
        "results": result_entries,
        "summary": {
            "total": total,
            "passed": passed_count,
            "failed": failed_count,
        }
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that exactly 4 supported versions are returned and that the
    /// range spans 1.27 – 1.30 inclusive.
    #[test]
    fn test_supported_versions_range() {
        let versions = supported_k8s_versions();

        assert_eq!(
            versions.len(),
            4,
            "Expected exactly 4 supported K8s versions (1.27, 1.28, 1.29, 1.30)"
        );

        let min = versions.iter().min().expect("versions must not be empty");
        let max = versions.iter().max().expect("versions must not be empty");

        assert_eq!(min.major, 1);
        assert_eq!(min.minor, 27, "Minimum supported version must be 1.27");

        assert_eq!(max.major, 1);
        assert_eq!(max.minor, 30, "Maximum supported version must be 1.30");

        // Ensure all four versions are present
        let minor_set: std::collections::HashSet<u32> = versions.iter().map(|v| v.minor).collect();
        for minor in [27u32, 28, 29, 30] {
            assert!(
                minor_set.contains(&minor),
                "Version 1.{} must be in supported_k8s_versions()",
                minor
            );
        }
    }

    /// Builds the full matrix and checks that every supported version appears
    /// at least once in the results.
    #[test]
    fn test_compat_matrix_construction() {
        let matrix = build_compatibility_matrix();

        assert_eq!(
            matrix.k8s_versions.len(),
            4,
            "Matrix must cover 4 K8s versions"
        );

        let expected_result_count = 4 * CompatibilityFeature::all().len();
        assert_eq!(
            matrix.results.len(),
            expected_result_count,
            "Matrix must contain {} results ({} versions × {} features)",
            expected_result_count,
            4,
            CompatibilityFeature::all().len()
        );

        // Every supported version must appear in k8s_versions
        let supported = supported_k8s_versions();
        for version in &supported {
            assert!(
                matrix.k8s_versions.contains(version),
                "Version {} missing from matrix.k8s_versions",
                version
            );
        }

        // Every version must have at least one result entry
        for version in &supported {
            let count = matrix.results.iter().filter(|r| &r.version == version).count();
            assert!(
                count > 0,
                "No results found for version {} in the matrix",
                version
            );
        }
    }

    /// Verifies that `matrix_to_json` produces an object with the required
    /// top-level keys and correct summary counts.
    #[test]
    fn test_matrix_json_export() {
        let matrix = build_compatibility_matrix();
        let json = matrix_to_json(&matrix);

        // Required top-level keys
        assert!(json.get("versions").is_some(), "JSON must have 'versions' key");
        assert!(json.get("features").is_some(), "JSON must have 'features' key");
        assert!(json.get("results").is_some(), "JSON must have 'results' key");
        assert!(json.get("summary").is_some(), "JSON must have 'summary' key");

        // versions array must have 4 entries
        let versions_arr = json["versions"].as_array().expect("'versions' must be an array");
        assert_eq!(versions_arr.len(), 4, "'versions' array must contain 4 entries");

        // features array must have one entry per CompatibilityFeature variant
        let features_arr = json["features"].as_array().expect("'features' must be an array");
        assert_eq!(
            features_arr.len(),
            CompatibilityFeature::all().len(),
            "'features' array length must match CompatibilityFeature::all()"
        );

        // summary must have total / passed / failed
        let summary = &json["summary"];
        assert!(summary.get("total").is_some(), "summary must have 'total'");
        assert!(summary.get("passed").is_some(), "summary must have 'passed'");
        assert!(summary.get("failed").is_some(), "summary must have 'failed'");

        // total == passed + failed
        let total = summary["total"].as_u64().expect("total must be a number");
        let passed = summary["passed"].as_u64().expect("passed must be a number");
        let failed = summary["failed"].as_u64().expect("failed must be a number");
        assert_eq!(
            total,
            passed + failed,
            "summary.total must equal passed + failed"
        );

        // results array must have the expected number of entries
        let results_arr = json["results"].as_array().expect("'results' must be an array");
        assert_eq!(
            results_arr.len(),
            (4 * CompatibilityFeature::all().len()),
            "'results' array length must equal versions × features"
        );
    }

    /// Verifies that `print_matrix_table` produces a string containing the
    /// version numbers and feature column headers.
    #[test]
    fn test_matrix_table_format() {
        let matrix = build_compatibility_matrix();
        let table = print_matrix_table(&matrix);

        // Title
        assert!(
            table.contains("Stellar-K8s Compatibility Matrix"),
            "Table must contain the title"
        );

        // Version column entries
        for version in supported_k8s_versions() {
            assert!(
                table.contains(&version.to_string()),
                "Table must contain version {}",
                version
            );
        }

        // Feature column headers
        for feature in CompatibilityFeature::all() {
            assert!(
                table.contains(feature.label()),
                "Table must contain feature label '{}'",
                feature.label()
            );
        }

        // Should use ✓ or ✗ symbols
        assert!(
            table.contains('✓') || table.contains('✗'),
            "Table must contain pass/fail symbols"
        );
    }

    /// Ensures every `CompatibilityFeature` variant is represented in the
    /// results returned by `check_api_version_compatibility` for the current
    /// target version (1.30).
    #[test]
    fn test_feature_coverage() {
        let current = K8sVersion::new(1, 30);
        let results = check_api_version_compatibility(&current);

        let all_features = CompatibilityFeature::all();

        assert_eq!(
            results.len(),
            all_features.len(),
            "check_api_version_compatibility must return one result per CompatibilityFeature variant"
        );

        // Verify each feature variant appears exactly once (by checking
        // that the result test_name contains the feature label substring)
        for feature in &all_features {
            let key = feature.label().to_lowercase().replace(' ', "_").replace('.', "");
            let found = results.iter().any(|r| r.test_name.contains(&key));
            assert!(
                found,
                "Feature '{}' (key '{}') not found in results",
                feature.label(),
                key
            );
        }
    }

    /// Checks that `K8sVersion`'s `Display` implementation formats correctly.
    #[test]
    fn test_k8s_version_display() {
        let v1_27 = K8sVersion::new(1, 27);
        let v1_30 = K8sVersion::new(1, 30);
        let v2_00 = K8sVersion::new(2, 0);

        assert_eq!(v1_27.to_string(), "1.27");
        assert_eq!(v1_30.to_string(), "1.30");
        assert_eq!(v2_00.to_string(), "2.0");

        // Display should also be usable in format!
        assert_eq!(format!("K8s {}", v1_27), "K8s 1.27");
    }

    /// Confirms that 1.27 is the minimum supported version.
    #[test]
    fn test_minimum_supported_version() {
        let versions = supported_k8s_versions();
        let min = versions
            .iter()
            .min()
            .expect("supported_k8s_versions must be non-empty");

        assert_eq!(
            min,
            &K8sVersion::new(1, 27),
            "Minimum supported K8s version must be 1.27"
        );
    }

    /// Asserts that the current target version (1.30) has all features passing.
    #[test]
    fn test_current_version_fully_supported() {
        let current = K8sVersion::new(1, 30);
        let results = check_api_version_compatibility(&current);

        let failed: Vec<&CompatTestResult> = results.iter().filter(|r| !r.passed).collect();

        assert!(
            failed.is_empty(),
            "Version 1.30 (current target) must have all features passing. \
             Failed features: {:?}",
            failed
                .iter()
                .map(|r| r.test_name.as_str())
                .collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Additional coverage tests
    // -----------------------------------------------------------------------

    /// Validates ordering — versions must come out in ascending order.
    #[test]
    fn test_supported_versions_ordered() {
        let versions = supported_k8s_versions();
        let mut sorted = versions.clone();
        sorted.sort();
        assert_eq!(versions, sorted, "supported_k8s_versions() must be sorted ascending");
    }

    /// Checks that version 1.30 appears in the matrix and its results are all passing.
    #[test]
    fn test_matrix_current_version_all_pass() {
        let matrix = build_compatibility_matrix();
        let current = K8sVersion::new(1, 30);

        assert!(
            matrix.k8s_versions.contains(&current),
            "Matrix must contain version 1.30"
        );

        let v130_results: Vec<&CompatTestResult> = matrix
            .results
            .iter()
            .filter(|r| r.version == current)
            .collect();

        assert!(
            !v130_results.is_empty(),
            "Matrix must contain results for version 1.30"
        );

        for result in &v130_results {
            assert!(
                result.passed,
                "Feature test '{}' must pass for version 1.30",
                result.test_name
            );
        }
    }

    /// Ensures that all results have non-empty test names.
    #[test]
    fn test_all_results_have_test_names() {
        let matrix = build_compatibility_matrix();
        for result in &matrix.results {
            assert!(
                !result.test_name.is_empty(),
                "Every CompatTestResult must have a non-empty test_name"
            );
        }
    }

    /// Verifies the JSON export can be round-tripped through serde_json.
    #[test]
    fn test_json_export_serializable() {
        let matrix = build_compatibility_matrix();
        let json_val = matrix_to_json(&matrix);

        // Convert to string and back
        let json_str = serde_json::to_string(&json_val).expect("JSON value must serialize");
        let reparsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("JSON string must deserialize");

        assert_eq!(
            json_val, reparsed,
            "JSON value must survive a serialization round-trip"
        );
    }

    /// Verifies the K8sVersion struct is serializable (required for JSON output).
    #[test]
    fn test_k8s_version_serde() {
        let version = K8sVersion::new(1, 28);
        let json = serde_json::to_string(&version).expect("K8sVersion must serialize to JSON");
        let deserialized: K8sVersion =
            serde_json::from_str(&json).expect("K8sVersion must deserialize from JSON");
        assert_eq!(version, deserialized);
    }

    /// Verifies the CompatTestResult struct is serializable.
    #[test]
    fn test_compat_result_serde() {
        let result = CompatTestResult {
            version: K8sVersion::new(1, 30),
            test_name: "k8s_1_30_crd_install_compat".to_string(),
            passed: true,
            notes: Some("Stable since 1.16".to_string()),
        };
        let json = serde_json::to_string(&result).expect("CompatTestResult must serialize");
        let deserialized: CompatTestResult =
            serde_json::from_str(&json).expect("CompatTestResult must deserialize");
        assert_eq!(result.test_name, deserialized.test_name);
        assert_eq!(result.passed, deserialized.passed);
    }

    /// Check that the ASCII table contains a separator line (--+--).
    #[test]
    fn test_matrix_table_has_separator_line() {
        let matrix = build_compatibility_matrix();
        let table = print_matrix_table(&matrix);
        assert!(
            table.contains("-+-") || table.contains("--+"),
            "Table must contain a column separator line"
        );
    }

    /// Exercises print_matrix_table and verifies it produces non-empty output.
    #[test]
    fn test_matrix_table_non_empty() {
        let matrix = build_compatibility_matrix();
        let table = print_matrix_table(&matrix);
        assert!(!table.is_empty(), "print_matrix_table must return non-empty output");

        // Print to stdout when running with --nocapture so developers can
        // inspect the table visually.
        println!("\n{table}");
    }

    /// Check that the JSON summary has the right total count.
    #[test]
    fn test_json_summary_total() {
        let matrix = build_compatibility_matrix();
        let json = matrix_to_json(&matrix);
        let expected_total = (4 * CompatibilityFeature::all().len()) as u64;
        let actual_total = json["summary"]["total"]
            .as_u64()
            .expect("summary.total must be a number");
        assert_eq!(actual_total, expected_total);
    }

    /// Verifies that every CompatibilityFeature variant has a non-empty label.
    #[test]
    fn test_all_features_have_labels() {
        for feature in CompatibilityFeature::all() {
            assert!(
                !feature.label().is_empty(),
                "CompatibilityFeature::{:?} must have a non-empty label",
                feature
            );
        }
    }

    /// Checks that each version produces the correct number of feature results.
    #[test]
    fn test_per_version_result_count() {
        let feature_count = CompatibilityFeature::all().len();
        for version in supported_k8s_versions() {
            let results = check_api_version_compatibility(&version);
            assert_eq!(
                results.len(),
                feature_count,
                "Version {} must produce exactly {} results",
                version,
                feature_count
            );
        }
    }
}
