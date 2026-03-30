//! Property-based and unit tests for OPA Gatekeeper policy manifests.
//!
//! Verifies that all ConstraintTemplate and Constraint YAML files under
//! `config/manifests/gatekeeper/` satisfy the correctness properties defined
//! in the design document, and that the installation guide exists with the
//! required content.
//!
//! Feature: opa-gatekeeper-policies

use proptest::prelude::*;
use std::fs;

// ---------------------------------------------------------------------------
// Task 6.1 — Helper functions
// ---------------------------------------------------------------------------

/// Reads a file and parses it as YAML. Panics with a clear message if the
/// file is missing or contains invalid YAML.
fn load_manifest(path: &str) -> serde_yaml::Value {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read manifest '{}': {}", path, e));
    serde_yaml::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse YAML in '{}': {}", path, e))
}

/// Reads all `*-template.yaml` files from `config/manifests/gatekeeper/`.
/// Returns a Vec of (path, parsed_value) pairs sorted by path for determinism.
fn load_template_files() -> Vec<(String, serde_yaml::Value)> {
    let dir = "config/manifests/gatekeeper";
    let mut results = Vec::new();
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("Failed to read directory '{}': {}", dir, e));
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with("-template.yaml") {
                let path_str = path.to_string_lossy().into_owned();
                let value = load_manifest(&path_str);
                results.push((path_str, value));
            }
        }
    }
    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Reads all `*-constraint.yaml` files from `config/manifests/gatekeeper/`.
/// Returns a Vec of (path, parsed_value) pairs sorted by path for determinism.
fn load_constraint_files() -> Vec<(String, serde_yaml::Value)> {
    let dir = "config/manifests/gatekeeper";
    let mut results = Vec::new();
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("Failed to read directory '{}': {}", dir, e));
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with("-constraint.yaml") {
                let path_str = path.to_string_lossy().into_owned();
                let value = load_manifest(&path_str);
                results.push((path_str, value));
            }
        }
    }
    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Extracts `spec.targets[0].rego` as a String from a parsed ConstraintTemplate
/// YAML value. Panics if the field is absent or not a string.
fn extract_rego_source(template: &serde_yaml::Value) -> String {
    template["spec"]["targets"]
        .as_sequence()
        .and_then(|targets| targets.first())
        .and_then(|t| t["rego"].as_str())
        .unwrap_or_else(|| panic!("spec.targets[0].rego not found in template"))
        .to_string()
}

// ---------------------------------------------------------------------------
// Task 6.2 — Property 1: ConstraintTemplates have kind set
//
// Feature: opa-gatekeeper-policies, Property 1: ConstraintTemplates have kind set
// Validates: Requirements 1.1, 2.1, 3.1
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_constraint_templates_have_kind_set(idx in any::<prop::sample::Index>()) {
        let templates = load_template_files();
        prop_assume!(!templates.is_empty());

        let (path, value) = idx.get(&templates);
        let kind = value["spec"]["crd"]["spec"]["names"]["kind"]
            .as_str()
            .unwrap_or("");

        prop_assert!(
            !kind.is_empty(),
            "Template '{}' must have a non-empty spec.crd.spec.names.kind",
            path
        );
    }
}

// ---------------------------------------------------------------------------
// Task 6.3 — Property 2: Constraints reference a valid ConstraintTemplate kind
//
// Feature: opa-gatekeeper-policies, Property 2: Constraints reference a valid ConstraintTemplate kind
// Validates: Requirements 1.1, 2.1, 3.1, 4.2, 4.3
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_constraints_reference_valid_template_kind(idx in any::<prop::sample::Index>()) {
        let constraints = load_constraint_files();
        let templates = load_template_files();
        prop_assume!(!constraints.is_empty());

        let (constraint_path, constraint_value) = idx.get(&constraints);
        let constraint_kind = constraint_value["kind"]
            .as_str()
            .unwrap_or_else(|| panic!("Constraint '{}' has no 'kind' field", constraint_path));

        let template_kinds: Vec<&str> = templates
            .iter()
            .filter_map(|(_, v)| v["spec"]["crd"]["spec"]["names"]["kind"].as_str())
            .collect();

        let match_count = template_kinds.iter().filter(|&&k| k == constraint_kind).count();

        prop_assert_eq!(
            match_count,
            1,
            "Constraint '{}' kind '{}' must match exactly one ConstraintTemplate kind (found {}). Known template kinds: {:?}",
            constraint_path,
            constraint_kind,
            match_count,
            template_kinds
        );
    }
}

// ---------------------------------------------------------------------------
// Task 6.4 — Property 3: ResourceLimits Rego source references cpu/memory fields and deny conditions
//
// Feature: opa-gatekeeper-policies, Property 3: ResourceLimits Rego source references cpu/memory fields and deny conditions
// Validates: Requirements 1.2, 1.3, 1.4, 1.6
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_resource_limits_rego_denies_violations(_dummy in any::<bool>()) {
        let template = load_manifest("config/manifests/gatekeeper/resource-limits-template.yaml");
        let rego = extract_rego_source(&template);

        prop_assert!(
            rego.contains("spec.resources.limits.cpu"),
            "ResourceLimits Rego must reference 'spec.resources.limits.cpu'"
        );
        prop_assert!(
            rego.contains("spec.resources.limits.memory"),
            "ResourceLimits Rego must reference 'spec.resources.limits.memory'"
        );
        prop_assert!(
            rego.contains("violation"),
            "ResourceLimits Rego must define a 'violation' rule"
        );
        prop_assert!(
            rego.contains("input.parameters.max_cpu"),
            "ResourceLimits Rego must reference 'input.parameters.max_cpu'"
        );
        prop_assert!(
            rego.contains("input.parameters.max_memory"),
            "ResourceLimits Rego must reference 'input.parameters.max_memory'"
        );
        prop_assert!(
            rego.contains("units.parse"),
            "ResourceLimits Rego must use 'units.parse' for CPU comparison"
        );
        prop_assert!(
            rego.contains("units.parse_bytes"),
            "ResourceLimits Rego must use 'units.parse_bytes' for memory comparison"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 6.5 — Property 4: ApprovedRegistries Rego source references spec.version and allowed_registries
//
// Feature: opa-gatekeeper-policies, Property 4: ApprovedRegistries Rego source references spec.version and allowed_registries
// Validates: Requirements 2.2, 2.3, 2.5
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_approved_registries_rego_denies_violations(_dummy in any::<bool>()) {
        let template = load_manifest("config/manifests/gatekeeper/approved-registries-template.yaml");
        let rego = extract_rego_source(&template);

        prop_assert!(
            rego.contains("spec.version"),
            "ApprovedRegistries Rego must reference 'spec.version'"
        );
        prop_assert!(
            rego.contains("allowed_registries"),
            "ApprovedRegistries Rego must reference 'allowed_registries'"
        );
        prop_assert!(
            rego.contains("startswith"),
            "ApprovedRegistries Rego must use 'startswith' for registry prefix matching"
        );
        prop_assert!(
            rego.contains("violation"),
            "ApprovedRegistries Rego must define a 'violation' rule"
        );
        prop_assert!(
            rego.contains("input.parameters.allowed_registries"),
            "ApprovedRegistries Rego must reference 'input.parameters.allowed_registries'"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 6.6 — Property 5: RequiredLabels Rego source references metadata.labels and required_labels
//
// Feature: opa-gatekeeper-policies, Property 5: RequiredLabels Rego source references metadata.labels and required_labels
// Validates: Requirements 3.2, 3.3, 3.5
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_required_labels_rego_denies_violations(_dummy in any::<bool>()) {
        let template = load_manifest("config/manifests/gatekeeper/required-labels-template.yaml");
        let rego = extract_rego_source(&template);

        prop_assert!(
            rego.contains("metadata.labels"),
            "RequiredLabels Rego must reference 'metadata.labels'"
        );
        prop_assert!(
            rego.contains("required_labels"),
            "RequiredLabels Rego must reference 'required_labels'"
        );
        prop_assert!(
            rego.contains("violation"),
            "RequiredLabels Rego must define a 'violation' rule"
        );
        prop_assert!(
            rego.contains("input.parameters.required_labels"),
            "RequiredLabels Rego must reference 'input.parameters.required_labels'"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 6.7 — Property 6: Operator exemption is present in all Constraint match specs
//
// Feature: opa-gatekeeper-policies, Property 6: Operator exemption is present in all Constraint match specs
// Validates: Requirements 5.1, 5.4
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_operator_exemption_in_all_constraints(idx in any::<prop::sample::Index>()) {
        let constraints = load_constraint_files();
        prop_assume!(!constraints.is_empty());

        let (path, value) = idx.get(&constraints);

        let excluded = value["spec"]["match"]["excludedNamespaces"]
            .as_sequence()
            .unwrap_or_else(|| panic!(
                "Constraint '{}' must have spec.match.excludedNamespaces as a sequence",
                path
            ));

        prop_assert!(
            !excluded.is_empty(),
            "Constraint '{}' spec.match.excludedNamespaces must be non-empty",
            path
        );

        let has_stellar = excluded
            .iter()
            .any(|ns| ns.as_str().unwrap_or("") == "stellar");

        prop_assert!(
            has_stellar,
            "Constraint '{}' spec.match.excludedNamespaces must contain 'stellar', found: {:?}",
            path,
            excluded
        );
    }
}

// ---------------------------------------------------------------------------
// Task 6.8 — Unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_all_manifest_files_exist() {
    let files = [
        "config/manifests/gatekeeper/resource-limits-template.yaml",
        "config/manifests/gatekeeper/resource-limits-constraint.yaml",
        "config/manifests/gatekeeper/approved-registries-template.yaml",
        "config/manifests/gatekeeper/approved-registries-constraint.yaml",
        "config/manifests/gatekeeper/required-labels-template.yaml",
        "config/manifests/gatekeeper/required-labels-constraint.yaml",
    ];
    for path in &files {
        assert!(
            std::path::Path::new(path).exists(),
            "Expected manifest file to exist: {}",
            path
        );
    }
}

#[test]
fn test_gatekeeper_config_exists_with_system_exclusions() {
    let path = "config/manifests/gatekeeper/gatekeeper-config.yaml";
    assert!(
        std::path::Path::new(path).exists(),
        "gatekeeper-config.yaml must exist at {}",
        path
    );

    let value = load_manifest(path);

    let match_entries = value["spec"]["match"]
        .as_sequence()
        .expect("spec.match must be a sequence in gatekeeper-config.yaml");

    let all_excluded: Vec<&str> = match_entries
        .iter()
        .filter_map(|entry| entry["excludedNamespaces"].as_sequence())
        .flatten()
        .filter_map(|ns| ns.as_str())
        .collect();

    assert!(
        all_excluded.contains(&"kube-system"),
        "gatekeeper-config.yaml must exclude 'kube-system', found: {:?}",
        all_excluded
    );
    assert!(
        all_excluded.contains(&"gatekeeper-system"),
        "gatekeeper-config.yaml must exclude 'gatekeeper-system', found: {:?}",
        all_excluded
    );
}

#[test]
fn test_all_manifest_yaml_files_parse_without_error() {
    let dir = "config/manifests/gatekeeper";
    let mut parsed_count = 0;
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("Failed to read directory '{}': {}", dir, e));

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            let path_str = path.to_string_lossy().into_owned();
            let content = fs::read_to_string(&path_str)
                .unwrap_or_else(|e| panic!("Failed to read '{}': {}", path_str, e));
            let result: Result<serde_yaml::Value, _> = serde_yaml::from_str(&content);
            assert!(
                result.is_ok(),
                "YAML file '{}' must parse without error: {:?}",
                path_str,
                result.err()
            );
            parsed_count += 1;
        }
    }

    assert!(
        parsed_count > 0,
        "Expected at least one .yaml file in {}",
        dir
    );
}

#[test]
fn test_guide_file_exists() {
    assert!(
        std::path::Path::new("docs/gatekeeper-policies.md").exists(),
        "docs/gatekeeper-policies.md must exist"
    );
}

#[test]
fn test_guide_contains_gatekeeper_version_prerequisite() {
    let content = fs::read_to_string("docs/gatekeeper-policies.md")
        .expect("docs/gatekeeper-policies.md must exist");
    assert!(
        content.contains("3.13"),
        "Guide must mention Gatekeeper version '3.13'"
    );
}

#[test]
fn test_guide_contains_kubectl_apply_commands() {
    let content = fs::read_to_string("docs/gatekeeper-policies.md")
        .expect("docs/gatekeeper-policies.md must exist");
    assert!(
        content.contains("config/manifests/gatekeeper/"),
        "Guide must contain kubectl apply commands referencing 'config/manifests/gatekeeper/'"
    );
}

#[test]
fn test_guide_contains_stellar_operator_service_account() {
    let content = fs::read_to_string("docs/gatekeeper-policies.md")
        .expect("docs/gatekeeper-policies.md must exist");
    assert!(
        content.contains("stellar-operator"),
        "Guide must reference the 'stellar-operator' service account"
    );
}

#[test]
fn test_guide_contains_audit_mode_references() {
    let content = fs::read_to_string("docs/gatekeeper-policies.md")
        .expect("docs/gatekeeper-policies.md must exist");
    assert!(
        content.contains("kubectl get constraint"),
        "Guide must contain 'kubectl get constraint'"
    );
    assert!(
        content.contains("audit"),
        "Guide must contain 'audit' mode references"
    );
}
