//! Controller module for StellarNode reconciliation
//! This module contains the main controller loop, reconciliation logic,
//! and resource management for Stellar nodes.

pub mod resource_meta;

mod archive_health;
pub mod captive_core;
pub mod conditions;
pub mod cross_cluster;
pub mod cve;
mod cve_reconciler;
#[cfg(test)]
mod cve_test;
pub mod dr;
#[cfg(test)]
mod dr_test;
mod finalizers;
mod health;
#[cfg(test)]
mod health_test;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod mtls;
pub mod peer_discovery;
#[cfg(test)]
mod peer_discovery_test;
mod reconciler;
#[cfg(test)]
mod reconciler_test;
mod remediation;
#[cfg(test)]
mod remediation_test;
mod resources;
mod vsl;

pub use archive_health::{calculate_backoff, check_history_archive_health, ArchiveHealthResult};
pub use cross_cluster::{check_peer_latency, ensure_cross_cluster_services, PeerLatencyStatus};
pub use cve_reconciler::reconcile_cve_patches;
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use health::{check_node_health, HealthCheckResult};
pub use peer_discovery::{
    get_peers_from_config_map, trigger_peer_config_reload, PeerDiscoveryConfig,
    PeerDiscoveryManager, PeerInfo,
};
pub use reconciler::{run_controller, ControllerState};
pub use remediation::{can_remediate, check_stale_node, RemediationLevel, StaleCheckResult};
