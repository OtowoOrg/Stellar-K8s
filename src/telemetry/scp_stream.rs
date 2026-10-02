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
//! SCP message type and high-level stream entry-point.
//!
//! Writers call [`ScpMessage::push`] (or use the [`ScpRingBuffer`] directly)
//! to enqueue messages without ever contending on a mutex.  A background
//! drain worker (see [`crate::telemetry::scp_drain`]) batches those messages
//! and forwards them to Kafka and/or Prometheus.

use serde::Serialize;

/// A single Stellar Consensus Protocol telemetry event.
#[derive(Debug, Clone, Serialize)]
pub struct ScpMessage {
    pub ledger_seq: u64,
    pub node_id: String,
    pub quorum_set_hash: [u8; 32],
    pub slot_index: u64,
    pub message_type: String,
    pub payload: Vec<u8>,
}

impl ScpMessage {
    /// Partition key used for Kafka routing: the quorum-set hash.
    pub fn partition_key(&self) -> Vec<u8> {
        self.quorum_set_hash.to_vec()
    }
}
