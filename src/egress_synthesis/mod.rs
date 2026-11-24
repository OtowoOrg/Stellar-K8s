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
//! Zero-trust egress policy synthesis from observed workload telemetry.
//!
//! The synthesizer observes the egress flows a workload really produces,
//! clusters the observed destinations into CIDR + port tuples and turns those
//! tuples into a candidate `NetworkPolicy`. The candidate is never applied
//! straight into a workload namespace: it is written to a shadow namespace
//! first, rendered as a readable YAML diff and only promoted once the
//! observe window has elapsed without exceeding the false-positive budget.
//!
//! The stages are deliberately separable so the operator can run the
//! synthesizer as a batch job:
//!
//! 1. [`flow`] ingests flow records (Hubble JSON lines or in-memory records).
//! 2. [`cluster`] clusters the observed destinations into CIDR + port tuples.
//! 3. [`candidate`] renders the tuples as a `NetworkPolicy` candidate.
//! 4. [`diff`] renders the difference against the applied policy as YAML.
//! 5. [`staging`] decides whether the candidate may be enforced.

pub mod candidate;
pub mod cluster;
pub mod controller;
pub mod diff;
pub mod flow;
pub mod staging;

pub use candidate::{CandidatePolicy, EgressRule};
pub use cluster::{cluster_destinations, DestinationCluster};
pub use controller::{apply_candidate, promote_candidate, shadow_namespace_for};
pub use diff::{render_policy_diff, PolicyDiff};
pub use flow::{FlowIngestor, FlowParseError, IngestStats, ObservedFlow};
pub use staging::{
    PromotionDecision, StagingMode, StagingState, MAX_FALSE_POSITIVE_RATE, OBSERVE_WINDOW_DAYS,
};

use chrono::{DateTime, Utc};

/// Default prefix length used when clustering observed destinations.
pub const DEFAULT_CLUSTER_PREFIX: u8 = 24;

/// Synthesizer that turns observed flows into egress policy candidates.
///
/// The synthesizer is intentionally cheap to clone-free share: it can be
/// driven by a batch controller that ingests a batch of flows, renders a
/// candidate and then asks for a promotion decision.
#[derive(Debug)]
pub struct EgressSynthesizer {
    ingestor: FlowIngestor,
    state: StagingState,
    prefix_len: u8,
}

impl EgressSynthesizer {
    /// Create a synthesizer that starts in observe-only mode.
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            ingestor: FlowIngestor::new(),
            state: StagingState::observe_only(now),
            prefix_len: DEFAULT_CLUSTER_PREFIX,
        }
    }

    /// Create a synthesizer with a custom clustering prefix length.
    pub fn with_prefix_len(now: DateTime<Utc>, prefix_len: u8) -> Self {
        let mut synthesizer = Self::new(now);
        synthesizer.prefix_len = prefix_len;
        synthesizer
    }

    /// Feed observed flows into the synthesizer.
    pub async fn observe(&self, flows: Vec<ObservedFlow>) -> IngestStats {
        self.ingestor.ingest(flows).await
    }

    /// Ingest raw flow log lines (Hubble JSON lines).
    pub async fn observe_lines(&self, lines: &[String]) -> IngestStats {
        self.ingestor.ingest_lines(lines).await
    }

    /// Cluster the observed flows into CIDR + port tuples.
    pub async fn destinations(&self) -> Vec<DestinationCluster> {
        cluster_destinations(&self.ingestor.flows().await, self.prefix_len)
    }

    /// Build the candidate policy for a workload.
    pub async fn candidate(&self, workload: &str, namespace: &str) -> CandidatePolicy {
        let clusters = self.destinations().await;
        CandidatePolicy::build(workload, namespace, &clusters, self.state.mode)
    }

    /// Current staging state.
    pub fn staging(&self) -> &StagingState {
        &self.state
    }

    /// Decide whether the candidate may be enforced at `now`.
    pub fn promotion(&self, now: DateTime<Utc>) -> PromotionDecision {
        self.state.promotion_decision(now)
    }

    /// Record that a flow was blocked by an enforced policy.
    ///
    /// Blocked flows are the false-positive signal: a flow the policy denies
    /// but the workload actually needed.
    pub fn record_blocked_flow(&mut self, blocked: u64) {
        self.state.record_blocked(blocked);
    }

    /// Record observed flows in the staging state, used for the
    /// false-positive rate.
    pub fn record_observed_flow(&mut self, observed: u64) {
        self.state.record_observed(observed);
    }
}
