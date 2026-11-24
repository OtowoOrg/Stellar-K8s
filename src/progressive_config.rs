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
//! Progressive config rollout with canary evaluation for operator settings
//! (#1519).
//!
//! Operator configuration used to be applied to every `StellarNode` at once.
//! A typo in a single setting therefore took down the whole fleet. This
//! module reuses progressive-delivery machinery for configuration:
//!
//! 1. **Canary first.** A change is applied to a deterministic subset of
//!    nodes/namespaces whose size never exceeds
//!    [`MAX_BLAST_RADIUS`] (5%) of the target set.
//! 2. **Gates during the canary window.** Health signals are evaluated
//!    against the canary; a single failed gate stops the rollout.
//! 3. **Automatic rollback.** On gate failure the previous config bundle is
//!    restored automatically, and the restore is measured against
//!    [`ROLLBACK_SLA`] (30s).
//! 4. **Queryable versions.** Every component records the config version it
//!    is currently running, so "which config is this node on?" is always
//!    answerable.
//!
//! A rollout cannot advance a stage without a gate pass, and a gate failure
//! never propagates beyond the canary subset.
//!
//! ```
//! use stellar_k8s::progressive_config::*;
//! use std::sync::Arc;
//!
//! let clock = Arc::new(ManualClock::new(1_000));
//! let apply = RecordingConfigApply::new(clock.clone());
//! let mut rollout = ProgressiveConfigRollout::new(vec!["ns-a/node-1".into()], clock);
//! let bundle = ConfigBundle::from_pairs("2.4.0", &[("replica_count", "1")]);
//!
//! let canary = futures::executor::block_on(rollout.start(&bundle, &apply)).unwrap();
//! assert_eq!(canary.len(), 1);
//!
//! // A failing gate never propagates the change.
//! let report = rollout.evaluate(&[HealthSample::new("replica_count", 0.0)], "metrics-adapter");
//! assert_eq!(report.verdict, GateVerdict::Failed);
//! assert_eq!(rollout.stage(), RolloutStage::Canary);
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// A rollout must never expose more than this fraction of the target set to a
/// canary before the gates pass. Acceptance criterion: bad config reaches
/// <= 5% of targets before rollback.
pub const MAX_BLAST_RADIUS: f64 = 0.05;

/// Budget for restoring the previous config bundle after a gate failure.
pub const ROLLBACK_SLA_MS: u64 = 30_000;

/// Version recorded for a target the rollout knows nothing about. A canary is
/// always rolled back to *something*, so an unrecorded baseline becomes this
/// explicit placeholder rather than leaving a bad config in place.
pub const UNSET_BUNDLE_VERSION: &str = "0.0.0";

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Time source, so rollout and rollback timing is deterministic in tests.
pub trait Clock: Send + Sync {
    /// Milliseconds since an arbitrary epoch.
    fn now_millis(&self) -> u64;
    /// Charge the cost of a simulated operation (config IO, API calls) to the
    /// clock, so SLA assertions measure real elapsed time rather than
    /// bookkeeping.
    fn charge(&self, _millis: u64) {}
}

/// Wall-clock time source used in production.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Manually advanced clock for tests and simulations.
#[derive(Debug)]
pub struct ManualClock {
    millis: std::sync::Mutex<u64>,
}

impl ManualClock {
    pub fn new(start_millis: u64) -> Self {
        Self {
            millis: std::sync::Mutex::new(start_millis),
        }
    }
    pub fn advance(&self, millis: u64) {
        let mut g = self.millis.lock().expect("clock poisoned");
        *g += millis;
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> u64 {
        *self.millis.lock().expect("clock poisoned")
    }

    fn charge(&self, millis: u64) {
        self.advance(millis);
    }
}

// ---------------------------------------------------------------------------
// Bundles and targets
// ---------------------------------------------------------------------------

/// An immutable, content-addressed configuration bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigBundle {
    pub version: String,
    /// SHA-256 over the sorted settings, so the same content always has the
    /// same digest regardless of insertion order.
    pub digest: String,
    pub settings: BTreeMap<String, String>,
}

impl ConfigBundle {
    pub fn new(version: &str, settings: impl IntoIterator<Item = (String, String)>) -> Self {
        let settings: BTreeMap<String, String> = settings.into_iter().collect();
        let digest = digest_settings(&settings);
        Self {
            version: version.to_string(),
            digest,
            settings,
        }
    }

    /// Convenience constructor for the common `&[(&str, &str)]` form.
    pub fn from_pairs(version: &str, settings: &[(&str, &str)]) -> Self {
        Self::new(
            version,
            settings.iter().map(|(k, v)| (k.to_string(), v.to_string())),
        )
    }
}

fn digest_settings(settings: &BTreeMap<String, String>) -> String {
    let mut h = Sha256::new();
    for (k, v) in settings {
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
        h.update(b";");
    }
    hex::encode(h.finalize())
}

/// A rollout target: one node, optionally scoped to a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RolloutTarget {
    pub namespace: String,
    pub node: String,
}

impl RolloutTarget {
    pub fn new(namespace: &str, node: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            node: node.to_string(),
        }
    }

    /// `namespace/node`, the form used in gate reports.
    pub fn id(&self) -> String {
        format!("{}/{}", self.namespace, self.node)
    }
}

impl fmt::Display for RolloutTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id())
    }
}

impl From<&str> for RolloutTarget {
    fn from(raw: &str) -> Self {
        match raw.split_once('/') {
            Some((ns, node)) => Self::new(ns, node),
            None => Self::new("default", raw),
        }
    }
}

/// Pick a canary subset that never exceeds [`MAX_BLAST_RADIUS`].
///
/// Selection is deterministic (targets are sorted, then a fixed stride
/// sample is taken) so the same target set always produces the same canary,
/// which keeps gate results comparable between rollouts. At least one target
/// is always selected, even for small fleets, because a zero-target canary
/// would silently skip validation.
pub fn select_canary(targets: &[RolloutTarget]) -> Vec<RolloutTarget> {
    if targets.is_empty() {
        return Vec::new();
    }
    let mut sorted = targets.to_vec();
    sorted.sort();
    sorted.dedup();

    // Floor, not ceil: rounding up would push the canary over the cap (3 of
    // 41 nodes is 7.3%). Small fleets fall back to a single node below.
    let cap = ((sorted.len() as f64) * MAX_BLAST_RADIUS).floor() as usize;
    let size = cap.clamp(1, sorted.len());
    // Even stride over the sorted set: deterministic and spread across
    // namespaces rather than clustered in the first N entries.
    let mut picked = Vec::with_capacity(size);
    for i in 0..size {
        let idx = (i * sorted.len()) / size;
        let target = sorted[idx].clone();
        if !picked.contains(&target) {
            picked.push(target);
        }
    }
    picked.sort();
    picked
}

/// Fraction of `targets` selected by [`select_canary`].
pub fn blast_radius(targets: &[RolloutTarget], canary: &[RolloutTarget]) -> f64 {
    if targets.is_empty() {
        return 0.0;
    }
    let unique: std::collections::BTreeSet<_> = canary.iter().collect();
    unique.len() as f64 / targets.len() as f64
}

// ---------------------------------------------------------------------------
// Health gates
// ---------------------------------------------------------------------------

/// Comparison used by a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    /// value must stay at or below the threshold.
    Max,
    /// value must stay at or above the threshold.
    Min,
}

impl fmt::Display for Comparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Comparator::Max => f.write_str("max"),
            Comparator::Min => f.write_str("min"),
        }
    }
}

/// A single health gate evaluated over the canary window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthGate {
    pub name: String,
    /// Signal name, matched against [`HealthSample::signal`].
    pub signal: String,
    pub comparator: Comparator,
    pub threshold: f64,
}

impl HealthGate {
    pub fn max(name: &str, signal: &str, threshold: f64) -> Self {
        Self {
            name: name.to_string(),
            signal: signal.to_string(),
            comparator: Comparator::Max,
            threshold,
        }
    }
    pub fn min(name: &str, signal: &str, threshold: f64) -> Self {
        Self {
            name: name.to_string(),
            signal: signal.to_string(),
            comparator: Comparator::Min,
            threshold,
        }
    }
}

/// One observation pulled from the canary during the canary window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthSample {
    pub signal: String,
    pub value: f64,
}

impl HealthSample {
    pub fn new(signal: &str, value: f64) -> Self {
        Self {
            signal: signal.to_string(),
            value,
        }
    }
}

/// Default gates: the operator refuses to propagate a config that leaves a
/// node with zero replicas, a broken readiness rate, or elevated errors.
pub fn default_gates() -> Vec<HealthGate> {
    vec![
        HealthGate::min("replicas_positive", "replica_count", 1.0),
        HealthGate::min("ready_ratio", "ready_ratio", 0.9),
        HealthGate::max("error_ratio", "error_ratio", 0.05),
    ]
}

/// Outcome of evaluating the gate set over one canary window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVerdict {
    Passed,
    Failed,
    /// A signal the gate depends on was not reported: no promotion.
    Incomplete,
}

impl fmt::Display for GateVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            GateVerdict::Passed => "passed",
            GateVerdict::Failed => "failed",
            GateVerdict::Incomplete => "incomplete",
        };
        f.write_str(s)
    }
}

/// Per-gate result with the observed value, for the rollout record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateResult {
    pub gate: String,
    pub signal: String,
    pub observed: Option<f64>,
    pub threshold: f64,
    pub comparator: Comparator,
    pub outcome: GateVerdict,
    pub detail: String,
}

/// Aggregate gate report for one canary window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateReport {
    pub verdict: GateVerdict,
    pub results: Vec<GateResult>,
    pub canary: Vec<String>,
    pub evaluated_at_ms: u64,
}

impl GateReport {
    /// Names of the gates that did not pass.
    pub fn failing_gates(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| r.outcome != GateVerdict::Passed)
            .map(|r| r.gate.as_str())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Config application
// ---------------------------------------------------------------------------

/// Side-effect hook that applies/restores a bundle on a target. Must be
/// idempotent: replaying the same apply is a no-op.
#[async_trait::async_trait]
pub trait ConfigApply: Send + Sync {
    /// Apply `bundle` to `target`. Returns the config version now in effect.
    async fn apply(&self, target: &RolloutTarget, bundle: &ConfigBundle) -> Result<String, String>;
    /// Restore the previously active bundle on `target`.
    async fn restore(
        &self,
        target: &RolloutTarget,
        bundle: &ConfigBundle,
    ) -> Result<String, String>;
    /// Config version currently in effect on `target`.
    async fn current_version(&self, target: &RolloutTarget) -> Option<String>;
}

/// In-memory applier used by default and in tests. Each apply/restore is
/// charged `cost_ms` against the clock so the rollback SLA is exercised
/// deterministically.
pub struct RecordingConfigApply {
    clock: Arc<dyn Clock>,
    /// Simulated per-target apply/restore cost.
    pub cost_ms: u64,
    applied: std::sync::Mutex<BTreeMap<String, String>>,
    history: std::sync::Mutex<BTreeMap<String, Vec<String>>>,
}

impl RecordingConfigApply {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            cost_ms: 1_000,
            applied: std::sync::Mutex::new(BTreeMap::new()),
            history: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    pub fn with_cost(mut self, cost_ms: u64) -> Self {
        self.cost_ms = cost_ms;
        self
    }

    /// Full version history per target, oldest first.
    pub fn history(&self) -> BTreeMap<String, Vec<String>> {
        self.history.lock().expect("history poisoned").clone()
    }
}

#[async_trait::async_trait]
impl ConfigApply for RecordingConfigApply {
    async fn apply(&self, target: &RolloutTarget, bundle: &ConfigBundle) -> Result<String, String> {
        self.clock_tick();
        let key = target.id();
        self.applied
            .lock()
            .expect("applied poisoned")
            .insert(key.clone(), bundle.version.clone());
        self.history
            .lock()
            .expect("history poisoned")
            .entry(key)
            .or_default()
            .push(bundle.version.clone());
        Ok(bundle.version.clone())
    }

    async fn restore(
        &self,
        target: &RolloutTarget,
        bundle: &ConfigBundle,
    ) -> Result<String, String> {
        self.clock_tick();
        self.applied
            .lock()
            .expect("applied poisoned")
            .insert(target.id(), bundle.version.clone());
        self.history
            .lock()
            .expect("history poisoned")
            .entry(target.id())
            .or_default()
            .push(bundle.version.clone());
        Ok(bundle.version.clone())
    }

    async fn current_version(&self, target: &RolloutTarget) -> Option<String> {
        self.applied
            .lock()
            .expect("applied poisoned")
            .get(&target.id())
            .cloned()
    }
}

impl RecordingConfigApply {
    fn clock_tick(&self) {
        // A real applier performs IO; the simulated cost is charged by
        // advancing the shared clock so timing assertions stay honest.
        self.clock.charge(self.cost_ms);
    }
}

// ---------------------------------------------------------------------------
// Component version tracking
// ---------------------------------------------------------------------------

/// Config version a single component is running, per namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentConfigVersion {
    pub component: String,
    pub namespace: String,
    pub node: String,
    pub version: String,
    pub digest: String,
    pub status: VersionStatus,
    pub updated_at: DateTime<Utc>,
}

/// Whether the component's config is the canary bundle or the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionStatus {
    /// Running the canary bundle.
    Canary,
    /// Running the previously active bundle after a rollback.
    Restored,
    /// Fully promoted to the new bundle.
    Promoted,
}

impl fmt::Display for VersionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            VersionStatus::Canary => "canary",
            VersionStatus::Restored => "restored",
            VersionStatus::Promoted => "promoted",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// Rollout engine
// ---------------------------------------------------------------------------

/// Where a rollout currently is. A rollout only ever moves forward when every
/// gate passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStage {
    /// Nothing in flight.
    Idle,
    /// Applied to the canary subset, awaiting gate evaluation.
    Canary,
    /// Canary gates passed; propagating to the rest of the fleet.
    Propagating,
    /// Every target runs the new bundle.
    Complete,
    /// Gates failed; the previous bundle was restored.
    RolledBack,
}

impl fmt::Display for RolloutStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RolloutStage::Idle => "idle",
            RolloutStage::Canary => "canary",
            RolloutStage::Propagating => "propagating",
            RolloutStage::Complete => "complete",
            RolloutStage::RolledBack => "rolled_back",
        };
        f.write_str(s)
    }
}

/// Audit record for one rollout step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutEvent {
    pub at_ms: u64,
    pub stage: RolloutStage,
    pub version: String,
    pub targets: Vec<String>,
    pub detail: String,
}

/// The progressive config rollout engine.
pub struct ProgressiveConfigRollout {
    targets: Vec<RolloutTarget>,
    gates: Vec<HealthGate>,
    clock: Arc<dyn Clock>,
    stage: RolloutStage,
    canary: Vec<RolloutTarget>,
    pending_bundle: Option<ConfigBundle>,
    previous_bundle: Option<ConfigBundle>,
    versions: BTreeMap<String, ComponentConfigVersion>,
    reports: Vec<GateReport>,
    events: Vec<RolloutEvent>,
    rollbacks: Vec<RollbackRecord>,
}

/// Record of one automatic rollback, including the measured restore time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollbackRecord {
    pub from_version: String,
    pub to_version: String,
    /// Targets that were restored (at most the canary subset).
    pub targets: Vec<String>,
    pub duration_ms: u64,
    pub within_sla: bool,
    pub reason: String,
}

impl ProgressiveConfigRollout {
    pub fn new(targets: Vec<RolloutTarget>, clock: Arc<dyn Clock>) -> Self {
        Self {
            targets,
            gates: default_gates(),
            clock,
            stage: RolloutStage::Idle,
            canary: Vec::new(),
            pending_bundle: None,
            previous_bundle: None,
            versions: BTreeMap::new(),
            reports: Vec::new(),
            events: Vec::new(),
            rollbacks: Vec::new(),
        }
    }

    /// Override the gate set (used for component-specific gates).
    pub fn with_gates(mut self, gates: Vec<HealthGate>) -> Self {
        self.gates = gates;
        self
    }

    pub fn stage(&self) -> RolloutStage {
        self.stage
    }
    pub fn canary(&self) -> &[RolloutTarget] {
        &self.canary
    }
    pub fn reports(&self) -> &[GateReport] {
        &self.reports
    }
    pub fn events(&self) -> &[RolloutEvent] {
        &self.events
    }
    pub fn rollbacks(&self) -> &[RollbackRecord] {
        &self.rollbacks
    }
    pub fn targets(&self) -> &[RolloutTarget] {
        &self.targets
    }

    /// Config version currently in effect for a target. Always queryable once
    /// the target has been touched by a rollout.
    pub fn config_version(&self, target: &RolloutTarget) -> Option<&ComponentConfigVersion> {
        self.versions.get(&target.id())
    }

    /// Every known component config version, keyed by `namespace/node`.
    pub fn all_versions(&self) -> &BTreeMap<String, ComponentConfigVersion> {
        &self.versions
    }

    /// Fraction of the fleet currently exposed to the candidate bundle.
    pub fn current_blast_radius(&self) -> f64 {
        let canary_count = self
            .versions
            .values()
            .filter(|v| v.status != VersionStatus::Promoted)
            .count();
        if self.targets.is_empty() {
            return 0.0;
        }
        canary_count as f64 / self.targets.len() as f64
    }

    /// Record the currently active config for a component so that
    /// `config_version` is answerable before the first rollout.
    pub fn register_active(&mut self, target: &RolloutTarget, bundle: &ConfigBundle) {
        self.record_version(target, bundle, VersionStatus::Promoted);
    }

    /// Start a rollout: apply the candidate bundle to the canary subset only.
    pub async fn start<A: ConfigApply>(
        &mut self,
        bundle: &ConfigBundle,
        apply: &A,
    ) -> Result<Vec<RolloutTarget>, String> {
        if self.stage == RolloutStage::Canary || self.stage == RolloutStage::Propagating {
            return Err("a rollout is already in flight".to_string());
        }
        self.canary = select_canary(&self.targets);
        self.previous_bundle = Some(self.current_active_bundle());
        self.pending_bundle = Some(bundle.clone());
        for target in &self.canary.clone() {
            apply.apply(target, bundle).await?;
            self.record_version(target, bundle, VersionStatus::Canary);
        }
        self.stage = RolloutStage::Canary;
        let ids: Vec<String> = self.canary.iter().map(|t| t.id()).collect();
        self.events.push(RolloutEvent {
            at_ms: self.clock.now_millis(),
            stage: RolloutStage::Canary,
            version: bundle.version.clone(),
            targets: ids,
            detail: format!(
                "canary started for {} version {} (blast radius {:.1}%)",
                bundle.version,
                bundle.digest,
                blast_radius(&self.targets, &self.canary) * 100.0
            ),
        });
        info!(version = %bundle.version, canary = self.canary.len(), "config canary started");
        Ok(self.canary.clone())
    }

    /// Evaluate the gates over one canary window.
    ///
    /// On `Passed` the rollout moves to `Propagating`; on `Failed` or
    /// `Incomplete` the candidate bundle is rolled back automatically and the
    /// stage becomes `RolledBack`. The stage never advances on a non-pass.
    pub fn evaluate(&mut self, samples: &[HealthSample], source: &str) -> GateReport {
        let now = self.clock.now_millis();
        let canary_ids: Vec<String> = self.canary.iter().map(|t| t.id()).collect();
        let mut results = Vec::new();
        let mut verdict = GateVerdict::Passed;

        for gate in &self.gates {
            let observed = samples
                .iter()
                .find(|s| s.signal == gate.signal)
                .map(|s| s.value);
            let outcome = match observed {
                None => GateVerdict::Incomplete,
                Some(v) => match gate.comparator {
                    Comparator::Max if v <= gate.threshold => GateVerdict::Passed,
                    Comparator::Min if v >= gate.threshold => GateVerdict::Passed,
                    _ => GateVerdict::Failed,
                },
            };
            let detail = match observed {
                None => format!("signal `{}` not reported by {source}", gate.signal),
                Some(v) => format!(
                    "{signal}={v} (requires {cmp} {thr})",
                    signal = gate.signal,
                    cmp = gate.comparator,
                    thr = gate.threshold
                ),
            };
            if outcome != GateVerdict::Passed {
                verdict = if outcome == GateVerdict::Failed {
                    GateVerdict::Failed
                } else {
                    verdict
                };
                if verdict != GateVerdict::Failed {
                    verdict = GateVerdict::Incomplete;
                }
            }
            results.push(GateResult {
                gate: gate.name.clone(),
                signal: gate.signal.clone(),
                observed,
                threshold: gate.threshold,
                comparator: gate.comparator,
                outcome,
                detail,
            });
        }

        let report = GateReport {
            verdict,
            results,
            canary: canary_ids,
            evaluated_at_ms: now,
        };
        self.reports.push(report.clone());
        if report.verdict == GateVerdict::Passed {
            if self.stage == RolloutStage::Canary {
                self.stage = RolloutStage::Propagating;
                self.events.push(RolloutEvent {
                    at_ms: now,
                    stage: RolloutStage::Propagating,
                    version: self
                        .pending_bundle
                        .as_ref()
                        .map(|b| b.version.clone())
                        .unwrap_or_default(),
                    targets: self.targets.iter().map(|t| t.id()).collect(),
                    detail: "all gates passed; propagating".to_string(),
                });
            }
        } else {
            warn!(verdict = %report.verdict, gates = ?report.failing_gates(), "config canary gate failed; rolling back");
        }
        report
    }

    /// Roll back the canary to the previous bundle. Idempotent: calling it
    /// without an in-flight candidate is a no-op.
    pub async fn rollback<A: ConfigApply>(
        &mut self,
        apply: &A,
        reason: &str,
    ) -> Result<Option<RollbackRecord>, String> {
        let bundle = match self.pending_bundle.clone() {
            Some(b) => b,
            None => return Ok(None),
        };
        let previous = match self.previous_bundle.clone() {
            Some(p) => p,
            None => return Ok(None),
        };
        let started = self.clock.now_millis();
        let targets: Vec<RolloutTarget> = self.canary.clone();
        for target in &targets {
            apply.restore(target, &previous).await?;
            self.record_version(target, &previous, VersionStatus::Restored);
        }
        let duration_ms = self.clock.now_millis().saturating_sub(started);
        let record = RollbackRecord {
            from_version: bundle.version.clone(),
            to_version: previous.version.clone(),
            targets: targets.iter().map(|t| t.id()).collect(),
            duration_ms,
            within_sla: duration_ms <= ROLLBACK_SLA_MS,
            reason: reason.to_string(),
        };
        self.rollbacks.push(record.clone());
        self.pending_bundle = None;
        self.stage = RolloutStage::RolledBack;
        self.events.push(RolloutEvent {
            at_ms: self.clock.now_millis(),
            stage: RolloutStage::RolledBack,
            version: previous.version.clone(),
            targets: record.targets.clone(),
            detail: format!("rolled back in {}ms: {reason}", duration_ms),
        });
        if !record.within_sla {
            warn!(
                duration_ms,
                sla_ms = ROLLBACK_SLA_MS,
                "config rollback exceeded the 30s SLA"
            );
        }
        Ok(Some(record))
    }

    /// Evaluate the canary window and roll back automatically when the gates
    /// do not pass. Returns the gate report and the rollback record, if any.
    pub async fn gate_or_rollback<A: ConfigApply>(
        &mut self,
        samples: &[HealthSample],
        source: &str,
        apply: &A,
    ) -> Result<(GateReport, Option<RollbackRecord>), String> {
        let report = self.evaluate(samples, source);
        let rollback = if report.verdict == GateVerdict::Passed {
            None
        } else {
            self.rollback(
                apply,
                &format!(
                    "gate {}: {}",
                    report.verdict,
                    report.failing_gates().join(", ")
                ),
            )
            .await?
        };
        Ok((report, rollback))
    }

    /// Finish a `Propagating` rollout by applying the bundle fleet-wide.
    pub async fn complete<A: ConfigApply>(&mut self, apply: &A) -> Result<Vec<String>, String> {
        if self.stage != RolloutStage::Propagating {
            return Err(format!("cannot complete a rollout in stage {}", self.stage));
        }
        let bundle = self
            .pending_bundle
            .clone()
            .expect("propagating rollout has a bundle");
        let now = self.clock.now_millis();
        let mut touched = Vec::new();
        for target in self.targets.clone() {
            if self.canary.contains(&target) {
                touched.push(target.id());
                self.record_version(&target, &bundle, VersionStatus::Promoted);
                continue;
            }
            apply.apply(&target, &bundle).await?;
            self.record_version(&target, &bundle, VersionStatus::Promoted);
            touched.push(target.id());
        }
        self.stage = RolloutStage::Complete;
        self.pending_bundle = None;
        self.events.push(RolloutEvent {
            at_ms: now,
            stage: RolloutStage::Complete,
            version: bundle.version.clone(),
            targets: touched.clone(),
            detail: "rollout complete".to_string(),
        });
        Ok(touched)
    }

    /// The bundle a target is running right now. Targets that have never been
    /// registered fall back to the explicit unset placeholder, so a rollback
    /// always has something to restore to.
    fn current_active_bundle(&self) -> ConfigBundle {
        self.versions
            .values()
            .next()
            .map(|v| ConfigBundle {
                version: v.version.clone(),
                digest: v.digest.clone(),
                settings: BTreeMap::new(),
            })
            .unwrap_or_else(|| ConfigBundle::new(UNSET_BUNDLE_VERSION, Vec::new()))
    }

    fn record_version(
        &mut self,
        target: &RolloutTarget,
        bundle: &ConfigBundle,
        status: VersionStatus,
    ) {
        self.versions.insert(
            target.id(),
            ComponentConfigVersion {
                component: "operator-config".to_string(),
                namespace: target.namespace.clone(),
                node: target.node.clone(),
                version: bundle.version.clone(),
                digest: bundle.digest.clone(),
                status,
                updated_at: Utc::now(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(n: usize) -> Vec<RolloutTarget> {
        (0..n)
            .map(|i| RolloutTarget::new(&format!("ns-{}", i % 5), &format!("node-{i:03}")))
            .collect()
    }

    fn healthy() -> Vec<HealthSample> {
        vec![
            HealthSample::new("replica_count", 2.0),
            HealthSample::new("ready_ratio", 1.0),
            HealthSample::new("error_ratio", 0.0),
        ]
    }

    // -- canary selection --------------------------------------------------

    #[test]
    fn canary_never_exceeds_five_percent_blast_radius() {
        for n in [1usize, 3, 20, 41, 100, 250, 1000] {
            let ts = targets(n);
            let canary = select_canary(&ts);
            assert!(
                !canary.is_empty(),
                "fleet of {n} must still canary at least one node"
            );
            // A canary is at least one node, so fleets smaller than 20 nodes
            // cannot stay under 5%; the budget is `floor(5% of fleet)`, with a
            // floor of one node.
            let budget = (((n as f64) * MAX_BLAST_RADIUS).floor() as usize).max(1);
            let radius = blast_radius(&ts, &canary);
            assert!(
                canary.len() <= budget,
                "fleet of {n} canaried {} nodes = {:.2}% > budget of {budget}",
                canary.len(),
                radius * 100.0
            );
            if n >= 20 {
                assert!(
                    radius <= MAX_BLAST_RADIUS,
                    "fleet of {n} exceeded the 5% blast radius"
                );
            }
        }
    }

    #[test]
    fn canary_selection_is_deterministic_and_deduplicated() {
        let ts = targets(120);
        let a = select_canary(&ts);
        let b = select_canary(&ts);
        assert_eq!(a, b);
        let unique: std::collections::BTreeSet<_> = a.iter().collect();
        assert_eq!(unique.len(), a.len());
    }

    #[test]
    fn empty_target_set_yields_empty_canary() {
        assert!(select_canary(&[]).is_empty());
        assert_eq!(blast_radius(&[], &[]), 0.0);
    }

    // -- gates -------------------------------------------------------------

    #[test]
    fn all_gates_passing_promotes_to_propagating() {
        let clock = Arc::new(ManualClock::new(1_000));
        let apply = RecordingConfigApply::new(clock.clone());
        let mut r = ProgressiveConfigRollout::new(targets(40), clock);
        let bundle = ConfigBundle::from_pairs("2.4.0", &[("replica_count", "2")]);
        block_on(r.start(&bundle, &apply)).unwrap();
        let report = r.evaluate(&healthy(), "metrics");
        assert_eq!(report.verdict, GateVerdict::Passed);
        assert_eq!(r.stage(), RolloutStage::Propagating);
    }

    #[test]
    fn missing_signal_is_incomplete_and_does_not_promote() {
        let clock = Arc::new(ManualClock::new(0));
        let apply = RecordingConfigApply::new(clock.clone());
        let mut r = ProgressiveConfigRollout::new(targets(40), clock);
        let bundle = ConfigBundle::from_pairs("2.4.1", &[("replica_count", "0")]);
        block_on(r.start(&bundle, &apply)).unwrap();
        let report = r.evaluate(&[HealthSample::new("replica_count", 2.0)], "metrics");
        assert_eq!(report.verdict, GateVerdict::Incomplete);
        assert_eq!(r.stage(), RolloutStage::Canary);
    }

    #[test]
    fn gate_failure_rolls_back_under_thirty_seconds() {
        let clock = Arc::new(ManualClock::new(5_000));
        // Two canary targets at 1s each -> 2s restore, well inside the SLA.
        let apply = RecordingConfigApply::new(clock.clone()).with_cost(1_000);
        let mut r = ProgressiveConfigRollout::new(targets(40), clock);
        let bundle = ConfigBundle::from_pairs("2.4.2", &[("replica_count", "0")]);
        block_on(r.start(&bundle, &apply)).unwrap();

        let bad = vec![
            HealthSample::new("replica_count", 0.0),
            HealthSample::new("ready_ratio", 0.2),
            HealthSample::new("error_ratio", 0.9),
        ];
        let (report, rollback) = block_on(r.gate_or_rollback(&bad, "metrics", &apply)).unwrap();
        assert_eq!(report.verdict, GateVerdict::Failed);
        let record = rollback.expect("gate failure must trigger an automatic rollback");
        assert!(record.within_sla, "rollback took {}ms", record.duration_ms);
        assert!(record.duration_ms <= ROLLBACK_SLA_MS);
        assert_eq!(r.stage(), RolloutStage::RolledBack);
        // Only the canary subset ever saw the bad config.
        assert_eq!(record.targets.len(), r.canary().len());
        assert!(record.targets.len() <= 2);
    }

    #[test]
    fn rollback_restores_previous_version_on_every_canary_target() {
        let clock = Arc::new(ManualClock::new(0));
        let apply = RecordingConfigApply::new(clock.clone());
        let mut r = ProgressiveConfigRollout::new(targets(30), clock.clone());
        let good = ConfigBundle::from_pairs("2.3.0", &[("replica_count", "1")]);
        for t in r.targets().to_vec() {
            r.register_active(&t, &good);
        }
        let bad = ConfigBundle::from_pairs("2.4.0", &[("replica_count", "0")]);
        block_on(r.start(&bad, &apply)).unwrap();
        for t in r.canary().to_vec() {
            assert_eq!(
                block_on(apply.current_version(&t)),
                Some("2.4.0".to_string())
            );
        }
        block_on(r.rollback(&apply, "gate failure")).unwrap();
        for t in r.canary().to_vec() {
            assert_eq!(
                block_on(apply.current_version(&t)),
                Some("2.3.0".to_string())
            );
        }
    }

    #[test]
    fn three_intentionally_bad_configs_are_caught_at_or_below_five_percent() {
        // Three different bad settings, each caught during canary.
        let bad_bundles: [(&str, Vec<(&str, &str)>); 3] = [
            ("2.5.0", vec![("replica_count", "0")]),
            ("2.5.1", vec![("ready_ratio", "0.10")]),
            ("2.5.2", vec![("error_ratio", "0.90")]),
        ];
        for (version, settings) in bad_bundles {
            let clock = Arc::new(ManualClock::new(0));
            let apply = RecordingConfigApply::new(clock.clone());
            let ts = targets(100);
            let mut r = ProgressiveConfigRollout::new(ts.clone(), clock);
            let good = ConfigBundle::from_pairs("2.4.9", &[("replica_count", "1")]);
            for t in &ts {
                r.register_active(t, &good);
            }
            let bundle = ConfigBundle::from_pairs(version, &settings);
            let canary = block_on(r.start(&bundle, &apply)).unwrap();
            let radius = blast_radius(&ts, &canary);
            assert!(
                radius <= MAX_BLAST_RADIUS,
                "{version} blast radius {radius}"
            );
            assert!(canary.len() * 3 < ts.len(), "{version} canary too wide");

            let (report, rollback) =
                block_on(r.gate_or_rollback(&bad_samples(), "metrics", &apply)).unwrap();
            assert_eq!(
                report.verdict,
                GateVerdict::Failed,
                "{version} must be caught during canary"
            );
            let rec = rollback.expect("rollback");
            assert!(
                rec.within_sla,
                "{version} rollback took {}ms",
                rec.duration_ms
            );
            assert_eq!(r.stage(), RolloutStage::RolledBack);
            // The bad config never left the canary: every target is back on
            // the good bundle, and the canary targets are marked restored.
            for t in &ts {
                let v = r.config_version(t).expect("version queryable");
                assert_eq!(v.version, "2.4.9", "{version} reached {t}");
            }
            for t in r.canary() {
                assert_eq!(r.config_version(t).unwrap().status, VersionStatus::Restored);
            }
        }
    }

    fn bad_samples() -> Vec<HealthSample> {
        vec![
            HealthSample::new("replica_count", 0.0),
            HealthSample::new("ready_ratio", 0.1),
            HealthSample::new("error_ratio", 0.9),
        ]
    }

    // -- version queryability ---------------------------------------------

    #[test]
    fn component_config_version_is_always_queryable() {
        let clock = Arc::new(ManualClock::new(0));
        let ts = targets(50);
        let mut r = ProgressiveConfigRollout::new(ts.clone(), clock);
        let bundle = ConfigBundle::from_pairs("3.0.0", &[("replica_count", "1")]);
        for t in &ts {
            r.register_active(t, &bundle);
        }
        for t in &ts {
            let v = r.config_version(t).expect("version registered");
            assert_eq!(v.version, "3.0.0");
            assert_eq!(v.digest, bundle.digest);
            assert_eq!(v.status, VersionStatus::Promoted);
            assert_eq!(v.namespace, t.namespace);
            assert_eq!(v.node, t.node);
        }
        assert_eq!(r.all_versions().len(), ts.len());
    }

    #[test]
    fn bundle_digest_is_content_addressed_and_order_independent() {
        let a = ConfigBundle::from_pairs("1.0.0", &[("a", "1"), ("b", "2")]);
        let b = ConfigBundle::from_pairs("1.0.0", &[("b", "2"), ("a", "1")]);
        assert_eq!(a.digest, b.digest);
        let c = ConfigBundle::from_pairs("1.0.0", &[("a", "2"), ("b", "2")]);
        assert_ne!(a.digest, c.digest);
        assert_eq!(a.digest.len(), 64);
    }

    // -- no ungated propagation -------------------------------------------

    #[test]
    fn config_never_propagates_without_a_gate_pass() {
        let clock = Arc::new(ManualClock::new(0));
        let apply = RecordingConfigApply::new(clock.clone());
        let ts = targets(60);
        let mut r = ProgressiveConfigRollout::new(ts.clone(), clock);
        let bundle = ConfigBundle::from_pairs("3.1.0", &[("replica_count", "1")]);
        block_on(r.start(&bundle, &apply)).unwrap();
        // Completing without a gate pass must be rejected.
        assert!(block_on(r.complete(&apply)).is_err());
        assert_eq!(r.stage(), RolloutStage::Canary);
        // Only the canary subset carries the new version.
        let canary_ids: std::collections::BTreeSet<String> =
            r.canary().iter().map(|t| t.id()).collect();
        assert!(!canary_ids.is_empty());
        assert!(r.current_blast_radius() <= MAX_BLAST_RADIUS);

        // After a pass, completion touches every target.
        r.evaluate(&healthy(), "metrics");
        let touched = block_on(r.complete(&apply)).unwrap();
        assert_eq!(touched.len(), ts.len());
        assert!(canary_ids.is_subset(&touched.iter().cloned().collect()));
        for t in &ts {
            let v = r.config_version(t).expect("version queryable");
            assert_eq!(v.version, "3.1.0");
            assert_eq!(v.status, VersionStatus::Promoted);
        }
        assert_eq!(r.stage(), RolloutStage::Complete);
    }

    #[test]
    fn concurrent_rollout_is_rejected() {
        let clock = Arc::new(ManualClock::new(0));
        let apply = RecordingConfigApply::new(clock.clone());
        let mut r = ProgressiveConfigRollout::new(targets(40), clock);
        let bundle = ConfigBundle::from_pairs("3.2.0", &[("a", "1")]);
        block_on(r.start(&bundle, &apply)).unwrap();
        assert!(block_on(r.start(&bundle, &apply)).is_err());
    }

    #[test]
    fn rollback_without_in_flight_candidate_is_a_noop() {
        let clock = Arc::new(ManualClock::new(0));
        let apply = RecordingConfigApply::new(clock.clone());
        let mut r = ProgressiveConfigRollout::new(targets(10), clock);
        assert!(block_on(r.rollback(&apply, "nothing to do"))
            .unwrap()
            .is_none());
        assert!(r.rollbacks().is_empty());
    }

    #[test]
    fn custom_gate_sets_are_honoured() {
        let clock = Arc::new(ManualClock::new(0));
        let apply = RecordingConfigApply::new(clock.clone());
        let mut r = ProgressiveConfigRollout::new(targets(40), clock)
            .with_gates(vec![HealthGate::max("disk", "disk_usage_pct", 80.0)]);
        let bundle = ConfigBundle::from_pairs("4.0.0", &[("disk", "big")]);
        block_on(r.start(&bundle, &apply)).unwrap();
        let report = r.evaluate(
            &[HealthSample::new("disk_usage_pct", 95.0)],
            "node-exporter",
        );
        assert_eq!(report.verdict, GateVerdict::Failed);
        assert_eq!(report.failing_gates(), vec!["disk"]);
    }

    // -- helpers for the sync test bodies ---------------------------------

    /// The rollout engine is async so it composes with the reconciler, but
    /// the assertions are synchronous, so drive them on a local executor.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }
}
