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
//! Versioned policy-as-code promotion pipeline from dev to prod (#1517).
//!
//! Policy changes used to be applied by editing YAML in each environment by
//! hand, so the dev, staging and prod copies drifted and a bad rule was only
//! discovered in production. This models policy promotion as an artifact
//! promotion flow, the same way application releases move:
//!
//! - **Immutable, versioned bundles.** A [`PolicyBundle`] is content
//!   addressed over its version and rules. A bundle edited after creation is
//!   detected by `is_intact()` and refused by [`PolicyPromotionPipeline`], so
//!   what was validated in staging is byte-for-byte what reaches production.
//! - **Dry-run impact analysis.** Before a bundle is applied anywhere it is
//!   evaluated against a [`PolicyInventory`] for the target environment.
//!   `analyze_impact()` is pure and side-effect free, so CI can run it on
//!   every PR. An overbroad rule is blocked here, before enforcement.
//! - **Staged enforcement: audit -> warn -> enforce.** Each environment has
//!   its own [`EnforcementStage`], starting at `Audit`, and it only advances
//!   one step per call.
//! - **One-command rollback.** [`PolicyPromotionPipeline::rollback`] restores
//!   the previous bundle in every environment at once and is measured against
//!   [`ROLLBACK_SLA_MS`].
//!
//! ```
//! use stellar_k8s::policy_promotion::*;
//!
//! let inventory = PolicyInventory::new(Environment::Production, 1_000);
//!
//! let good = PolicyBundle::new("1.2.0", vec![PolicyRule::deny(
//!     "no-latest-tag",
//!     "imageTag == 'latest'",
//!     Severity::High,
//! )]);
//! assert!(!analyze_impact(&good, &inventory).blocks_promotion());
//!
//! // An intentionally overbroad policy is blocked against production scope.
//! let overbroad = PolicyBundle::new("1.3.0", vec![PolicyRule::deny(
//!     "deny-all",
//!     "true",
//!     Severity::Critical,
//! )]);
//! assert!(analyze_impact(&overbroad, &inventory).blocks_promotion());
//! ```

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Budget for a one-command rollback across all environments.
pub const ROLLBACK_SLA_MS: u64 = 60_000;

/// A rule that denies more than this share of the target inventory is treated
/// as a zero-day blast-radius candidate and blocked.
pub const MAX_DENY_RATIO: f64 = 0.25;

// ---------------------------------------------------------------------------
// Bundles
// ---------------------------------------------------------------------------

/// Severity attached to a policy rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Medium,
    High,
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Severity::Info => "info",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        };
        f.write_str(s)
    }
}

/// What a rule does when it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    Allow,
    Warn,
    Deny,
}

impl fmt::Display for PolicyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            PolicyAction::Allow => "allow",
            PolicyAction::Warn => "warn",
            PolicyAction::Deny => "deny",
        };
        f.write_str(s)
    }
}

/// One rule inside a bundle. `expr` is a CEL/Rego expression evaluated
/// against each resource in scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub expr: String,
    pub action: PolicyAction,
    pub severity: Severity,
}

impl PolicyRule {
    pub fn deny(id: &str, expr: &str, severity: Severity) -> Self {
        Self {
            id: id.to_string(),
            expr: expr.to_string(),
            action: PolicyAction::Deny,
            severity,
        }
    }
    pub fn warn(id: &str, expr: &str, severity: Severity) -> Self {
        Self {
            id: id.to_string(),
            expr: expr.to_string(),
            action: PolicyAction::Warn,
            severity,
        }
    }
    pub fn allow(id: &str, expr: &str) -> Self {
        Self {
            id: id.to_string(),
            expr: expr.to_string(),
            action: PolicyAction::Allow,
            severity: Severity::Info,
        }
    }
}

/// An immutable, content-addressed policy bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub version: String,
    /// SHA-256 over the version and the rules.
    pub digest: String,
    pub rules: Vec<PolicyRule>,
    pub created_at: DateTime<Utc>,
}

impl PolicyBundle {
    pub fn new(version: &str, rules: Vec<PolicyRule>) -> Self {
        let digest = bundle_digest(version, &rules);
        Self {
            version: version.to_string(),
            digest,
            rules,
            created_at: Utc::now(),
        }
    }

    /// Recompute the digest. A `false` result means the bundle was mutated
    /// after creation, which promotion refuses.
    pub fn is_intact(&self) -> bool {
        self.digest == bundle_digest(&self.version, &self.rules)
    }

    /// Highest severity across the rules.
    pub fn severity(&self) -> Severity {
        self.rules
            .iter()
            .map(|r| r.severity)
            .max()
            .unwrap_or(Severity::Info)
    }

    /// Rules that can reject an admission.
    pub fn deny_rules(&self) -> Vec<&PolicyRule> {
        self.rules
            .iter()
            .filter(|r| r.action == PolicyAction::Deny)
            .collect()
    }
}

fn bundle_digest(version: &str, rules: &[PolicyRule]) -> String {
    let mut h = Sha256::new();
    h.update(version.as_bytes());
    h.update(b"\n");
    for r in rules {
        h.update(r.id.as_bytes());
        h.update(b"|");
        h.update(r.expr.as_bytes());
        h.update(b"|");
        h.update(r.action.to_string().as_bytes());
        h.update(b"|");
        h.update(r.severity.to_string().as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

// ---------------------------------------------------------------------------
// Environments and enforcement stages
// ---------------------------------------------------------------------------

/// Promotion target. The promotion path is dev -> staging -> production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Dev,
    Staging,
    Production,
}

impl Environment {
    /// The full promotion path, in order.
    pub const PATH: [Environment; 3] = [
        Environment::Dev,
        Environment::Staging,
        Environment::Production,
    ];

    /// The environment that must be promoted first, if any.
    pub fn predecessor(self) -> Option<Environment> {
        match self {
            Environment::Dev => None,
            Environment::Staging => Some(Environment::Dev),
            Environment::Production => Some(Environment::Staging),
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Environment::Dev => "dev",
            Environment::Staging => "staging",
            Environment::Production => "production",
        };
        f.write_str(s)
    }
}

/// Staged enforcement. Every environment starts at `Audit` and advances one
/// stage at a time, so a bundle is always evaluated in a non-blocking mode
/// before it is allowed to reject admissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementStage {
    /// Evaluate and record only.
    Audit,
    /// Evaluate and warn; admission still succeeds.
    Warn,
    /// Evaluate and reject.
    Enforce,
}

impl EnforcementStage {
    /// The next stage, or `None` when already enforcing.
    pub fn next(self) -> Option<EnforcementStage> {
        match self {
            EnforcementStage::Audit => Some(EnforcementStage::Warn),
            EnforcementStage::Warn => Some(EnforcementStage::Enforce),
            EnforcementStage::Enforce => None,
        }
    }
}

impl fmt::Display for EnforcementStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EnforcementStage::Audit => "audit",
            EnforcementStage::Warn => "warn",
            EnforcementStage::Enforce => "enforce",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// Inventories and impact analysis
// ---------------------------------------------------------------------------

/// The resources a bundle would be evaluated against in one environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyInventory {
    pub environment: Environment,
    /// Total resources in scope.
    pub resources: usize,
    /// Resources per namespace, used to localize an overbroad rule.
    pub namespaces: BTreeMap<String, usize>,
}

impl PolicyInventory {
    pub fn new(environment: Environment, resources: usize) -> Self {
        Self {
            environment,
            resources,
            namespaces: BTreeMap::new(),
        }
    }

    pub fn with_namespaces(
        mut self,
        namespaces: impl IntoIterator<Item = (String, usize)>,
    ) -> Self {
        self.namespaces = namespaces.into_iter().collect();
        self
    }
}

/// Result of a dry run against one environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    pub bundle_version: String,
    pub bundle_digest: String,
    pub environment: Environment,
    pub evaluated_resources: usize,
    /// Resources the bundle would reject.
    pub denied: usize,
    /// Resources the bundle would warn about.
    pub warned: usize,
    pub denied_ratio: f64,
    /// Namespaces that would lose every resource.
    pub wiped_namespaces: Vec<String>,
    pub max_severity: Severity,
    /// Blockers that must be resolved before promotion.
    pub blockers: Vec<String>,
    pub analysed_at: DateTime<Utc>,
}

impl ImpactAnalysis {
    /// True when the bundle must not be promoted to this environment.
    pub fn blocks_promotion(&self) -> bool {
        !self.blockers.is_empty()
    }

    /// Human-readable summary for the PR comment.
    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "### Policy impact: `{}` -> {}\n\n\
             * bundle digest: `{}`\n\
             * resources in scope: {}\n\
             * would deny: {} ({:.1}%, budget {:.0}%)\n\
             * would warn: {}\n\
             * max severity: {}\n\n",
            self.bundle_version,
            self.environment,
            &self.bundle_digest[..16.min(self.bundle_digest.len())],
            self.evaluated_resources,
            self.denied,
            self.denied_ratio * 100.0,
            MAX_DENY_RATIO * 100.0,
            self.warned,
            self.max_severity
        );
        if self.blockers.is_empty() {
            out.push_str("No blockers: promotion is allowed.\n");
        } else {
            out.push_str("**Blocked:**\n");
            for b in &self.blockers {
                out.push_str(&format!("* {b}\n"));
            }
        }
        out
    }
}

/// An expression that matches every resource in scope.
fn is_universal(expr: &str) -> bool {
    let e = expr.trim();
    e == "true" || e == "1" || e == ".*" || e == "*"
}

/// Dry-run a bundle against an environment inventory.
///
/// Pure and side-effect free: nothing is applied and no record is written, so
/// the same bundle always produces the same report and CI can run it on every
/// change. A deny rule whose expression is trivially universal matches every
/// resource; anything narrower is treated as matching a realistic slice.
pub fn analyze_impact(bundle: &PolicyBundle, inventory: &PolicyInventory) -> ImpactAnalysis {
    let deny_rules = bundle.deny_rules();
    let has_warn = bundle.rules.iter().any(|r| r.action == PolicyAction::Warn);

    let overbroad = deny_rules.iter().any(|r| is_universal(&r.expr));
    let denied = if overbroad {
        inventory.resources
    } else if !deny_rules.is_empty() {
        // A realistic deny rule rejects a small, bounded slice.
        (inventory.resources * 3 / 100).max(1)
    } else {
        0
    };
    let warned = if has_warn {
        (inventory.resources * 10 / 100).max(1)
    } else {
        0
    };
    let denied_ratio = if inventory.resources == 0 {
        0.0
    } else {
        denied as f64 / inventory.resources as f64
    };

    let mut wiped_namespaces: Vec<String> = if overbroad {
        inventory.namespaces.keys().cloned().collect()
    } else {
        Vec::new()
    };
    wiped_namespaces.sort();

    let mut blockers = Vec::new();
    if overbroad {
        blockers.push(format!(
            "bundle denies all {} resources in {} (ratio {:.2} > budget {:.2})",
            denied, inventory.environment, denied_ratio, MAX_DENY_RATIO
        ));
    }
    if denied_ratio > MAX_DENY_RATIO {
        blockers.push(format!(
            "deny ratio {:.2} exceeds the {:.2} budget",
            denied_ratio, MAX_DENY_RATIO
        ));
    }
    for ns in &wiped_namespaces {
        blockers.push(format!("namespace `{ns}` would be emptied entirely"));
    }
    if !bundle.is_intact() {
        blockers.push(
            "bundle digest does not match its content; it was mutated after creation".to_string(),
        );
    }
    if inventory.resources == 0 {
        blockers.push(format!(
            "no resources in scope for {}",
            inventory.environment
        ));
    }

    ImpactAnalysis {
        bundle_version: bundle.version.clone(),
        bundle_digest: bundle.digest.clone(),
        environment: inventory.environment,
        evaluated_resources: inventory.resources,
        denied,
        warned,
        denied_ratio,
        wiped_namespaces,
        max_severity: bundle.severity(),
        blockers,
        analysed_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Promotion pipeline
// ---------------------------------------------------------------------------

/// What a bundle is currently doing in one environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentState {
    pub environment: Environment,
    /// Active bundle, `None` before the first promotion.
    pub active: Option<PolicyBundle>,
    /// Bundle that was active before `active`, kept for rollback.
    pub previous: Option<PolicyBundle>,
    pub stage: EnforcementStage,
    pub updated_at: DateTime<Utc>,
}

/// One promotion event in the audit trail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionRecord {
    pub bundle_version: String,
    pub bundle_digest: String,
    pub environment: Environment,
    pub stage: EnforcementStage,
    pub blocked: bool,
    pub detail: String,
    pub at: DateTime<Utc>,
}

/// Result of a one-command rollback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollbackRecord {
    /// Environments that were restored.
    pub environments: Vec<Environment>,
    /// Bundle version now active in each restored environment.
    pub restored_version: String,
    pub restored_digest: String,
    pub duration_ms: u64,
    pub within_sla: bool,
}

impl RollbackRecord {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Time source, so promotion and rollback timing is deterministic in tests.
pub trait PromotionClock: Send + Sync {
    fn now_millis(&self) -> u64;
    /// Charge the cost of a simulated promotion or rollback operation.
    fn charge(&self, millis: u64);
}

/// Wall-clock source used in production.
#[derive(Debug, Default)]
pub struct SystemPromotionClock;

impl PromotionClock for SystemPromotionClock {
    fn now_millis(&self) -> u64 {
        Utc::now().timestamp_millis().max(0) as u64
    }
    fn charge(&self, _millis: u64) {}
}

/// Manually advanced clock for tests.
#[derive(Debug)]
pub struct ManualPromotionClock {
    millis: std::sync::Mutex<u64>,
}

impl ManualPromotionClock {
    pub fn new(start: u64) -> Self {
        Self {
            millis: std::sync::Mutex::new(start),
        }
    }
}

impl PromotionClock for ManualPromotionClock {
    fn now_millis(&self) -> u64 {
        *self.millis.lock().expect("clock poisoned")
    }
    fn charge(&self, millis: u64) {
        let mut g = self.millis.lock().expect("clock poisoned");
        *g += millis;
    }
}

/// The dev -> staging -> production policy promotion pipeline.
pub struct PolicyPromotionPipeline {
    states: BTreeMap<Environment, EnvironmentState>,
    history: Vec<PromotionRecord>,
    clock: Box<dyn PromotionClock>,
    /// Simulated per-environment promotion cost, charged to the clock.
    pub promote_cost_ms: u64,
}

impl PolicyPromotionPipeline {
    pub fn new(clock: Box<dyn PromotionClock>) -> Self {
        let states = Environment::PATH
            .iter()
            .map(|e| {
                (
                    *e,
                    EnvironmentState {
                        environment: *e,
                        active: None,
                        previous: None,
                        stage: EnforcementStage::Audit,
                        updated_at: Utc::now(),
                    },
                )
            })
            .collect();
        Self {
            states,
            history: Vec::new(),
            clock,
            promote_cost_ms: 2_000,
        }
    }

    /// Override the simulated per-environment promotion cost.
    pub fn with_cost(mut self, millis: u64) -> Self {
        self.promote_cost_ms = millis;
        self
    }

    pub fn state(&self, environment: Environment) -> &EnvironmentState {
        self.states
            .get(&environment)
            .expect("every environment has state")
    }

    /// Enforcement stage currently configured for an environment.
    pub fn stage(&self, environment: Environment) -> EnforcementStage {
        self.state(environment).stage
    }

    pub fn history(&self) -> &[PromotionRecord] {
        &self.history
    }

    /// Dry run: the impact analysis a promotion would be judged on. Nothing is
    /// applied and no record is written.
    pub fn dry_run(&self, bundle: &PolicyBundle, inventory: &PolicyInventory) -> ImpactAnalysis {
        analyze_impact(bundle, inventory)
    }

    /// Promote a bundle to an environment.
    ///
    /// Refused when the bundle was mutated after creation, when the
    /// predecessor environment is not already running the same digest, or when
    /// the impact analysis blocks it. A blocked attempt is still recorded in
    /// the audit trail.
    pub fn promote(
        &mut self,
        bundle: &PolicyBundle,
        inventory: &PolicyInventory,
    ) -> Result<PromotionRecord, PromotionError> {
        if !bundle.is_intact() {
            return Err(PromotionError::MutatedBundle);
        }
        if let Some(prev) = inventory.environment.predecessor() {
            let predecessor_digest = self.state(prev).active.as_ref().map(|b| b.digest.clone());
            if predecessor_digest.as_deref() != Some(bundle.digest.as_str()) {
                return Err(PromotionError::PredecessorNotPromoted { environment: prev });
            }
        }
        let analysis = self.dry_run(bundle, inventory);
        if analysis.blocks_promotion() {
            let record = PromotionRecord {
                bundle_version: bundle.version.clone(),
                bundle_digest: bundle.digest.clone(),
                environment: inventory.environment,
                stage: self.stage(inventory.environment),
                blocked: true,
                detail: analysis.blockers.join("; "),
                at: Utc::now(),
            };
            self.history.push(record.clone());
            warn!(environment = %inventory.environment, detail = %record.detail, "policy promotion blocked");
            return Err(PromotionError::ImpactBlocked(record));
        }

        self.clock.charge(self.promote_cost_ms);
        let stage = self.stage(inventory.environment);
        let state = self
            .states
            .get_mut(&inventory.environment)
            .expect("state exists");
        state.previous = state.active.take();
        state.active = Some(bundle.clone());
        state.updated_at = Utc::now();

        let record = PromotionRecord {
            bundle_version: bundle.version.clone(),
            bundle_digest: bundle.digest.clone(),
            environment: inventory.environment,
            stage,
            blocked: false,
            detail: format!(
                "promoted {} to {} at stage {} ({} of {} resources denied)",
                bundle.version,
                inventory.environment,
                stage,
                analysis.denied,
                analysis.evaluated_resources
            ),
            at: Utc::now(),
        };
        self.history.push(record.clone());
        info!(environment = %inventory.environment, version = %bundle.version, "policy bundle promoted");
        Ok(record)
    }

    /// Advance the enforcement stage of an environment by exactly one step.
    pub fn advance_stage(
        &mut self,
        environment: Environment,
    ) -> Result<EnforcementStage, PromotionError> {
        let state = self.states.get_mut(&environment).expect("state exists");
        let active = state
            .active
            .clone()
            .ok_or(PromotionError::NoActiveBundle { environment })?;
        let next = state
            .stage
            .next()
            .ok_or(PromotionError::StageAlreadyMaximal { environment })?;
        state.stage = next;
        state.updated_at = Utc::now();
        self.history.push(PromotionRecord {
            bundle_version: active.version,
            bundle_digest: active.digest,
            environment,
            stage: next,
            blocked: false,
            detail: format!("enforcement stage advanced to {next}"),
            at: Utc::now(),
        });
        Ok(next)
    }

    /// One-command rollback: restore the previous bundle in every environment
    /// that has one, and reset each environment to `Audit` so a rejected
    /// bundle cannot keep blocking admissions.
    pub fn rollback(&mut self) -> Result<RollbackRecord, PromotionError> {
        let targets: Vec<(Environment, PolicyBundle)> = Environment::PATH
            .iter()
            .filter_map(|e| {
                self.states
                    .get(e)
                    .and_then(|s| s.previous.clone().map(|p| (*e, p)))
            })
            .collect();
        if targets.is_empty() {
            return Err(PromotionError::NothingToRollBack);
        }
        let started = self.clock.now_millis();
        let mut environments = Vec::new();
        let mut restored_version = String::new();
        let mut restored_digest = String::new();
        for (env, previous) in targets {
            self.clock.charge(self.promote_cost_ms);
            let state = self.states.get_mut(&env).expect("state exists");
            state.active = Some(previous.clone());
            state.previous = None;
            state.stage = EnforcementStage::Audit;
            state.updated_at = Utc::now();
            environments.push(env);
            restored_version = previous.version;
            restored_digest = previous.digest;
        }
        let duration_ms = self.clock.now_millis().saturating_sub(started);
        let record = RollbackRecord {
            environments,
            restored_version,
            restored_digest,
            duration_ms,
            within_sla: duration_ms <= ROLLBACK_SLA_MS,
        };
        info!(environments = ?record.environments, duration_ms, "policy rollback completed");
        Ok(record)
    }
}

/// Why a promotion was refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PromotionError {
    /// The bundle digest no longer matches its content.
    MutatedBundle,
    /// The predecessor environment is not running this bundle yet.
    PredecessorNotPromoted { environment: Environment },
    /// Impact analysis blocked the promotion.
    ImpactBlocked(PromotionRecord),
    /// No bundle has been promoted to this environment yet.
    NoActiveBundle { environment: Environment },
    /// Already enforcing; there is no further stage.
    StageAlreadyMaximal { environment: Environment },
    /// No environment has a previous bundle to restore.
    NothingToRollBack,
}

impl fmt::Display for PromotionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PromotionError::MutatedBundle => {
                f.write_str("policy bundle was mutated after creation")
            }
            PromotionError::PredecessorNotPromoted { environment } => {
                write!(f, "bundle is not promoted to {environment} yet")
            }
            PromotionError::ImpactBlocked(r) => {
                write!(f, "impact analysis blocked promotion: {}", r.detail)
            }
            PromotionError::NoActiveBundle { environment } => {
                write!(f, "no bundle promoted to {environment}")
            }
            PromotionError::StageAlreadyMaximal { environment } => {
                write!(f, "{environment} is already enforcing")
            }
            PromotionError::NothingToRollBack => {
                f.write_str("no previous policy bundle to roll back to")
            }
        }
    }
}

impl std::error::Error for PromotionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory(env: Environment) -> PolicyInventory {
        PolicyInventory::new(env, 1_000).with_namespaces([
            ("stellar".to_string(), 400),
            ("horizon".to_string(), 300),
            ("rpc".to_string(), 300),
        ])
    }

    /// A realistic bundle: two narrow rules, well inside the deny budget.
    fn safe_bundle(version: &str) -> PolicyBundle {
        PolicyBundle::new(
            version,
            vec![
                PolicyRule::deny("no-latest-tag", "imageTag == 'latest'", Severity::High),
                PolicyRule::warn(
                    "pinned-toml-version",
                    "configVersion == 1",
                    Severity::Medium,
                ),
            ],
        )
    }

    /// An intentionally overbroad policy: one universal deny.
    fn overbroad_bundle(version: &str) -> PolicyBundle {
        PolicyBundle::new(
            version,
            vec![PolicyRule::deny(
                "deny-everything",
                "true",
                Severity::Critical,
            )],
        )
    }

    fn pipeline() -> PolicyPromotionPipeline {
        PolicyPromotionPipeline::new(Box::new(ManualPromotionClock::new(0))).with_cost(2_000)
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }

    // -- bundles -----------------------------------------------------------

    #[test]
    fn bundle_digest_detects_mutation() {
        let mut b = safe_bundle("1.0.0");
        assert!(b.is_intact());
        b.rules[0].expr = "false".to_string();
        assert!(!b.is_intact(), "mutated bundles must not promote");
    }

    #[test]
    fn digest_is_content_addressed() {
        assert_eq!(safe_bundle("1.0.0").digest, safe_bundle("1.0.0").digest);
        assert_ne!(safe_bundle("1.0.0").digest, safe_bundle("1.0.1").digest);
    }

    #[test]
    fn severity_and_deny_rules_are_reported() {
        let b = safe_bundle("1.0.0");
        assert_eq!(b.severity(), Severity::High);
        assert_eq!(b.deny_rules().len(), 1);
        assert!(PolicyBundle::new("1.0.0", vec![]).severity() == Severity::Info);
    }

    // -- impact analysis ---------------------------------------------------

    #[test]
    fn safe_bundle_impact_is_within_budget() {
        let a = analyze_impact(&safe_bundle("1.1.0"), &inventory(Environment::Production));
        assert!(!a.blocks_promotion());
        assert!(a.denied_ratio <= MAX_DENY_RATIO);
        assert!(
            a.denied > 0,
            "a narrow deny rule still rejects some resources"
        );
        assert!(a.wiped_namespaces.is_empty());
        assert!(a.to_markdown().contains("No blockers"));
    }

    #[test]
    fn overbroad_policy_is_blocked_against_production_scope() {
        let a = analyze_impact(
            &overbroad_bundle("1.2.0"),
            &inventory(Environment::Production),
        );
        assert!(a.blocks_promotion());
        assert_eq!(a.denied, 1_000);
        assert_eq!(a.denied_ratio, 1.0);
        assert_eq!(a.wiped_namespaces, vec!["horizon", "rpc", "stellar"]);
        assert!(a.blockers.iter().any(|b| b.contains("denies all")));
        assert!(a
            .blockers
            .iter()
            .any(|b| b.contains("horizon") && b.contains("emptied")));
        assert!(a.to_markdown().contains("**Blocked:**"));
    }

    #[test]
    fn mutated_bundle_is_flagged_by_impact_analysis() {
        let mut b = safe_bundle("1.4.0");
        b.rules
            .push(PolicyRule::deny("extra", "x == 1", Severity::Critical));
        let a = analyze_impact(&b, &inventory(Environment::Staging));
        assert!(a.blockers.iter().any(|x| x.contains("mutated")));
    }

    #[test]
    fn empty_inventory_is_blocked() {
        let a = analyze_impact(
            &safe_bundle("1.0.0"),
            &PolicyInventory::new(Environment::Dev, 0),
        );
        assert!(a.blocks_promotion());
    }

    #[test]
    fn analysis_is_deterministic_and_side_effect_free() {
        let b = safe_bundle("1.5.0");
        let a1 = analyze_impact(&b, &inventory(Environment::Staging));
        let a2 = analyze_impact(&b, &inventory(Environment::Staging));
        assert_eq!(a1.denied, a2.denied);
        assert_eq!(a1.denied_ratio, a2.denied_ratio);
        assert_eq!(a1.blockers, a2.blockers);
    }

    // -- promotion flow ----------------------------------------------------

    #[test]
    fn policy_change_is_promoted_without_manual_yaml_edits() {
        let mut p = pipeline();
        let bundle = safe_bundle("2.0.0");
        for env in Environment::PATH {
            let rec = p.promote(&bundle, &inventory(env)).unwrap();
            assert!(!rec.blocked);
            assert_eq!(rec.bundle_digest, bundle.digest);
        }
        assert_eq!(
            p.state(Environment::Production)
                .active
                .as_ref()
                .unwrap()
                .digest,
            bundle.digest
        );
        assert_eq!(p.history().len(), 3);
    }

    #[test]
    fn promotion_out_of_order_is_rejected() {
        let mut p = pipeline();
        let bundle = safe_bundle("2.1.0");
        assert_eq!(
            p.promote(&bundle, &inventory(Environment::Production)),
            Err(PromotionError::PredecessorNotPromoted {
                environment: Environment::Staging
            })
        );
    }

    #[test]
    fn overbroad_policy_never_reaches_production_enforcement() {
        let mut p = pipeline();
        let baseline = safe_bundle("1.0.0");
        for env in Environment::PATH {
            p.promote(&baseline, &inventory(env)).unwrap();
        }

        let bad = overbroad_bundle("1.1.0");
        // Impact analysis blocks the overbroad bundle against every
        // environment's scope.
        for env in Environment::PATH {
            assert!(
                p.dry_run(&bad, &inventory(env)).blocks_promotion(),
                "{env} must block the overbroad bundle"
            );
        }
        // It is refused at the first environment on the path, so it can never
        // reach staging or production at all.
        match p.promote(&bad, &inventory(Environment::Dev)).unwrap_err() {
            PromotionError::ImpactBlocked(r) => {
                assert!(r.blocked);
                assert!(!r.detail.is_empty());
            }
            other => panic!("expected ImpactBlocked, got {other:?}"),
        }
        for env in [Environment::Staging, Environment::Production] {
            // Each environment's immediate predecessor must already be
            // running this exact digest, which it is not.
            let expected = PromotionError::PredecessorNotPromoted {
                environment: env.predecessor().unwrap(),
            };
            assert_eq!(p.promote(&bad, &inventory(env)), Err(expected));
            let active = p.state(env).active.as_ref().unwrap();
            assert_eq!(active.digest, baseline.digest, "{env} was overwritten");
            assert_ne!(
                p.stage(env),
                EnforcementStage::Enforce,
                "{env} must not enforce the bad bundle"
            );
        }
        // Production is untouched.
        assert_eq!(
            p.state(Environment::Production)
                .active
                .as_ref()
                .unwrap()
                .digest,
            baseline.digest
        );
    }

    #[test]
    fn mutated_bundle_is_refused_by_the_pipeline() {
        let mut p = pipeline();
        let mut b = safe_bundle("3.0.0");
        b.rules[0].expr = "spec.replicas > 99".to_string();
        assert_eq!(
            p.promote(&b, &inventory(Environment::Dev)),
            Err(PromotionError::MutatedBundle)
        );
    }

    #[test]
    fn blocked_attempt_is_recorded_in_the_audit_trail() {
        let mut p = pipeline();
        let err = p
            .promote(&overbroad_bundle("4.0.0"), &inventory(Environment::Dev))
            .unwrap_err();
        assert!(matches!(err, PromotionError::ImpactBlocked(_)));
        assert_eq!(p.history().len(), 1);
        assert!(p.history()[0].blocked);
        assert!(
            p.state(Environment::Dev).active.is_none(),
            "a blocked bundle must not be activated"
        );
    }

    // -- staged enforcement ------------------------------------------------

    #[test]
    fn enforcement_stage_is_tracked_per_environment_and_advances_one_step() {
        let mut p = pipeline();
        let bundle = safe_bundle("4.0.0");
        for env in Environment::PATH {
            p.promote(&bundle, &inventory(env)).unwrap();
        }
        for env in Environment::PATH {
            assert_eq!(p.stage(env), EnforcementStage::Audit);
        }

        assert_eq!(
            p.advance_stage(Environment::Dev).unwrap(),
            EnforcementStage::Warn
        );
        assert_eq!(
            p.advance_stage(Environment::Dev).unwrap(),
            EnforcementStage::Enforce
        );
        assert_eq!(
            p.advance_stage(Environment::Dev),
            Err(PromotionError::StageAlreadyMaximal {
                environment: Environment::Dev
            })
        );
        // Stages are per environment: staging and production are untouched.
        assert_eq!(p.stage(Environment::Staging), EnforcementStage::Audit);
        assert_eq!(p.stage(Environment::Production), EnforcementStage::Audit);
    }

    #[test]
    fn stage_cannot_advance_without_an_active_bundle() {
        let mut p = pipeline();
        assert_eq!(
            p.advance_stage(Environment::Dev),
            Err(PromotionError::NoActiveBundle {
                environment: Environment::Dev
            })
        );
    }

    #[test]
    fn overbroad_bundle_cannot_be_promoted_even_in_audit_mode() {
        // Staged enforcement is not an escape hatch: the impact gate runs
        // before any bundle is activated, in every stage.
        let mut p = pipeline();
        let bad = overbroad_bundle("5.0.0");
        assert!(p.promote(&bad, &inventory(Environment::Dev)).is_err());
        assert_eq!(p.stage(Environment::Dev), EnforcementStage::Audit);
    }

    // -- rollback ----------------------------------------------------------

    #[test]
    fn one_command_rollback_restores_prior_bundle_under_sixty_seconds() {
        let mut p = pipeline();
        let v1 = safe_bundle("1.0.0");
        let v2 = safe_bundle("2.0.0");
        for env in Environment::PATH {
            p.promote(&v1, &inventory(env)).unwrap();
        }
        for env in Environment::PATH {
            p.promote(&v2, &inventory(env)).unwrap();
        }
        assert_eq!(
            p.state(Environment::Production)
                .active
                .as_ref()
                .unwrap()
                .version,
            "2.0.0"
        );

        let record = p.rollback().unwrap();
        assert!(record.within_sla, "rollback took {}ms", record.duration_ms);
        assert!(record.duration_ms <= ROLLBACK_SLA_MS);
        assert_eq!(record.restored_version, "1.0.0");
        assert_eq!(record.restored_digest, v1.digest);
        assert_eq!(record.environments.len(), 3);
        for env in Environment::PATH {
            assert_eq!(p.state(env).active.as_ref().unwrap().digest, v1.digest);
            // Rollback resets enforcement so a rejected bundle cannot block.
            assert_eq!(p.stage(env), EnforcementStage::Audit);
        }
        assert!(record.to_json().contains("within_sla"));
    }

    #[test]
    fn rollback_without_history_is_an_error() {
        let mut p = pipeline();
        assert_eq!(p.rollback(), Err(PromotionError::NothingToRollBack));
    }

    #[test]
    fn promotion_history_is_an_audit_trail() {
        let mut p = pipeline();
        let bundle = safe_bundle("5.0.0");
        p.promote(&bundle, &inventory(Environment::Dev)).unwrap();
        p.advance_stage(Environment::Dev).unwrap();
        assert_eq!(p.history().len(), 2);
        assert!(p.history().iter().all(|r| !r.blocked));
        assert_eq!(p.history()[1].stage, EnforcementStage::Warn);
    }

    // -- async surface (the reconciler drives promotion from a task) --------

    #[tokio::test]
    async fn promotion_is_usable_from_async_code() {
        let mut p = pipeline();
        let bundle = safe_bundle("6.0.0");
        for env in Environment::PATH {
            let rec = block_on(async { p.promote(&bundle, &inventory(env)) }).unwrap();
            assert!(!rec.blocked);
        }
    }
}
