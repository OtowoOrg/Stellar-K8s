//! Controller module for StellarNode reconciliation
//! This module contains the main controller loop, reconciliation logic,
//! and resource management for Stellar nodes.

pub mod resource_meta;

mod archive_health;
pub mod conditions;
pub mod cross_cluster;
pub mod dr;
mod finalizers;
mod health;
#[cfg(test)]
mod health_test;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod mtls;
pub mod peer_discovery;
mod reconciler;
mod remediation;
mod resources;
mod vsl;

pub use archive_health::{calculate_backoff, check_history_archive_health, ArchiveHealthResult};
pub use cross_cluster::{check_peer_latency, ensure_cross_cluster_services, PeerLatencyStatus};
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use health::{check_node_health, HealthCheckResult};
pub use peer_discovery::{
    get_peers_from_config_map, trigger_peer_config_reload, PeerDiscoveryConfig,
    PeerDiscoveryManager, PeerInfo,
};
pub use reconciler::{run_controller, ControllerState};
pub use remediation::{can_remediate, check_stale_node, RemediationLevel, StaleCheckResult};
