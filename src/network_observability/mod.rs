//! Advanced network observability with flow analysis.
//!
//! Captures and analyzes network flows, detects anomalies, and provides
//! deep insights into service communication patterns.

pub mod analyzer;
pub mod anomaly;
pub mod flow;
pub mod performance;
pub mod security;
pub mod topology;

pub use analyzer::FlowAnalyzer;
pub use anomaly::{AnomalyDetector, AnomalyType, NetworkAnomaly};
pub use flow::{FlowStats, FlowStore, NetworkFlow, Protocol};
pub use performance::PerformanceAnalyzer;
pub use security::SecurityMonitor;
pub use topology::{ServiceDependency, TopologyGraph};
