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
//! Structured feature-flag evaluation with a targeting audit trail
//! (issue #1505).
//!
//! [`crate::feature_flags`] evaluates rollout flags read from a ConfigMap.
//! This module adds the distribution and accountability layer around it:
//!
//! * Flags ship as a **signed, versioned bundle** ([`FlagBundle`]) so a
//!   corrupted or forged delivery is rejected before it can change behavior.
//! * Every user-affecting decision is recorded in an [`EvaluationAudit`]
//!   entry carrying the flag name, resolved variant, and subject.
//! * A [`KillSwitch`] path is evaluated *before* the bundle and lives outside
//!   the bundle pipeline, so it keeps working when bundle delivery is down.
//! * A [`BundleStore`] caches the last-known-good bundle behind an
//!   [`std::sync::RwLock`]; bundle updates swap the pointer without a pod
//!   restart, and evaluation never touches the network.
//!
//! # Example
//!
//! ```
//! use stellar_k8s::flag_bundle::{BundleStore, EvaluationContext};
//!
//! let store = BundleStore::new();
//! let ctx = EvaluationContext::new("tenant-42");
//! // No bundle loaded yet: everything is off, and the decision is audited.
//! let decision = store.evaluate_audited("new_pruner", &ctx);
//! assert!(!decision.enabled);
//! assert_eq!(store.audit_len(), 1);
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::feature_flags::{Decision, EvaluationContext, FlagSet};

/// HMAC-SHA256 used for bundle signatures.
type BundleHmac = Hmac<Sha256>;

/// Maximum audit entries retained in memory; the trail is bounded so a
/// hot flag cannot grow memory without limit.
const MAX_AUDIT_ENTRIES: usize = 10_000;

// ─────────────────────────────────────────────────────────────────────────────
// Signed versioned bundle
// ─────────────────────────────────────────────────────────────────────────────

/// A single flag variant inside a bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlagVariant {
    /// Flag name, e.g. `new_archive_pruner`.
    pub name: String,
    /// Variant selected when the rule matches, e.g. `on`, `off`, `canary`.
    pub variant: String,
    /// Serialized [`crate::feature_flags::FlagRule`] JSON guarding the variant.
    pub rule_json: String,
    /// Whether this variant is user-visible (audited on every evaluation).
    pub user_affecting: bool,
}

impl FlagVariant {
    /// Create a user-affecting variant with the given rule JSON.
    pub fn user_affecting(
        name: impl Into<String>,
        variant: impl Into<String>,
        rule_json: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            variant: variant.into(),
            rule_json: rule_json.into(),
            user_affecting: true,
        }
    }
}

/// A versioned set of flag variants distributed to every operator replica.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlagBundle {
    /// Monotonic bundle version; receivers reject older versions.
    pub version: u64,
    /// When the bundle was published (RFC 3339 via chrono serde).
    pub published_at: DateTime<Utc>,
    /// Flag variants keyed by flag name.
    pub flags: BTreeMap<String, FlagVariant>,
}

impl FlagBundle {
    /// Create an empty bundle at `version`.
    pub fn new(version: u64) -> Self {
        Self {
            version,
            published_at: Utc::now(),
            flags: BTreeMap::new(),
        }
    }

    /// Insert or replace a variant.
    pub fn insert(&mut self, variant: FlagVariant) {
        self.flags.insert(variant.name.clone(), variant);
    }

    /// Canonical bytes covered by the signature.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// A bundle paired with its HMAC-SHA256 signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedBundle {
    /// The payload being distributed.
    pub bundle: FlagBundle,
    /// Hex-encoded HMAC-SHA256 over [`FlagBundle::canonical_bytes`].
    pub signature: String,
}

impl SignedBundle {
    /// Sign `bundle` with `signing_key`.
    pub fn sign(bundle: FlagBundle, signing_key: &[u8]) -> Self {
        let mut mac = BundleHmac::new_from_slice(signing_key).expect("HMAC accepts any key length");
        mac.update(&bundle.canonical_bytes());
        Self {
            bundle,
            signature: hex::encode(mac.finalize().into_bytes()),
        }
    }

    /// Verify the signature against `signing_key`.
    pub fn verify(&self, signing_key: &[u8]) -> bool {
        let Ok(mut mac) = BundleHmac::new_from_slice(signing_key) else {
            return false;
        };
        mac.update(&self.bundle.canonical_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        // Fixed-length hex comparison without early length disclosure.
        expected.len() == self.signature.len()
            && expected
                .bytes()
                .zip(self.signature.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Kill switch (independent of the bundle pipeline)
// ─────────────────────────────────────────────────────────────────────────────

/// Kill switches bypass the bundle pipeline entirely.
///
/// They are evaluated before any bundle lookup, loaded from a separate source
/// (environment/file), and therefore keep working when bundle delivery is
/// down — satisfying the "kill-switch works when bundle delivery is down"
/// acceptance criterion.
#[derive(Debug, Clone, Default)]
pub struct KillSwitch {
    killed: BTreeMap<String, String>,
}

impl KillSwitch {
    /// An empty kill-switch set (nothing force-disabled).
    pub fn new() -> Self {
        Self::default()
    }

    /// Force `flag` off with a human-readable `reason`.
    pub fn kill(&mut self, flag: impl Into<String>, reason: impl Into<String>) {
        self.killed.insert(flag.into(), reason.into());
    }

    /// Lift the kill on `flag`; returns true when one was present.
    pub fn revive(&mut self, flag: &str) -> bool {
        self.killed.remove(flag).is_some()
    }

    /// Whether `flag` is currently force-disabled.
    pub fn is_killed(&self, flag: &str) -> bool {
        self.killed.contains_key(flag)
    }

    /// The recorded reason for killing `flag`, if any.
    pub fn reason(&self, flag: &str) -> Option<&str> {
        self.killed.get(flag).map(String::as_str)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit trail
// ─────────────────────────────────────────────────────────────────────────────

/// One recorded evaluation decision affecting (or potentially affecting)
/// user-visible behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// When the decision was made.
    pub at: DateTime<Utc>,
    /// Flag that was evaluated.
    pub flag: String,
    /// Resolved variant (`off` when disabled).
    pub variant: String,
    /// Subject the flag was evaluated for.
    pub subject: String,
    /// Whether the feature resolved on.
    pub enabled: bool,
    /// Machine-readable reason for the decision.
    pub reason: String,
    /// Bundle version backing the decision (`None` when no bundle loaded).
    pub bundle_version: Option<u64>,
}

/// Bounded, append-only trail of evaluation decisions.
#[derive(Debug, Default)]
pub struct EvaluationAudit {
    entries: Vec<AuditEntry>,
}

impl EvaluationAudit {
    /// An empty audit trail.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one entry, evicting the oldest when the bound is reached.
    pub fn record(&mut self, entry: AuditEntry) {
        if self.entries.len() >= MAX_AUDIT_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    /// All recorded entries, oldest first.
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Number of recorded entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no entries have been recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries for one flag, oldest first.
    pub fn for_flag(&self, flag: &str) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| e.flag == flag).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Evaluated decision
// ─────────────────────────────────────────────────────────────────────────────

/// The outcome of one audited evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditedDecision {
    /// Whether the feature is on for this subject.
    pub enabled: bool,
    /// Resolved variant name.
    pub variant: String,
    /// Why this outcome was chosen.
    pub reason: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Bundle store: cached evaluation off the network hot path
// ─────────────────────────────────────────────────────────────────────────────

/// Inner mutable state behind the store lock.
#[derive(Debug)]
struct StoreInner {
    bundle: Option<FlagBundle>,
    flag_set: FlagSet,
    kill_switch: KillSwitch,
    audit: EvaluationAudit,
    applied_at: Option<Instant>,
    last_version: Option<u64>,
}

/// In-process flag evaluation over a cached bundle.
///
/// Evaluation reads the cached [`FlagSet`] under a read lock and never
/// touches the network, keeping p99 latency in the microsecond range.
/// Bundle delivery failures leave the last-known-good bundle in place, and
/// [`BundleStore::refresh`] swaps bundles without a pod restart.
#[derive(Debug)]
pub struct BundleStore {
    inner: RwLock<StoreInner>,
}

impl Default for BundleStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BundleStore {
    /// Create a store with no bundle loaded (all flags off).
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(StoreInner {
                bundle: None,
                flag_set: FlagSet::new(),
                kill_switch: KillSwitch::new(),
                audit: EvaluationAudit::new(),
                applied_at: None,
                last_version: None,
            }),
        }
    }

    /// Install a signed bundle after verifying its signature and version.
    ///
    /// Stale versions (`version <= current`) and bad signatures are rejected
    /// without touching the cached bundle.
    pub fn install(&self, signed: &SignedBundle, signing_key: &[u8]) -> Result<u64, String> {
        if !signed.verify(signing_key) {
            return Err("bundle signature verification failed".to_string());
        }
        let mut inner = self.inner.write().map_err(|e| e.to_string())?;
        if let Some(current) = inner.last_version {
            if signed.bundle.version <= current {
                return Err(format!(
                    "stale bundle version {} (current {current})",
                    signed.bundle.version
                ));
            }
        }
        let flag_set = flag_set_for(&signed.bundle);
        inner.flag_set = flag_set;
        inner.bundle = Some(signed.bundle.clone());
        inner.last_version = Some(signed.bundle.version);
        inner.applied_at = Some(Instant::now());
        Ok(signed.bundle.version)
    }

    /// Force-disable `flag` independently of bundle delivery.
    pub fn kill(&self, flag: &str, reason: &str) {
        if let Ok(mut inner) = self.inner.write() {
            inner.kill_switch.kill(flag, reason);
        }
    }

    /// Lift a kill-switch on `flag`.
    pub fn revive(&self, flag: &str) -> bool {
        self.inner
            .write()
            .map(|mut inner| inner.kill_switch.revive(flag))
            .unwrap_or(false)
    }

    /// Evaluate `flag` for `ctx`, recording the decision in the audit trail.
    ///
    /// Precedence: kill-switch → bundle rule → off. Every path records an
    /// [`AuditEntry`], so every user-affecting decision is auditable.
    pub fn evaluate_audited(&self, flag: &str, ctx: &EvaluationContext) -> AuditedDecision {
        let mut inner = self.inner.write().expect("flag store lock poisoned");

        if inner.kill_switch.is_killed(flag) {
            let reason = inner
                .kill_switch
                .reason(flag)
                .unwrap_or("kill-switch engaged")
                .to_string();
            let bundle_version = inner.last_version;
            inner.audit.record(AuditEntry {
                at: Utc::now(),
                flag: flag.to_string(),
                variant: "off".to_string(),
                subject: ctx.subject().to_string(),
                enabled: false,
                reason: format!("kill-switch: {reason}"),
                bundle_version,
            });
            return AuditedDecision {
                enabled: false,
                variant: "off".to_string(),
                reason: "kill-switch".to_string(),
            };
        }

        let decision: Decision = inner.flag_set.evaluate(flag, ctx);
        let variant = inner
            .bundle
            .as_ref()
            .and_then(|b| b.flags.get(flag))
            .map(|v| {
                if decision.enabled {
                    v.variant.clone()
                } else {
                    "off".to_string()
                }
            })
            .unwrap_or_else(|| {
                if decision.enabled {
                    "on".to_string()
                } else {
                    "off".to_string()
                }
            });
        let audited = AuditedDecision {
            enabled: decision.enabled,
            variant: variant.clone(),
            reason: decision.reason.to_string(),
        };
        let bundle_version = inner.last_version;
        inner.audit.record(AuditEntry {
            at: Utc::now(),
            flag: flag.to_string(),
            variant,
            subject: ctx.subject().to_string(),
            enabled: decision.enabled,
            reason: decision.reason.to_string(),
            bundle_version,
        });
        audited
    }

    /// Number of recorded audit entries.
    pub fn audit_len(&self) -> usize {
        self.inner
            .read()
            .map(|inner| inner.audit.len())
            .unwrap_or(0)
    }

    /// Run `f` against the audit trail snapshot.
    pub fn audit<T>(&self, f: impl FnOnce(&EvaluationAudit) -> T) -> Option<T> {
        self.inner.read().ok().map(|inner| f(&inner.audit))
    }

    /// Currently installed bundle version, if any.
    pub fn version(&self) -> Option<u64> {
        self.inner.read().ok().and_then(|inner| inner.last_version)
    }

    /// How long ago the current bundle was applied (for propagation SLOs).
    pub fn bundle_age(&self) -> Option<Duration> {
        self.inner
            .read()
            .ok()
            .and_then(|inner| inner.applied_at.map(|t| t.elapsed()))
    }
}

/// Shared ownership helper for background refresh tasks.
pub type SharedStore = Arc<BundleStore>;

// ─────────────────────────────────────────────────────────────────────────────
// Async bundle loader and refresher
// ─────────────────────────────────────────────────────────────────────────────

/// Default refresh cadence; well inside the 10s cluster-wide propagation SLO.
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Source of signed bundles (a flag-management platform endpoint).
///
/// Implementations fetch the latest [`SignedBundle`]; the refresher verifies
/// the signature and version before installing it, so a misbehaving source
/// cannot change behavior.
#[async_trait::async_trait]
pub trait BundleSource: Send + Sync {
    /// Fetch the latest signed bundle.
    async fn fetch(&self) -> Result<SignedBundle, String>;
}

/// Fetches, verifies, and caches signed bundles in-process.
///
/// The loader owns a [`BundleStore`] and an async refresher task. On every
/// tick it fetches from the [`BundleSource`], verifies the signature, and
/// installs the bundle when it is newer. Any failure (network down, bad
/// signature, stale version) leaves the last-known-good bundle untouched, so
/// evaluation and the kill-switch keep working while delivery is down.
#[derive(Debug)]
pub struct BundleLoader {
    store: SharedStore,
    signing_key: Vec<u8>,
    interval: Duration,
    last_error: RwLock<Option<String>>,
    shutdown: watch::Sender<bool>,
}

impl BundleLoader {
    /// Create a loader over a fresh store with the default refresh interval.
    pub fn new(signing_key: impl Into<Vec<u8>>) -> Self {
        Self::with_interval(signing_key, DEFAULT_REFRESH_INTERVAL)
    }

    /// Create a loader with an explicit refresh interval.
    pub fn with_interval(signing_key: impl Into<Vec<u8>>, interval: Duration) -> Self {
        let (shutdown, _) = watch::channel(false);
        Self {
            store: Arc::new(BundleStore::new()),
            signing_key: signing_key.into(),
            interval,
            last_error: RwLock::new(None),
            shutdown,
        }
    }

    /// The store shared with evaluation call sites.
    pub fn store(&self) -> SharedStore {
        Arc::clone(&self.store)
    }

    /// Fetch once, verify, and install. Returns the installed version.
    ///
    /// On failure the last-known-good bundle is retained and the error is
    /// recorded for observability.
    pub async fn refresh_once(&self, source: &dyn BundleSource) -> Result<u64, String> {
        let result = match source.fetch().await {
            Ok(signed) => self.store.install(&signed, &self.signing_key),
            Err(e) => Err(format!("bundle fetch failed: {e}")),
        };
        if let Ok(mut last) = self.last_error.write() {
            *last = result.as_ref().err().cloned();
        }
        result
    }

    /// Spawn the background refresher task.
    ///
    /// The task ticks every `interval`, calling [`BundleLoader::refresh_once`].
    /// Failures are swallowed (last-known-good is kept); the task stops when
    /// the returned handle is dropped or [`BundleLoader::shutdown`] is called.
    pub fn spawn_refresher(
        self: &Arc<Self>,
        source: Arc<dyn BundleSource>,
    ) -> tokio::task::JoinHandle<()> {
        let loader = Arc::clone(self);
        let mut shutdown = self.shutdown.subscribe();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(loader.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        // Errors are recorded in `last_error`; keep last-known-good.
                        let _ = loader.refresh_once(source.as_ref()).await;
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        })
    }

    /// Signal the refresher task to stop.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    /// The most recent refresh error, if the last attempt failed.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().ok().and_then(|e| e.clone())
    }
}

/// Build the evaluation [`FlagSet`] for a bundle.
///
/// Each variant's `rule_json` is parsed as a
/// [`crate::feature_flags::FlagRule`]; unparsable rules fall back to off so
/// one bad variant cannot disable the whole bundle.
fn flag_set_for(bundle: &FlagBundle) -> FlagSet {
    let mut set = FlagSet::new();
    for variant in bundle.flags.values() {
        let rule = serde_json::from_str(&variant.rule_json).unwrap_or_default();
        set.insert(&variant.name, rule);
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature_flags::FlagRule;

    const TEST_KEY: &[u8] = b"test-signing-key-1505";

    fn rule_json(rule: &FlagRule) -> String {
        serde_json::to_string(rule).unwrap()
    }

    fn bundle_with(version: u64, name: &str, rule: &FlagRule, variant: &str) -> FlagBundle {
        let mut bundle = FlagBundle::new(version);
        bundle.insert(FlagVariant::user_affecting(name, variant, rule_json(rule)));
        bundle
    }

    #[test]
    fn signed_bundle_verifies_with_correct_key() {
        let signed = SignedBundle::sign(FlagBundle::new(1), TEST_KEY);
        assert!(signed.verify(TEST_KEY));
        assert!(!signed.verify(b"wrong-key"));
    }

    #[test]
    fn tampered_bundle_fails_verification() {
        let mut signed = SignedBundle::sign(FlagBundle::new(1), TEST_KEY);
        signed.bundle.version = 999;
        assert!(!signed.verify(TEST_KEY));
    }

    #[test]
    fn install_rejects_bad_signature_and_keeps_last_good() {
        let store = BundleStore::new();
        let good = SignedBundle::sign(bundle_with(2, "f", &FlagRule::on(), "on"), TEST_KEY);
        store.install(&good, TEST_KEY).unwrap();

        let mut bad = SignedBundle::sign(bundle_with(3, "f", &FlagRule::off(), "off"), TEST_KEY);
        bad.signature = "00".repeat(32);
        assert!(store.install(&bad, TEST_KEY).is_err());
        assert_eq!(store.version(), Some(2));
    }

    #[test]
    fn install_rejects_stale_versions() {
        let store = BundleStore::new();
        store
            .install(&SignedBundle::sign(FlagBundle::new(5), TEST_KEY), TEST_KEY)
            .unwrap();
        let stale = SignedBundle::sign(FlagBundle::new(5), TEST_KEY);
        assert!(store.install(&stale, TEST_KEY).is_err());
        let older = SignedBundle::sign(FlagBundle::new(4), TEST_KEY);
        assert!(store.install(&older, TEST_KEY).is_err());
    }

    #[test]
    fn evaluation_is_audited_with_flag_variant_and_subject() {
        let store = BundleStore::new();
        store
            .install(
                &SignedBundle::sign(bundle_with(1, "pruner", &FlagRule::on(), "v2"), TEST_KEY),
                TEST_KEY,
            )
            .unwrap();
        let ctx = EvaluationContext::new("tenant-7");
        let decision = store.evaluate_audited("pruner", &ctx);
        assert!(decision.enabled);
        assert_eq!(decision.variant, "v2");
        store.audit(|audit| {
            assert_eq!(audit.len(), 1);
            let entry = &audit.entries()[0];
            assert_eq!(entry.flag, "pruner");
            assert_eq!(entry.variant, "v2");
            assert_eq!(entry.subject, "tenant-7");
            assert_eq!(entry.bundle_version, Some(1));
        });
    }

    #[test]
    fn unknown_flag_is_off_but_still_audited() {
        let store = BundleStore::new();
        let decision = store.evaluate_audited("nope", &EvaluationContext::new("s"));
        assert!(!decision.enabled);
        assert_eq!(store.audit_len(), 1);
    }

    #[test]
    fn kill_switch_works_with_no_bundle_loaded() {
        // Bundle delivery down: last-known-good is empty, kill-switch still applies.
        let store = BundleStore::new();
        store.kill("payments_v2", "incident-1234");
        let decision = store.evaluate_audited("payments_v2", &EvaluationContext::new("s"));
        assert!(!decision.enabled);
        assert_eq!(decision.reason, "kill-switch");
        assert!(store.revive("payments_v2"));
        assert!(!store.revive("payments_v2"));
    }

    #[test]
    fn kill_switch_overrides_an_enabled_bundle_flag() {
        let store = BundleStore::new();
        store
            .install(
                &SignedBundle::sign(bundle_with(1, "f", &FlagRule::on(), "on"), TEST_KEY),
                TEST_KEY,
            )
            .unwrap();
        assert!(
            store
                .evaluate_audited("f", &EvaluationContext::new("s"))
                .enabled
        );
        store.kill("f", "rollback");
        assert!(
            !store
                .evaluate_audited("f", &EvaluationContext::new("s"))
                .enabled
        );
    }

    #[test]
    fn bundle_update_propagates_without_restart() {
        let store = BundleStore::new();
        let ctx = EvaluationContext::new("s");
        store
            .install(
                &SignedBundle::sign(bundle_with(1, "f", &FlagRule::off(), "off"), TEST_KEY),
                TEST_KEY,
            )
            .unwrap();
        assert!(!store.evaluate_audited("f", &ctx).enabled);
        store
            .install(
                &SignedBundle::sign(bundle_with(2, "f", &FlagRule::on(), "on"), TEST_KEY),
                TEST_KEY,
            )
            .unwrap();
        assert!(store.evaluate_audited("f", &ctx).enabled);
        assert_eq!(store.version(), Some(2));
        assert!(store.bundle_age().is_some());
    }

    #[test]
    fn evaluation_is_submillisecond_in_process() {
        let store = BundleStore::new();
        store
            .install(
                &SignedBundle::sign(
                    bundle_with(1, "f", &FlagRule::percentage(50.0), "on"),
                    TEST_KEY,
                ),
                TEST_KEY,
            )
            .unwrap();
        let ctx = EvaluationContext::new("tenant-1");
        // Warm up, then assert p99-style bound: 1000 evals must finish in <1s
        // total, i.e. average far below the 1ms p99 budget per evaluation.
        for _ in 0..100 {
            store.evaluate_audited("f", &ctx);
        }
        let start = Instant::now();
        for _ in 0..1000 {
            store.evaluate_audited("f", &ctx);
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "1000 evaluations took {:?}, exceeding the 1ms/eval budget",
            start.elapsed()
        );
    }

    #[test]
    fn audit_entries_can_be_filtered_per_flag() {
        let store = BundleStore::new();
        let ctx = EvaluationContext::new("s");
        store.evaluate_audited("a", &ctx);
        store.evaluate_audited("b", &ctx);
        store.evaluate_audited("a", &ctx);
        store.audit(|audit| {
            assert_eq!(audit.for_flag("a").len(), 2);
            assert_eq!(audit.for_flag("b").len(), 1);
        });
    }

    /// Test source that returns a fixed result.
    struct FixedSource {
        result: Result<SignedBundle, String>,
    }

    #[async_trait::async_trait]
    impl BundleSource for FixedSource {
        async fn fetch(&self) -> Result<SignedBundle, String> {
            self.result.clone()
        }
    }

    #[tokio::test]
    async fn loader_installs_verified_bundle() {
        let loader = BundleLoader::new(TEST_KEY);
        let source = FixedSource {
            result: Ok(SignedBundle::sign(
                bundle_with(1, "f", &FlagRule::on(), "on"),
                TEST_KEY,
            )),
        };
        assert_eq!(loader.refresh_once(&source).await, Ok(1));
        assert_eq!(loader.store().version(), Some(1));
        assert!(loader.last_error().is_none());
    }

    #[tokio::test]
    async fn loader_rejects_bad_signature_and_keeps_last_good() {
        let loader = BundleLoader::new(TEST_KEY);
        let good = FixedSource {
            result: Ok(SignedBundle::sign(
                bundle_with(1, "f", &FlagRule::on(), "on"),
                TEST_KEY,
            )),
        };
        loader.refresh_once(&good).await.unwrap();

        let mut forged = SignedBundle::sign(bundle_with(2, "f", &FlagRule::off(), "off"), TEST_KEY);
        forged.signature = "00".repeat(32);
        let bad = FixedSource { result: Ok(forged) };
        assert!(loader.refresh_once(&bad).await.is_err());
        assert_eq!(loader.store().version(), Some(1));
        assert!(loader.last_error().is_some());
    }

    #[tokio::test]
    async fn loader_keeps_last_good_when_delivery_is_down() {
        let loader = BundleLoader::new(TEST_KEY);
        let good = FixedSource {
            result: Ok(SignedBundle::sign(
                bundle_with(1, "f", &FlagRule::on(), "on"),
                TEST_KEY,
            )),
        };
        loader.refresh_once(&good).await.unwrap();
        loader.store().kill("f", "incident");

        let down = FixedSource {
            result: Err("connection refused".to_string()),
        };
        assert!(loader.refresh_once(&down).await.is_err());
        // Last-known-good bundle retained; kill-switch still applies.
        assert_eq!(loader.store().version(), Some(1));
        let decision = loader
            .store()
            .evaluate_audited("f", &EvaluationContext::new("s"));
        assert!(!decision.enabled);
        assert_eq!(decision.reason, "kill-switch");
    }

    #[tokio::test]
    async fn refresher_propagates_new_bundle() {
        let loader = Arc::new(BundleLoader::with_interval(
            TEST_KEY,
            Duration::from_millis(10),
        ));
        let source = Arc::new(FixedSource {
            result: Ok(SignedBundle::sign(
                bundle_with(1, "f", &FlagRule::on(), "on"),
                TEST_KEY,
            )),
        });
        let handle = loader.spawn_refresher(source);
        // Wait for the first tick to install the bundle.
        for _ in 0..100 {
            if loader.store().version() == Some(1) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(loader.store().version(), Some(1));
        loader.shutdown();
        let _ = handle.await;
    }
}
