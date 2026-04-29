//! Quorum analysis module for SCP (Stellar Consensus Protocol) health monitoring
//!
//! This module provides comprehensive quorum health analysis for Stellar validators,
//! including critical node detection, quorum overlap calculation, and consensus latency tracking.

pub mod analyzer;
pub mod error;
pub mod graph;
pub mod latency;
pub mod optimizer;
pub mod scp_client;
pub mod scp_kafka_stream;
pub mod topology_health_consumer;
pub mod types;
pub mod uptime;

pub use analyzer::{QuorumAnalysisResult, QuorumAnalyzer};
pub use error::QuorumAnalysisError;
pub use graph::{CriticalNodeAnalysis, OverlapAnalysis, QuorumGraph};
pub use latency::{ConsensusLatencyTracker, LatencyMeasurement, LatencyStats};
pub use optimizer::QuorumOptimizer;
pub use scp_client::ScpClient;
pub use scp_kafka_stream::{ScpKafkaConfig, ScpKafkaProducer, ScpMessage, ScpStreamingSidecar};
pub use topology_health_consumer::{
    TopologicalHealth, TopologyHealthConfig, TopologyHealthConsumer, ValidatorHealth,
};
pub use types::{BallotState, NominationState, QuorumSetInfo, ScpState};
pub use uptime::PeerUptimeTracker;
