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
//! Error types for quorum analysis

use thiserror::Error;

#[derive(Error, Debug)]
pub enum QuorumAnalysisError {
    /// HTTP request to Stellar Core failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Failed to parse SCP state response
    #[error("Failed to parse SCP state: {0}")]
    ParseError(String),

    /// Quorum graph is invalid (e.g., no intersection)
    #[error("Invalid quorum topology: {0}")]
    InvalidTopology(String),

    /// Analysis timeout exceeded
    #[error("Analysis timeout exceeded")]
    Timeout,

    /// Kubernetes API error when updating status
    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Kafka producer error
    #[error("Kafka error: {0}")]
    KafkaError(String),
}

pub type Result<T> = std::result::Result<T, QuorumAnalysisError>;
