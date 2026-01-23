//! Controller module for StellarNode reconciliation
//!
//! This module contains the main controller loop, reconciliation logic,
//! and resource management for Stellar nodes.

mod archive_health;
mod finalizers;
pub mod metrics;
mod health;
#[cfg(test)]
mod health_test;
pub mod mtls;
mod reconciler;
mod remediation;
mod resources;
mod vsl;

pub use archive_health::{calculate_backoff, check_history_archive_health, ArchiveHealthResult};
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use health::{check_node_health, HealthCheckResult};
pub use reconciler::{run_controller, ControllerState};
pub use remediation::{check_stale_node, can_remediate, RemediationLevel, StaleCheckResult};
