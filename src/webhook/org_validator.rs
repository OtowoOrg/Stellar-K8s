//! Organizational Standards Validator
//!
//! Validates that all StellarNode resources meet organizational standards:
//! - `resources.limits` and `resources.requests` are always present and non-empty.
//! - Resource limits do not exceed per-node-type maximums.
//! - Required labels (`project-id`, `owner`) are present.
//!
//! This runs as part of the built-in webhook validation pipeline, before any
//! WASM plugins, so it cannot be bypassed.

use crate::crd::{NodeType, StellarNode};

/// A single validation failure with a clear, actionable message.
#[derive(Debug, Clone)]
pub struct OrgValidationError {
    pub field: String,
    pub message: String,
    pub hint: String,
}

impl OrgValidationError {
    fn new(field: impl Into<String>, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            hint: hint.into(),
        }
    }
}

/// Maximum resource limits per node type (enforced by policy).
struct MaxLimits {
    cpu_millicores: u64,
    memory_mib: u64,
}

fn max_limits_for(node_type: &NodeType) -> MaxLimits {
    match node_type {
        NodeType::Validator => MaxLimits {
            cpu_millicores: 8_000, // 8 cores
            memory_mib: 16_384,    // 16 GiB
        },
        NodeType::Horizon => MaxLimits {
            cpu_millicores: 8_000,
            memory_mib: 16_384,
        },
        NodeType::SorobanRpc => MaxLimits {
            cpu_millicores: 16_000, // 16 cores — Soroban is more compute-intensive
            memory_mib: 32_768,     // 32 GiB
        },
    }
}

/// Required labels that every StellarNode must carry.
const REQUIRED_LABELS: &[(&str, &str)] = &[
    (
        "project-id",
        "Add 'project-id: <your-project>' to metadata.labels for billing attribution.",
    ),
    (
        "owner",
        "Add 'owner: <team-or-user>' to metadata.labels to identify the responsible team.",
    ),
];

/// Run all organizational standard checks against a StellarNode.
/// Returns a list of errors; empty means the resource is compliant.
pub fn validate_org_standards(node: &StellarNode) -> Vec<OrgValidationError> {
    let mut errors = Vec::new();

    validate_resource_presence(node, &mut errors);
    validate_resource_limits(node, &mut errors);
    validate_required_labels(node, &mut errors);

    errors
}

/// Ensure resources.requests and resources.limits are non-empty / non-zero.
fn validate_resource_presence(node: &StellarNode, errors: &mut Vec<OrgValidationError>) {
    let r = &node.spec.resources;

    if r.requests.cpu.trim().is_empty() || r.requests.cpu == "0" {
        errors.push(OrgValidationError::new(
            "spec.resources.requests.cpu",
            "CPU request must be set to a non-zero value.",
            "Set spec.resources.requests.cpu to a value like '500m' or '1'.",
        ));
    }

    if r.requests.memory.trim().is_empty() || r.requests.memory == "0" {
        errors.push(OrgValidationError::new(
            "spec.resources.requests.memory",
            "Memory request must be set to a non-zero value.",
            "Set spec.resources.requests.memory to a value like '512Mi' or '1Gi'.",
        ));
    }

    if r.limits.cpu.trim().is_empty() || r.limits.cpu == "0" {
        errors.push(OrgValidationError::new(
            "spec.resources.limits.cpu",
            "CPU limit must be set to prevent noisy-neighbor issues.",
            "Set spec.resources.limits.cpu to a value like '2' or '4000m'.",
        ));
    }

    if r.limits.memory.trim().is_empty() || r.limits.memory == "0" {
        errors.push(OrgValidationError::new(
            "spec.resources.limits.memory",
            "Memory limit must be set to prevent noisy-neighbor issues.",
            "Set spec.resources.limits.memory to a value like '2Gi' or '4Gi'.",
        ));
    }
}

/// Ensure resource limits do not exceed per-node-type maximums.
fn validate_resource_limits(node: &StellarNode, errors: &mut Vec<OrgValidationError>) {
    let max = max_limits_for(&node.spec.node_type);
    let limits = &node.spec.resources.limits;

    if let Some(cpu_mc) = parse_cpu_millicores(&limits.cpu) {
        if cpu_mc > max.cpu_millicores {
            errors.push(OrgValidationError::new(
                "spec.resources.limits.cpu",
                format!(
                    "CPU limit '{}' ({} millicores) exceeds the maximum allowed {} millicores for {:?} nodes.",
                    limits.cpu, cpu_mc, max.cpu_millicores, node.spec.node_type
                ),
                format!(
                    "Reduce spec.resources.limits.cpu to at most '{}m' for {:?} nodes.",
                    max.cpu_millicores, node.spec.node_type
                ),
            ));
        }
    }

    if let Some(mem_mib) = parse_memory_mib(&limits.memory) {
        if mem_mib > max.memory_mib {
            errors.push(OrgValidationError::new(
                "spec.resources.limits.memory",
                format!(
                    "Memory limit '{}' ({} MiB) exceeds the maximum allowed {} MiB for {:?} nodes.",
                    limits.memory, mem_mib, max.memory_mib, node.spec.node_type
                ),
                format!(
                    "Reduce spec.resources.limits.memory to at most '{}Mi' for {:?} nodes.",
                    max.memory_mib, node.spec.node_type
                ),
            ));
        }
    }
}

/// Ensure required labels are present on the StellarNode.
fn validate_required_labels(node: &StellarNode, errors: &mut Vec<OrgValidationError>) {
    let labels = node.metadata.labels.as_ref();

    for (label_key, hint) in REQUIRED_LABELS {
        let present = labels
            .and_then(|l| l.get(*label_key))
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);

        if !present {
            errors.push(OrgValidationError::new(
                format!("metadata.labels.{}", label_key),
                format!("Required label '{}' is missing or empty.", label_key),
                hint.to_string(),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Resource quantity parsers
// ---------------------------------------------------------------------------

/// Parse a Kubernetes CPU quantity string into millicores.
/// Supports: "500m", "1", "2.5", "4000m"
fn parse_cpu_millicores(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.ends_with('m') {
        s[..s.len() - 1].parse::<u64>().ok()
    } else {
        // Whole cores — multiply by 1000
        s.parse::<f64>().ok().map(|v| (v * 1000.0) as u64)
    }
}

/// Parse a Kubernetes memory quantity string into MiB.
/// Supports: "512Mi", "1Gi", "2048M", "1073741824" (bytes)
fn parse_memory_mib(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.ends_with("Gi") {
        s[..s.len() - 2]
            .parse::<f64>()
            .ok()
            .map(|v| (v * 1024.0) as u64)
    } else if s.ends_with("Mi") {
        s[..s.len() - 2].parse::<u64>().ok()
    } else if s.ends_with("G") {
        s[..s.len() - 1]
            .parse::<f64>()
            .ok()
            .map(|v| (v * 953.674) as u64) // 1 GB ≈ 953.674 MiB
    } else if s.ends_with("M") {
        s[..s.len() - 1]
            .parse::<f64>()
            .ok()
            .map(|v| (v * 0.953674) as u64)
    } else {
        // Raw bytes
        s.parse::<u64>().ok().map(|b| b / (1024 * 1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::types::{ResourceRequirements, ResourceSpec, StellarNetwork};
    use crate::crd::{NodeType, StellarNode, StellarNodeSpec};

    fn make_node(
        node_type: NodeType,
        cpu_req: &str,
        mem_req: &str,
        cpu_lim: &str,
        mem_lim: &str,
        labels: Option<std::collections::BTreeMap<String, String>>,
    ) -> StellarNode {
        let mut node = StellarNode::new(
            "test",
            StellarNodeSpec {
                node_type,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                resources: ResourceRequirements {
                    requests: ResourceSpec {
                        cpu: cpu_req.to_string(),
                        memory: mem_req.to_string(),
                    },
                    limits: ResourceSpec {
                        cpu: cpu_lim.to_string(),
                        memory: mem_lim.to_string(),
                    },
                },
                ..Default::default()
            },
        );
        node.metadata.labels = labels;
        node
    }

    fn good_labels() -> Option<std::collections::BTreeMap<String, String>> {
        let mut m = std::collections::BTreeMap::new();
        m.insert("project-id".to_string(), "stellar-prod".to_string());
        m.insert("owner".to_string(), "platform-team".to_string());
        Some(m)
    }

    #[test]
    fn valid_node_passes() {
        let node = make_node(
            NodeType::Validator,
            "500m",
            "1Gi",
            "2",
            "4Gi",
            good_labels(),
        );
        let errors = validate_org_standards(&node);
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn missing_labels_rejected() {
        let node = make_node(NodeType::Validator, "500m", "1Gi", "2", "4Gi", None);
        let errors = validate_org_standards(&node);
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.field.contains("project-id")));
        assert!(errors.iter().any(|e| e.field.contains("owner")));
    }

    #[test]
    fn empty_cpu_limit_rejected() {
        let node = make_node(NodeType::Validator, "500m", "1Gi", "", "4Gi", good_labels());
        let errors = validate_org_standards(&node);
        assert!(errors.iter().any(|e| e.field.contains("limits.cpu")));
    }

    #[test]
    fn cpu_limit_exceeds_max_rejected() {
        // Validator max is 8000m (8 cores); 16 cores should fail.
        let node = make_node(
            NodeType::Validator,
            "500m",
            "1Gi",
            "16",
            "4Gi",
            good_labels(),
        );
        let errors = validate_org_standards(&node);
        assert!(errors.iter().any(|e| e.field.contains("limits.cpu")));
    }

    #[test]
    fn memory_limit_exceeds_max_rejected() {
        // Validator max is 16384 MiB; 32Gi should fail.
        let node = make_node(
            NodeType::Validator,
            "500m",
            "1Gi",
            "2",
            "32Gi",
            good_labels(),
        );
        let errors = validate_org_standards(&node);
        assert!(errors.iter().any(|e| e.field.contains("limits.memory")));
    }

    #[test]
    fn soroban_allows_higher_limits() {
        // SorobanRpc max is 16 cores / 32 GiB — 16 cores should pass.
        let node = make_node(
            NodeType::SorobanRpc,
            "1",
            "2Gi",
            "16",
            "32Gi",
            good_labels(),
        );
        let errors = validate_org_standards(&node);
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn parse_cpu_millicores_works() {
        assert_eq!(parse_cpu_millicores("500m"), Some(500));
        assert_eq!(parse_cpu_millicores("2"), Some(2000));
        assert_eq!(parse_cpu_millicores("0"), Some(0));
    }

    #[test]
    fn parse_memory_mib_works() {
        assert_eq!(parse_memory_mib("512Mi"), Some(512));
        assert_eq!(parse_memory_mib("1Gi"), Some(1024));
        assert_eq!(parse_memory_mib("2Gi"), Some(2048));
    }
}
