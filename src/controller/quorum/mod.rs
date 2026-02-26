//! Quorum analysis module for SCP (Stellar Consensus Protocol) health monitoring
//!
//! This module provides comprehensive quorum health analysis for Stellar validators,
//! including critical node detection, quorum overlap calculation, and consensus latency tracking.

pub mod analyzer;
pub mod error;
pub mod graph;
pub mod latency;
pub mod scp_client;
pub mod types;

pub use analyzer::{QuorumAnalysisResult, QuorumAnalyzer};
pub use error::QuorumAnalysisError;
pub use graph::{CriticalNodeAnalysis, OverlapAnalysis, QuorumGraph};
pub use latency::{ConsensusLatencyTracker, LatencyMeasurement, LatencyStats};
pub use scp_client::ScpClient;
pub use types::{BallotState, NominationState, QuorumSetInfo, ScpState};
