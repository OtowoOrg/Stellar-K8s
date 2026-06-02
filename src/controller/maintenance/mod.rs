//! Maintenance Window controller for Horizon DB maintenance tasks.
//!
//! Handles scheduling and coordination of VACUUM FULL and REINDEX operations.

pub mod bloat;
pub mod controller;
pub mod coordinator;
pub mod node_drain;
pub mod query_profiler;

pub use bloat::BloatDetector;
pub use controller::MaintenanceController;
pub use coordinator::MaintenanceCoordinator;
pub use node_drain::NodeDrainOrchestrator;
pub use query_profiler::{IndexSuggestion, QueryProfiler, SlowQuery};
