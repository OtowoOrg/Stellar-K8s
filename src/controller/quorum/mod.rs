// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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
#[cfg(feature = "kafka")]
pub mod scp_kafka_stream;
#[cfg(feature = "kafka")]
pub mod topology_health_consumer;
pub mod types;
pub mod uptime;

pub use analyzer::{QuorumAnalysisResult, QuorumAnalyzer};
pub use error::QuorumAnalysisError;
pub use graph::{CriticalNodeAnalysis, OverlapAnalysis, QuorumGraph};
pub use latency::{ConsensusLatencyTracker, LatencyMeasurement, LatencyStats};
pub use optimizer::QuorumOptimizer;
pub use scp_client::ScpClient;
#[cfg(feature = "kafka")]
pub use scp_kafka_stream::{ScpKafkaConfig, ScpKafkaProducer, ScpMessage, ScpStreamingSidecar};
#[cfg(feature = "kafka")]
pub use topology_health_consumer::{
    TopologicalHealth, TopologyHealthConfig, TopologyHealthConsumer, ValidatorHealth,
};
pub use types::{BallotState, NominationState, QuorumSetInfo, ScpState};
pub use uptime::PeerUptimeTracker;
