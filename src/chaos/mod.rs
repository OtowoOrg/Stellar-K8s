//! Chaos Engineering Framework
//!
//! Provides fault injection, experiment scheduling, and resilience testing.

pub mod analytics;
pub mod fault_injection;
pub mod scheduler;

pub use analytics::{ChaosAnalytics, ChaosMetrics, ChaosReport, CiCdIntegration};
pub use fault_injection::{FaultInjectionManager, FaultInjector, FaultResult, FaultMetrics};
pub use scheduler::{ChaosEngine, ExperimentScheduler, ExperimentExecutor};