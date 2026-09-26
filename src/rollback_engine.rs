// Copyright 2026 Stellar-K8s Contributors
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
//! Automated rollback decision engine driven by SLO burn rates (#1496).
//!
//! Rollbacks fire on multi-window error-budget burn (short + long windows
//! must both breach) rather than fixed health-check thresholds. Burn rates
//! are computed from existing SLI series (see [`crate::error_budget`] and
//! [`crate::composite_slo`] recording rules) — no new telemetry pipeline.
//!
//! The rollback action is idempotent and safely retryable, the decision
//! rationale is attached to the deployment record, and post-rollback
//! analysis is captured for review.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Service criticality tier; each tier has its own burn thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
    Critical,
    High,
    Standard,
}

/// Burn-rate thresholds for one tier. Both windows must breach for a
/// rollback (multi-window evaluation suppresses false positives).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurnThresholds {
    /// Fast-burn threshold for the short window (e.g. 1h).
    pub fast_burn: f64,
    /// Slow-burn threshold for the long window (e.g. 6h).
    pub slow_burn: f64,
    /// Minimum error-budget consumption (0..1) before rollback is eligible.
    pub min_budget_consumed: f64,
}

impl BurnThresholds {
    pub fn for_tier(tier: ServiceTier) -> Self {
        match tier {
            // Critical services roll back aggressively; standard services
            // tolerate more burn to keep the false-rollback rate < 1%.
            ServiceTier::Critical => Self { fast_burn: 6.0, slow_burn: 2.0, min_budget_consumed: 0.05 },
            ServiceTier::High => Self { fast_burn: 10.0, slow_burn: 3.0, min_budget_consumed: 0.10 },
            ServiceTier::Standard => Self { fast_burn: 14.0, slow_burn: 4.0, min_budget_consumed: 0.15 },
        }
    }
}

/// One burn-rate observation over two windows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurnObservation {
    pub deployment_id: String,
    pub tier: ServiceTier,
    /// Burn rate over the short window (e.g. 1h).
    pub short_burn: f64,
    /// Burn rate over the long window (e.g. 6h).
    pub long_burn: f64,
    /// Fraction of error budget consumed (0..1).
    pub budget_consumed: f64,
    pub short_window_secs: u64,
    pub long_window_secs: u64,
    pub observed_at: DateTime<Utc>,
}

impl BurnObservation {
    pub fn new(
        deployment_id: &str,
        tier: ServiceTier,
        short_burn: f64,
        long_burn: f64,
        budget_consumed: f64,
    ) -> Self {
        Self {
            deployment_id: deployment_id.to_string(),
            tier,
            short_burn,
            long_burn,
            budget_consumed,
            short_window_secs: 3600,
            long_window_secs: 21600,
            observed_at: Utc::now(),
        }
    }
}

/// Engine decision for one deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackVerdict {
    Rollback,
    Hold,
}

/// Decision with rationale attached to the deployment record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackDecision {
    pub deployment_id: String,
    pub tier: ServiceTier,
    pub verdict: RollbackVerdict,
    /// Human/machine-readable rationale (burn values, thresholds, windows).
    pub rationale: String,
    pub short_burn: f64,
    pub long_burn: f64,
    pub budget_consumed: f64,
    pub evaluated_at: DateTime<Utc>,
}

impl RollbackDecision {
    /// Serialize the rationale block stored on the deployment record.
    pub fn record_annotation(&self) -> HashMap<String, String> {
        HashMap::from([
            ("stellar.org/rollback-verdict".to_string(), format!("{:?}", self.verdict)),
            ("stellar.org/rollback-rationale".to_string(), self.rationale.clone()),
            ("stellar.org/rollback-evaluated-at".to_string(), self.evaluated_at.to_rfc3339()),
        ])
    }
}

/// Post-rollback analysis captured for review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRollbackAnalysis {
    pub deployment_id: String,
    pub rolled_back_at: DateTime<Utc>,
    pub trigger: RollbackDecision,
    pub outcome: String,
    pub reviewer: Option<String>,
}

/// Side-effect hook the engine drives. Must be idempotent: repeated calls
/// with the same deployment ID return `Ok(false)` (already rolled back).
#[async_trait::async_trait]
pub trait RollbackAction: Send + Sync {
    async fn rollback(&mut self, deployment_id: &str, reason: &str) -> Result<bool, String>;
    async fn is_rolled_back(&self, deployment_id: &str) -> bool;
}

/// In-memory idempotent rollback handle used by default and in tests.
#[derive(Debug, Default)]
pub struct MemoryRollbackAction {
    rolled_back: HashSet<String>,
    pub attempts: u64,
}

#[async_trait::async_trait]
impl RollbackAction for MemoryRollbackAction {
    async fn rollback(&mut self, deployment_id: &str, _reason: &str) -> Result<bool, String> {
        self.attempts += 1;
        if self.rolled_back.contains(deployment_id) {
            return Ok(false); // idempotent no-op
        }
        self.rolled_back.insert(deployment_id.to_string());
        Ok(true)
    }

    async fn is_rolled_back(&self, deployment_id: &str) -> bool {
        self.rolled_back.contains(deployment_id)
    }
}

/// The decision engine. Thresholds are configurable per tier.
pub struct RollbackEngine {
    thresholds: HashMap<ServiceTier, BurnThresholds>,
    pub decisions: Vec<RollbackDecision>,
    pub analyses: Vec<PostRollbackAnalysis>,
}

impl RollbackEngine {
    pub fn new() -> Self {
        Self {
            thresholds: HashMap::from([
                (ServiceTier::Critical, BurnThresholds::for_tier(ServiceTier::Critical)),
                (ServiceTier::High, BurnThresholds::for_tier(ServiceTier::High)),
                (ServiceTier::Standard, BurnThresholds::for_tier(ServiceTier::Standard)),
            ]),
            decisions: Vec::new(),
            analyses: Vec::new(),
        }
    }

    pub fn with_threshold(mut self, tier: ServiceTier, t: BurnThresholds) -> Self {
        self.thresholds.insert(tier, t);
        self
    }

    /// Evaluate one observation. Rollback requires short AND long windows to
    /// breach plus minimum budget consumption (multi-window burn-rate gate).
    pub fn evaluate(&mut self, obs: &BurnObservation) -> RollbackDecision {
        let t = self.thresholds.get(&obs.tier).cloned().unwrap_or_else(|| BurnThresholds::for_tier(obs.tier));
        let breach = obs.short_burn >= t.fast_burn
            && obs.long_burn >= t.slow_burn
            && obs.budget_consumed >= t.min_budget_consumed;
        let verdict = if breach { RollbackVerdict::Rollback } else { RollbackVerdict::Hold };
        let rationale = format!(
            "tier={:?} short_burn={:.2} (thresh {:.2}/{ }s) long_burn={:.2} (thresh {:.2}/{ }s) budget_consumed={:.3} (min {:.3}) verdict={verdict:?}",
            obs.tier,
            obs.short_burn, t.fast_burn, obs.short_window_secs,
            obs.long_burn, t.slow_burn, obs.long_window_secs,
            obs.budget_consumed, t.min_budget_consumed,
        );
        let decision = RollbackDecision {
            deployment_id: obs.deployment_id.clone(),
            tier: obs.tier,
            verdict,
            rationale,
            short_burn: obs.short_burn,
            long_burn: obs.long_burn,
            budget_consumed: obs.budget_consumed,
            evaluated_at: Utc::now(),
        };
        if verdict == RollbackVerdict::Rollback {
            warn!(deployment = %obs.deployment_id, rationale = %decision.rationale, "burn-rate gate breached; rollback advised");
        } else {
            info!(deployment = %obs.deployment_id, "burn-rate gate holds; no rollback");
        }
        self.decisions.push(decision.clone());
        decision
    }

    /// Execute the rollback for a `Rollback` decision. Idempotent and safely
    /// retryable: already-rolled-back deployments are a no-op that still
    /// records analysis exactly once per call.
    pub async fn execute<A: RollbackAction>(
        &mut self,
        decision: &RollbackDecision,
        action: &mut A,
    ) -> Result<bool, String> {
        if decision.verdict != RollbackVerdict::Rollback {
            return Ok(false);
        }
        let did = action.rollback(&decision.deployment_id, &decision.rationale).await?;
        self.analyses.push(PostRollbackAnalysis {
            deployment_id: decision.deployment_id.clone(),
            rolled_back_at: Utc::now(),
            trigger: decision.clone(),
            outcome: if did { "rolled back".to_string() } else { "already rolled back (idempotent no-op)".to_string() },
            reviewer: None,
        });
        Ok(did)
    }

    pub fn decisions_for(&self, deployment_id: &str) -> Vec<&RollbackDecision> {
        self.decisions.iter().filter(|d| d.deployment_id == deployment_id).collect()
    }
}

impl Default for RollbackEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn critical_burn_on_both_windows_triggers_rollback() {
        let mut engine = RollbackEngine::new();
        let obs = BurnObservation::new("dep-bad", ServiceTier::Critical, 8.0, 3.0, 0.2);
        let d = engine.evaluate(&obs);
        assert_eq!(d.verdict, RollbackVerdict::Rollback);
        assert!(d.rationale.contains("short_burn=8.00"));
        let mut action = MemoryRollbackAction::default();
        assert!(engine.execute(&d, &mut action).await.unwrap());
        // Idempotent retry is a safe no-op.
        assert!(!engine.execute(&d, &mut action).await.unwrap());
        assert!(action.is_rolled_back("dep-bad").await);
        assert_eq!(engine.analyses.len(), 2);
    }

    #[tokio::test]
    async fn single_window_spike_holds() {
        let mut engine = RollbackEngine::new();
        // Fast short-window spike but quiet long window -> Hold (no false rollback).
        let d = engine.evaluate(&BurnObservation::new("dep-spike", ServiceTier::High, 25.0, 0.5, 0.2));
        assert_eq!(d.verdict, RollbackVerdict::Hold);
    }

    #[tokio::test]
    async fn low_budget_consumption_holds() {
        let mut engine = RollbackEngine::new();
        let d = engine.evaluate(&BurnObservation::new("dep-early", ServiceTier::Critical, 9.0, 4.0, 0.01));
        assert_eq!(d.verdict, RollbackVerdict::Hold);
    }

    #[test]
    fn rationale_attaches_to_deployment_record() {
        let mut engine = RollbackEngine::new();
        let d = engine.evaluate(&BurnObservation::new("dep-x", ServiceTier::Standard, 20.0, 5.0, 0.5));
        assert_eq!(d.verdict, RollbackVerdict::Rollback);
        let ann = d.record_annotation();
        assert!(ann.contains_key("stellar.org/rollback-rationale"));
        assert!(ann.contains_key("stellar.org/rollback-verdict"));
    }

    #[test]
    fn seeded_bad_deploys_roll_back_exactly_breachers() {
        let mut engine = RollbackEngine::new();
        // Four seeded scenarios at differing severities.
        let cases = vec![
            (ServiceTier::Critical, 8.0, 3.0, 0.30, RollbackVerdict::Rollback),
            (ServiceTier::High, 11.0, 3.5, 0.20, RollbackVerdict::Rollback),
            (ServiceTier::Standard, 5.0, 1.0, 0.05, RollbackVerdict::Hold),
            (ServiceTier::High, 30.0, 0.4, 0.50, RollbackVerdict::Hold),
        ];
        for (i, (tier, s, l, b, want)) in cases.into_iter().enumerate() {
            let d = engine.evaluate(&BurnObservation::new(&format!("seed-{i}"), tier, s, l, b));
            assert_eq!(d.verdict, want, "seed-{i} mismatch");
        }
    }
}
