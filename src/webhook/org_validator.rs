//! Organizational Standards Validator
//!
//! Validates that all StellarNode resources meet organizational standards:
//! - `resources.limits` and `resources.requests` are always present and non-empty.
//! - Resource limits do not exceed per-node-type maximums.
//! - In **production mode** (`spec.network == Mainnet`), resource *requests*
//!   meet per-node-type minimums so that under-provisioned nodes are rejected
//!   before they reach the public network.
//! - Required labels (`project-id`, `owner`) are present.
//!
//! This runs as part of the built-in webhook validation pipeline, before any
//! WASM plugins, so it cannot be bypassed.

use crate::crd::{NodeType, StellarNetwork, StellarNode};

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

/// Minimum resource *requests* per node type, enforced only in production mode.
///
/// These floors prevent operators from scheduling under-provisioned nodes onto
/// the public network, where insufficient CPU/memory causes ledger lag,
/// dropped consensus messages, and degraded API latency.
struct MinRequests {
    cpu_millicores: u64,
    memory_mib: u64,
}

fn min_requests_for(node_type: &NodeType) -> MinRequests {
    match node_type {
        // A Mainnet validator must keep up with consensus and history.
        NodeType::Validator => MinRequests {
            cpu_millicores: 2_000, // 2 cores
            memory_mib: 4_096,     // 4 GiB
        },
        // Horizon serves API traffic and ingests the ledger.
        NodeType::Horizon => MinRequests {
            cpu_millicores: 2_000, // 2 cores
            memory_mib: 4_096,     // 4 GiB
        },
        // Soroban RPC executes smart contracts and is the most demanding.
        NodeType::SorobanRpc => MinRequests {
            cpu_millicores: 4_000, // 4 cores
            memory_mib: 8_192,     // 8 GiB
        },
    }
}

/// Whether the node targets a production network.
///
/// Production mode is defined as the public Stellar network (`Mainnet`).
/// Testnet, Futurenet, and custom networks are treated as non-production and
/// are exempt from the minimum-resource floor so developers can run cheaply.
fn is_production(node: &StellarNode) -> bool {
    matches!(node.spec.network, StellarNetwork::Mainnet)
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
    validate_minimum_resources(node, &mut errors);
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

/// In production mode, ensure resource *requests* meet per-node-type minimums.
///
/// Non-production nodes (Testnet/Futurenet/Custom) are exempt. Unparseable
/// quantities are skipped here — `validate_resource_presence` already rejects
/// empty/zero values, and the Kubernetes API server rejects malformed ones.
fn validate_minimum_resources(node: &StellarNode, errors: &mut Vec<OrgValidationError>) {
    if !is_production(node) {
        return;
    }

    let min = min_requests_for(&node.spec.node_type);
    let requests = &node.spec.resources.requests;

    if let Some(cpu_mc) = parse_cpu_millicores(&requests.cpu) {
        if cpu_mc < min.cpu_millicores {
            errors.push(OrgValidationError::new(
                "spec.resources.requests.cpu",
                format!(
                    "CPU request '{}' ({} millicores) is below the minimum {} millicores required for {:?} nodes in production (network: Mainnet).",
                    requests.cpu, cpu_mc, min.cpu_millicores, node.spec.node_type
                ),
                format!(
                    "Increase spec.resources.requests.cpu to at least '{}m' for production {:?} nodes.",
                    min.cpu_millicores, node.spec.node_type
                ),
            ));
        }
    }

    if let Some(mem_mib) = parse_memory_mib(&requests.memory) {
        if mem_mib < min.memory_mib {
            errors.push(OrgValidationError::new(
                "spec.resources.requests.memory",
                format!(
                    "Memory request '{}' ({} MiB) is below the minimum {} MiB required for {:?} nodes in production (network: Mainnet).",
                    requests.memory, mem_mib, min.memory_mib, node.spec.node_type
                ),
                format!(
                    "Increase spec.resources.requests.memory to at least '{}Mi' for production {:?} nodes.",
                    min.memory_mib, node.spec.node_type
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

    // -----------------------------------------------------------------------
    // Minimum-resource validation (production mode) — issue #678
    // -----------------------------------------------------------------------

    /// Mark a node as production (Mainnet).
    fn as_production(mut node: StellarNode) -> StellarNode {
        node.spec.network = StellarNetwork::Mainnet;
        node
    }

    #[test]
    fn production_cpu_below_min_rejected() {
        // Validator min is 2 cores; 500m is far below.
        let node = as_production(make_node(
            NodeType::Validator,
            "500m",
            "4Gi",
            "2",
            "4Gi",
            good_labels(),
        ));
        let errors = validate_org_standards(&node);
        assert!(
            errors
                .iter()
                .any(|e| e.field == "spec.resources.requests.cpu"),
            "expected a CPU request floor violation, got: {errors:?}"
        );
    }

    #[test]
    fn production_memory_below_min_rejected() {
        // Validator min is 4Gi; 1Gi is below.
        let node = as_production(make_node(
            NodeType::Validator,
            "2",
            "1Gi",
            "2",
            "4Gi",
            good_labels(),
        ));
        let errors = validate_org_standards(&node);
        assert!(
            errors
                .iter()
                .any(|e| e.field == "spec.resources.requests.memory"),
            "expected a memory request floor violation, got: {errors:?}"
        );
    }

    #[test]
    fn production_below_min_reports_both_cpu_and_memory() {
        let node = as_production(make_node(
            NodeType::Validator,
            "500m",
            "1Gi",
            "2",
            "4Gi",
            good_labels(),
        ));
        let errors = validate_org_standards(&node);
        assert!(errors
            .iter()
            .any(|e| e.field == "spec.resources.requests.cpu"));
        assert!(errors
            .iter()
            .any(|e| e.field == "spec.resources.requests.memory"));
    }

    #[test]
    fn production_meeting_min_passes() {
        // Exactly at the Validator floor: 2 cores / 4Gi.
        let node = as_production(make_node(
            NodeType::Validator,
            "2",
            "4Gi",
            "4",
            "8Gi",
            good_labels(),
        ));
        let errors = validate_org_standards(&node);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn production_min_boundary_is_inclusive() {
        // Requests expressed in alternate units that equal the floor exactly
        // (2000m == 2 cores, 4096Mi == 4Gi) must pass.
        let node = as_production(make_node(
            NodeType::Validator,
            "2000m",
            "4096Mi",
            "4",
            "8Gi",
            good_labels(),
        ));
        let errors = validate_org_standards(&node);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn testnet_below_min_is_allowed() {
        // Same under-provisioned spec as production tests, but on Testnet:
        // the floor is not enforced for non-production networks.
        let node = make_node(
            NodeType::Validator,
            "500m",
            "1Gi",
            "2",
            "4Gi",
            good_labels(),
        );
        assert_eq!(node.spec.network, StellarNetwork::Testnet);
        let errors = validate_org_standards(&node);
        assert!(
            errors.is_empty(),
            "non-production nodes must not be subject to the floor, got: {errors:?}"
        );
    }

    #[test]
    fn soroban_production_has_higher_floor() {
        // 2 cores / 4Gi satisfies the Validator floor but NOT Soroban's (4/8Gi).
        let node = as_production(make_node(
            NodeType::SorobanRpc,
            "2",
            "4Gi",
            "8",
            "16Gi",
            good_labels(),
        ));
        let errors = validate_org_standards(&node);
        assert!(errors
            .iter()
            .any(|e| e.field == "spec.resources.requests.cpu"));
        assert!(errors
            .iter()
            .any(|e| e.field == "spec.resources.requests.memory"));

        // Bumping to the Soroban floor clears both violations.
        let ok = as_production(make_node(
            NodeType::SorobanRpc,
            "4",
            "8Gi",
            "8",
            "16Gi",
            good_labels(),
        ));
        assert!(validate_org_standards(&ok).is_empty());
    }
}
