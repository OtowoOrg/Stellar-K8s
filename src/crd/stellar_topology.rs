//! StellarTopology Custom Resource Definition
//!
//! `StellarTopology` is the entry point of the Advanced Network Topology
//! Management epic (#869). It declares a validator network's peer relationships
//! and quorum expectations, and the operator continuously evaluates quorum
//! health, detects network partitions, and surfaces peer-optimization
//! recommendations.
//!
//! The graph analysis and partition/simulation logic live in
//! [`crate::controller::topology`] and are pure so they can be unit tested
//! without a live network. SCP message streaming, the visualization dashboard,
//! and historical querying build on this foundation (see the epic).

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

/// Spec for a `StellarTopology` resource.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarTopology",
    namespaced,
    status = "StellarTopologyStatus",
    shortname = "stopo",
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Quorum%","type":"number","jsonPath":".status.quorumHealthPct"}"#,
    printcolumn = r#"{"name":"Partitioned","type":"boolean","jsonPath":".status.partitionDetected"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarTopologySpec {
    /// The validators that make up this network and their peer relationships.
    pub validators: Vec<TopologyValidator>,

    /// Minimum fraction of validators (percentage, 0–100) that must be online
    /// for the network to be considered healthy. Default: 66.0 (roughly the
    /// classic BFT two-thirds threshold).
    #[serde(default = "default_min_online_pct")]
    pub min_online_pct: f64,

    /// Time budget, in seconds, within which a partition should be detected
    /// and surfaced. Default: 30.
    #[serde(default = "default_detection_window")]
    pub partition_detection_window_seconds: u32,
}

fn default_min_online_pct() -> f64 {
    66.0
}

fn default_detection_window() -> u32 {
    30
}

/// A validator and the peers it is connected to.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopologyValidator {
    /// Stable identifier (e.g. the StellarNode name or validator public key).
    pub name: String,

    /// Names of the peers this validator is connected to. Connections are
    /// treated as undirected for reachability analysis.
    #[serde(default)]
    pub peers: Vec<String>,
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Status for a `StellarTopology` resource.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarTopologyStatus {
    /// High-level phase derived from quorum health and partition detection.
    #[serde(default)]
    pub phase: TopologyPhase,

    /// Human-readable message describing the current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// RFC 3339 timestamp of the most recent evaluation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluation: Option<String>,

    /// Total number of validators declared in the topology.
    #[serde(default)]
    pub total_validators: i32,

    /// Number of validators currently observed online.
    #[serde(default)]
    pub online_validators: i32,

    /// Fraction of validators online, as a percentage (0–100).
    #[serde(default)]
    pub quorum_health_pct: f64,

    /// Whether the online validators form more than one disconnected group.
    #[serde(default)]
    pub partition_detected: bool,

    /// Number of disconnected groups among online validators (1 == connected).
    #[serde(default)]
    pub partition_count: i32,

    /// Peer-optimization and remediation recommendations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<String>,

    /// Kubernetes-style conditions for detailed status tracking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<crate::crd::Condition>,
}

/// High-level phase of a `StellarTopology` evaluation.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum TopologyPhase {
    /// Resource created, not yet evaluated.
    #[default]
    Pending,
    /// Quorum healthy and the network is fully connected.
    Healthy,
    /// Connected, but online validators are below the quorum threshold.
    Degraded,
    /// Online validators are split into disconnected groups.
    Partitioned,
}

impl std::fmt::Display for TopologyPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopologyPhase::Pending => write!(f, "Pending"),
            TopologyPhase::Healthy => write!(f, "Healthy"),
            TopologyPhase::Degraded => write!(f, "Degraded"),
            TopologyPhase::Partitioned => write!(f, "Partitioned"),
        }
    }
}
