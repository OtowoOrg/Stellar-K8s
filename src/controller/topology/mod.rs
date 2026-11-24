//! Node topology awareness and spread-constraint enforcement.
//!
//! * [`zone`] — zone discovery and anti-affinity strength resolution.
//! * [`rules`] — topology-spread rule generation.
//! * [`enforcer`] — mutating controller that injects spread constraints.

pub mod enforcer;
pub mod rules;
mod zone;

pub use zone::{
    count_sibling_nodes, fetch_zone_topology, resolve_anti_affinity_strength, ZoneTopology,
    ZONE_LABEL,
};
pub use enforcer::{
    build_statefulset_patch, discover_cluster_topology, enforce_namespace, enforce_on_statefulset,
    EnforcementResult,
};
pub use rules::{
    ClusterTopology, TopologyMode, TopologyRuleSet, TopologySpreadConstraint, WhenUnsatisfiable,
};
