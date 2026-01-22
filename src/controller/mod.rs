//! Controller module for StellarNode reconciliation
//!
//! This module contains the main controller loop, reconciliation logic,
//! and resource management for Stellar nodes.

mod archive_health;
mod finalizers;
mod reconciler;
mod resources;

pub use archive_health::{check_history_archive_health, calculate_backoff, ArchiveHealthResult};
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use reconciler::{run_controller, ControllerState};
