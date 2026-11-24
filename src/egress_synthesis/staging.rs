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
//! Staged rollout of synthesized egress policies.
//!
//! A synthesized policy is never enforced on the first run. The candidate is
//! observed for a window of days first; only when the window has elapsed and
//! the observed false-positive rate stays inside the budget may the policy
//! be promoted into enforcement.

use chrono::{DateTime, Duration, Utc};

/// Number of days a candidate has to be observed before enforcement.
pub const OBSERVE_WINDOW_DAYS: i64 = 7;

/// Maximum share of flows that may be blocked by the candidate policy.
pub const MAX_FALSE_POSITIVE_RATE: f64 = 0.001;

/// Enforcement mode of a synthesized policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagingMode {
    /// The candidate is only rendered and reviewed.
    ObserveOnly,
    /// The candidate is applied as a deny-by-default egress policy.
    Enforce,
}

impl StagingMode {
    /// The mode as it is written to the policy labels.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe",
            Self::Enforce => "enforce",
        }
    }
}

/// State of the staged rollout of a synthesized policy.
#[derive(Debug, Clone)]
pub struct StagingState {
    /// Current mode.
    pub mode: StagingMode,
    /// When the candidate started to be observed.
    pub first_observed_at: DateTime<Utc>,
    /// When the candidate was promoted to enforcement.
    pub last_promoted_at: Option<DateTime<Utc>>,
    /// Flows that were blocked while the policy was enforced.
    pub blocked_flows: u64,
    /// Flows that were observed in total.
    pub observed_flows: u64,
}

/// Outcome of a promotion check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionDecision {
    /// The candidate stays in observe-only mode.
    RemainObserveOnly {
        /// Why the candidate was not promoted.
        reason: String,
    },
    /// The candidate may be enforced.
    PromoteToEnforce,
}

impl StagingState {
    /// Create a state that starts in observe-only mode.
    pub fn observe_only(now: DateTime<Utc>) -> Self {
        Self {
            mode: StagingMode::ObserveOnly,
            first_observed_at: now,
            last_promoted_at: None,
            blocked_flows: 0,
            observed_flows: 0,
        }
    }

    /// Whether the observe window has elapsed at `now`.
    pub fn observe_window_elapsed(&self, now: DateTime<Utc>) -> bool {
        self.observe_window_elapsed_days(now) >= OBSERVE_WINDOW_DAYS
    }

    /// Number of whole days the candidate has been observed at `now`.
    pub fn observe_window_elapsed_days(&self, now: DateTime<Utc>) -> i64 {
        let elapsed = now - self.first_observed_at;
        let days = elapsed.num_days();
        if elapsed < Duration::days(OBSERVE_WINDOW_DAYS) && elapsed >= Duration::zero() {
            return 0;
        }
        days
    }

    /// Share of observed flows that the candidate would have blocked.
    pub fn false_positive_rate(&self) -> f64 {
        if self.observed_flows == 0 {
            return 0.0;
        }
        self.blocked_flows as f64 / self.observed_flows as f64
    }

    /// Whether the false-positive budget is respected.
    pub fn within_false_positive_budget(&self) -> bool {
        self.false_positive_rate() < MAX_FALSE_POSITIVE_RATE
    }

    /// Decide whether the candidate may be enforced at `now`.
    pub fn promotion_decision(&self, now: DateTime<Utc>) -> PromotionDecision {
        if self.mode == StagingMode::Enforce {
            return PromotionDecision::RemainObserveOnly {
                reason: "policy is already enforced".to_string(),
            };
        }

        let days = self.observe_window_elapsed_days(now);
        if days < OBSERVE_WINDOW_DAYS {
            return PromotionDecision::RemainObserveOnly {
                reason: format!(
                    "observe window not elapsed: {days} of {OBSERVE_WINDOW_DAYS} days observed"
                ),
            };
        }

        let rate = self.false_positive_rate();
        if !self.within_false_positive_budget() {
            return PromotionDecision::RemainObserveOnly {
                reason: format!(
                    "false-positive rate {rate:.4} exceeds budget {MAX_FALSE_POSITIVE_RATE:.4}"
                ),
            };
        }

        PromotionDecision::PromoteToEnforce
    }

    /// Promote the state to enforcement.
    pub fn promote(&mut self, now: DateTime<Utc>) {
        self.mode = StagingMode::Enforce;
        self.last_promoted_at = Some(now);
    }

    /// Record flows that were blocked by the enforced policy.
    pub fn record_blocked(&mut self, blocked: u64) {
        self.blocked_flows += blocked;
    }

    /// Record observed flows.
    pub fn record_observed(&mut self, observed: u64) {
        self.observed_flows += observed;
    }
}
