//! Controller module for StellarNode reconciliation
//! This module contains the main controller loop, reconciliation logic,
//! and resource management for Stellar nodes.

mod archive_health;
pub mod conditions;
pub mod dr;
mod finalizers;
mod health;
#[cfg(test)]
mod health_test;
pub mod metrics;
pub mod mtls;
mod reconciler;
mod remediation;
mod resources;
mod vsl;

pub use archive_health::{calculate_backoff, check_history_archive_health, ArchiveHealthResult};
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use health::{check_node_health, HealthCheckResult};
pub use reconciler::{run_controller, ControllerState};
pub use remediation::{can_remediate, check_stale_node, RemediationLevel, StaleCheckResult};
