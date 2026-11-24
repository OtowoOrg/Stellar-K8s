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
