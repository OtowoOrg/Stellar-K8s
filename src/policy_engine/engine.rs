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
//! Signed-bundle verification, LRU cache, rollback, and fail-closed admission.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer, SigningKey};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::controller::vsl::verify_ed25519_signature;
use crate::crd::secret_policy::KmsProvider;
use crate::crd::stellar_policy_bundle::annotations;
use crate::crd::{CelPolicySpec, PolicyTrustRootRef, StellarPolicyBundleSpec};
use crate::error::{Error, Result};
use crate::policy_engine::cel::{self, Expr};

/// Admission snapshot evaluated by the signed policy engine.
///
/// Kept independent of the `admission-webhook` feature so key-management
/// and controller paths can load bundles without compiling Wasm.
#[derive(Clone, Debug, Default)]
pub struct AdmissionView {
    pub operation: String,
    pub namespace: String,
    pub name: String,
    pub object: Option<serde_json::Value>,
    pub username: String,
}

/// Maximum accepted clock skew for `notBefore` (fail-closed beyond this).
pub const MAX_CLOCK_SKEW: Duration = Duration::from_secs(30);

/// Default LRU capacity for verified bundles.
const DEFAULT_CACHE: usize = 32;

/// Cluster-wide propagation SLO from the epic.
pub const PROPAGATION_SLO: Duration = Duration::from_secs(10);

/// Admission evaluation SLO (p99) from the epic.
pub const EVAL_P99_SLO: Duration = Duration::from_millis(5);

/// Trust anchors for Ed25519 policy-bundle signatures.
#[derive(Clone, Debug, Default)]
pub struct TrustRoot {
    /// key_id → base64-encoded 32-byte Ed25519 public key.
    keys: BTreeMap<String, String>,
    /// Optional KMS provenance recorded for audit / key-management integration.
    kms_bindings: BTreeMap<String, (KmsProvider, String)>,
}

impl TrustRoot {
    /// Empty trust root — every signed load fails closed.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Insert a verifying key (`key_id` → base64 public key).
    pub fn insert(&mut self, key_id: impl Into<String>, pubkey_b64: impl Into<String>) {
        self.keys.insert(key_id.into(), pubkey_b64.into());
    }

    /// Bind a key id to an existing SecretPolicy / KMS inventory entry.
    pub fn bind_kms(
        &mut self,
        key_id: impl Into<String>,
        provider: KmsProvider,
        kms_key_id: impl Into<String>,
    ) {
        self.kms_bindings
            .insert(key_id.into(), (provider, kms_key_id.into()));
    }

    /// Resolve a verifying key by id.
    pub fn public_key(&self, key_id: &str) -> Option<&str> {
        self.keys.get(key_id).map(String::as_str)
    }

    /// Apply a CRD trust-root reference (Secret / KMS metadata).
    pub fn apply_ref(&self, trust_ref: &PolicyTrustRootRef, key_id: &str) -> Result<&str> {
        if let Some(expected) = trust_ref.kms_key_id.as_deref() {
            match self.kms_bindings.get(key_id) {
                Some((_provider, bound)) if bound == expected => {}
                Some(_) => {
                    return Err(Error::ValidationError(
                        "trust root KMS key id does not match configured inventory".to_string(),
                    ))
                }
                None => {
                    return Err(Error::ValidationError(
                        "trust root KMS binding missing for signer".to_string(),
                    ))
                }
            }
        }
        self.public_key(key_id).ok_or_else(|| {
            Error::ValidationError("incorrect trust root: signer key id is not trusted".to_string())
        })
    }
}

/// Clock used so adversarial expiry / skew tests stay deterministic.
pub trait Clock: Send + Sync {
    /// Current UTC instant.
    fn now(&self) -> DateTime<Utc>;
}

/// System UTC clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Test / injected clock.
#[derive(Debug, Clone)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// Unsigned canonical payload that is hashed and signed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalBundle {
    pub algorithm: String,
    pub key_id: String,
    pub nonce: String,
    pub not_after: DateTime<Utc>,
    pub not_before: DateTime<Utc>,
    pub policies: Vec<CelPolicySpec>,
    pub version: String,
}

impl CanonicalBundle {
    /// Canonical JSON bytes (field order is structurally fixed).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    /// SHA-256 hex digest of the canonical payload.
    pub fn digest(&self) -> Result<String> {
        let bytes = self.canonical_bytes()?;
        Ok(hex::encode(Sha256::digest(&bytes)))
    }
}

/// Signed bundle accepted from a CR / operator-independent authoring pipeline.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedPolicyBundle {
    #[serde(flatten)]
    pub body: CanonicalBundle,
    /// Base64 Ed25519 signature. Empty means unsigned.
    #[serde(default)]
    pub signature: String,
}

impl SignedPolicyBundle {
    /// Build a signed bundle with the given signing key.
    pub fn sign(body: CanonicalBundle, signing_key: &SigningKey) -> Result<Self> {
        let payload = body.canonical_bytes()?;
        let sig = signing_key.sign(&payload);
        Ok(Self {
            body,
            signature: STANDARD.encode(sig.to_bytes()),
        })
    }

    /// Convert a CRD spec into the wire bundle type.
    pub fn from_spec(spec: &StellarPolicyBundleSpec) -> Self {
        Self {
            body: CanonicalBundle {
                algorithm: spec.algorithm.clone(),
                key_id: spec.key_id.clone(),
                nonce: spec.nonce.clone(),
                not_after: spec.not_after,
                not_before: spec.not_before,
                policies: spec.policies.clone(),
                version: spec.version.clone(),
            },
            signature: spec.signature.clone(),
        }
    }
}

/// Why a bundle was rejected (all paths fail closed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleRejectReason {
    Unsigned,
    InvalidSignature,
    Tampered,
    Expired,
    ClockSkewed,
    Replayed,
    IncorrectTrustRoot,
    Malformed(String),
}

impl std::fmt::Display for BundleRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => write!(f, "unsigned policy bundle"),
            Self::InvalidSignature => write!(f, "invalid policy bundle signature"),
            Self::Tampered => write!(f, "tampered policy bundle"),
            Self::Expired => write!(f, "expired policy bundle"),
            Self::ClockSkewed => write!(f, "clock-skewed policy bundle"),
            Self::Replayed => write!(f, "replayed policy bundle"),
            Self::IncorrectTrustRoot => write!(f, "incorrect trust root"),
            Self::Malformed(s) => write!(f, "malformed policy bundle: {s}"),
        }
    }
}

/// Compiled, already-verified bundle stored in the LRU.
#[derive(Clone, Debug)]
pub struct CompiledBundle {
    pub digest: String,
    pub version: String,
    pub policies: Vec<(String, Expr)>,
    pub verified_at: Instant,
}

/// Admission decision from the policy engine.
#[derive(Clone, Debug)]
pub struct AdmitDecision {
    pub allowed: bool,
    pub message: Option<String>,
    pub warnings: Vec<String>,
    pub eval_duration: Duration,
    pub bundle_hash: Option<String>,
    pub emergency_override: bool,
}

/// Prometheus-style counters and gauges for the engine.
#[derive(Debug, Default)]
pub struct PolicyEngineMetrics {
    pub eval_count: AtomicU64,
    pub eval_ns_total: AtomicU64,
    pub eval_ns_max: AtomicU64,
    pub bundle_verifications: AtomicU64,
    pub rejected_bundles: AtomicU64,
    pub cache_hits: AtomicU64,
    pub rollbacks: AtomicU64,
    pub emergency_overrides: AtomicU64,
    pub emergency_override_rejected: AtomicU64,
    pub propagation_ns_total: AtomicU64,
    pub propagation_events: AtomicU64,
}

impl PolicyEngineMetrics {
    fn record_eval(&self, d: Duration) {
        let ns = d.as_nanos() as u64;
        self.eval_count.fetch_add(1, Ordering::Relaxed);
        self.eval_ns_total.fetch_add(ns, Ordering::Relaxed);
        let mut current = self.eval_ns_max.load(Ordering::Relaxed);
        while ns > current {
            match self.eval_ns_max.compare_exchange_weak(
                current,
                ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(v) => current = v,
            }
        }
    }

    /// Render text exposition for the existing metrics pipeline.
    pub fn render_prometheus(&self, active_hash: &str, active_version: &str) -> String {
        format!(
            "# HELP stellar_policy_eval_latency_ns Admission policy evaluation latency (ns).\n\
             # TYPE stellar_policy_eval_latency_ns gauge\n\
             stellar_policy_eval_total {}\n\
             stellar_policy_eval_latency_ns_total {}\n\
             stellar_policy_eval_latency_ns_max {}\n\
             # HELP stellar_policy_bundle_verifications_total Bundle signature verifications.\n\
             # TYPE stellar_policy_bundle_verifications_total counter\n\
             stellar_policy_bundle_verifications_total {}\n\
             # HELP stellar_policy_rejected_bundles_total Cryptographically invalid bundles.\n\
             # TYPE stellar_policy_rejected_bundles_total counter\n\
             stellar_policy_rejected_bundles_total {}\n\
             stellar_policy_bundle_cache_hits_total {}\n\
             stellar_policy_rollbacks_total {}\n\
             stellar_policy_emergency_overrides_total {}\n\
             stellar_policy_emergency_override_rejected_total {}\n\
             stellar_policy_propagation_latency_ns_total {}\n\
             stellar_policy_propagation_events_total {}\n\
             stellar_policy_active_bundle_info{{hash=\"{}\",version=\"{}\"}} 1\n",
            self.eval_count.load(Ordering::Relaxed),
            self.eval_ns_total.load(Ordering::Relaxed),
            self.eval_ns_max.load(Ordering::Relaxed),
            self.bundle_verifications.load(Ordering::Relaxed),
            self.rejected_bundles.load(Ordering::Relaxed),
            self.cache_hits.load(Ordering::Relaxed),
            self.rollbacks.load(Ordering::Relaxed),
            self.emergency_overrides.load(Ordering::Relaxed),
            self.emergency_override_rejected.load(Ordering::Relaxed),
            self.propagation_ns_total.load(Ordering::Relaxed),
            self.propagation_events.load(Ordering::Relaxed),
            active_hash,
            active_version,
        )
    }
}

struct EngineInner {
    current: Option<Arc<CompiledBundle>>,
    previous: Option<Arc<CompiledBundle>>,
    cache: LruCache<String, Arc<CompiledBundle>>,
    seen_nonces: HashMap<(String, String), String>,
    last_verified_digest: Option<String>,
}

/// In-process signed CEL policy engine used by the admission webhook.
pub struct PolicyEngine {
    trust: RwLock<TrustRoot>,
    inner: Mutex<EngineInner>,
    clock: Arc<dyn Clock>,
    enforced: AtomicBool,
    /// Shared generation used for cluster-wide fan-out.
    generation: Arc<RwLock<(u64, Option<Arc<CompiledBundle>>, Instant)>>,
    pub metrics: PolicyEngineMetrics,
}

impl PolicyEngine {
    /// Create an unenforced engine with the given trust root.
    pub fn new(trust: TrustRoot) -> Self {
        Self::with_clock(trust, Arc::new(SystemClock))
    }

    /// Create an engine with an injected clock (adversarial time tests).
    pub fn with_clock(trust: TrustRoot, clock: Arc<dyn Clock>) -> Self {
        Self {
            trust: RwLock::new(trust),
            inner: Mutex::new(EngineInner {
                current: None,
                previous: None,
                cache: LruCache::new(NonZeroUsize::new(DEFAULT_CACHE).expect("cache size")),
                seen_nonces: HashMap::new(),
                last_verified_digest: None,
            }),
            clock,
            enforced: AtomicBool::new(false),
            generation: Arc::new(RwLock::new((0, None, Instant::now()))),
            metrics: PolicyEngineMetrics::default(),
        }
    }

    /// Subscribe a replica engine to the same generation slot (propagation).
    pub fn share_generation(&self) -> Arc<RwLock<(u64, Option<Arc<CompiledBundle>>, Instant)>> {
        Arc::clone(&self.generation)
    }

    /// Pull the latest published bundle from a shared generation bus.
    pub fn sync_from(
        &self,
        bus: &RwLock<(u64, Option<Arc<CompiledBundle>>, Instant)>,
    ) -> Option<Duration> {
        let guard = bus.read().unwrap_or_else(|e| e.into_inner());
        let published_at = guard.2;
        let bundle = guard.1.clone()?;
        drop(guard);
        let mut inner = self.lock_inner();
        if inner.current.as_ref().map(|c| c.digest.as_str()) != Some(bundle.digest.as_str()) {
            inner.previous = inner.current.clone();
            inner.current = Some(bundle);
            self.enforced.store(true, Ordering::SeqCst);
            let lag = published_at.elapsed();
            self.metrics
                .propagation_ns_total
                .fetch_add(lag.as_nanos() as u64, Ordering::Relaxed);
            self.metrics
                .propagation_events
                .fetch_add(1, Ordering::Relaxed);
            return Some(lag);
        }
        Some(Duration::ZERO)
    }

    /// Whether signed-bundle evaluation is required (fail-closed).
    pub fn is_enforced(&self) -> bool {
        self.enforced.load(Ordering::SeqCst)
    }

    /// Force fail-closed enforcement (used when a cluster mandates signed policy).
    pub fn set_enforced(&self, on: bool) {
        self.enforced.store(on, Ordering::SeqCst);
    }

    /// Replace the in-memory trust root (key-management rotation).
    pub fn set_trust_root(&self, trust: TrustRoot) {
        *self.trust.write().unwrap_or_else(|e| e.into_inner()) = trust;
    }

    /// Active bundle digest, if any.
    pub fn active_hash(&self) -> Option<String> {
        self.lock_inner().current.as_ref().map(|c| c.digest.clone())
    }

    /// Active bundle version, if any.
    pub fn active_version(&self) -> Option<String> {
        self.lock_inner()
            .current
            .as_ref()
            .map(|c| c.version.clone())
    }

    /// Prometheus text for the webhook / operator metrics endpoint.
    pub fn render_prometheus(&self) -> String {
        let (hash, version) = {
            let inner = self.lock_inner();
            (
                inner
                    .current
                    .as_ref()
                    .map(|c| c.digest.clone())
                    .unwrap_or_default(),
                inner
                    .current
                    .as_ref()
                    .map(|c| c.version.clone())
                    .unwrap_or_default(),
            )
        };
        self.metrics.render_prometheus(&hash, &version)
    }

    /// Load a signed bundle. Cryptographic failures fail closed and do not
    /// replace the last-known-good bundle.
    pub fn load_bundle(&self, bundle: &SignedPolicyBundle) -> Result<String> {
        match self.verify_and_compile(bundle, None) {
            Ok(compiled) => {
                self.activate(compiled);
                Ok(self.active_hash().unwrap_or_default())
            }
            Err(reason) => {
                self.metrics
                    .rejected_bundles
                    .fetch_add(1, Ordering::Relaxed);
                Err(Error::ValidationError(reason.to_string()))
            }
        }
    }

    /// Load from a CRD spec, honoring `trustRootRef` when present.
    pub fn load_spec(&self, spec: &StellarPolicyBundleSpec) -> Result<String> {
        let bundle = SignedPolicyBundle::from_spec(spec);
        match self.verify_and_compile(&bundle, spec.trust_root_ref.as_ref()) {
            Ok(compiled) => {
                self.activate(compiled);
                Ok(self.active_hash().unwrap_or_default())
            }
            Err(reason) => {
                self.metrics
                    .rejected_bundles
                    .fetch_add(1, Ordering::Relaxed);
                Err(Error::ValidationError(reason.to_string()))
            }
        }
    }

    /// Roll back to the previously verified bundle.
    pub fn rollback(&self) -> Result<String> {
        let mut inner = self.lock_inner();
        let previous = inner.previous.clone().ok_or_else(|| {
            Error::ValidationError("no previous verified bundle to roll back to".to_string())
        })?;
        let current = inner.current.clone();
        inner.current = Some(previous.clone());
        inner.previous = current;
        self.enforced.store(true, Ordering::SeqCst);
        self.metrics.rollbacks.fetch_add(1, Ordering::Relaxed);
        self.publish_locked(&inner);
        Ok(previous.digest.clone())
    }

    /// Admit or deny an admission request. Fail-closed on any engine error.
    pub fn admit(&self, input: &AdmissionView) -> Result<AdmitDecision> {
        let started = Instant::now();
        if let Some(decision) = self.evaluate_override(input) {
            self.metrics.record_eval(started.elapsed());
            return Ok(decision);
        }

        if !self.is_enforced() {
            let d = started.elapsed();
            self.metrics.record_eval(d);
            return Ok(AdmitDecision {
                allowed: true,
                message: None,
                warnings: vec![],
                eval_duration: d,
                bundle_hash: self.active_hash(),
                emergency_override: false,
            });
        }

        let compiled = {
            let inner = self.lock_inner();
            inner.current.clone()
        };
        let Some(compiled) = compiled else {
            let d = started.elapsed();
            self.metrics.record_eval(d);
            return Ok(AdmitDecision {
                allowed: false,
                message: Some(
                    "policy engine fail-closed: no verified signed bundle is active".to_string(),
                ),
                warnings: vec![],
                eval_duration: d,
                bundle_hash: None,
                emergency_override: false,
            });
        };

        let root = admission_root(input);
        for (name, expr) in &compiled.policies {
            match cel::eval(expr, &root) {
                Ok(true) => {}
                Ok(false) => {
                    let d = started.elapsed();
                    self.metrics.record_eval(d);
                    return Ok(AdmitDecision {
                        allowed: false,
                        message: Some(format!("denied by signed policy `{name}`")),
                        warnings: vec![],
                        eval_duration: d,
                        bundle_hash: Some(compiled.digest.clone()),
                        emergency_override: false,
                    });
                }
                Err(e) => {
                    let d = started.elapsed();
                    self.metrics.record_eval(d);
                    return Ok(AdmitDecision {
                        allowed: false,
                        message: Some(format!("policy engine fail-closed ({name}): {e}")),
                        warnings: vec![],
                        eval_duration: d,
                        bundle_hash: Some(compiled.digest.clone()),
                        emergency_override: false,
                    });
                }
            }
        }

        let d = started.elapsed();
        self.metrics.record_eval(d);
        Ok(AdmitDecision {
            allowed: true,
            message: None,
            warnings: vec![],
            eval_duration: d,
            bundle_hash: Some(compiled.digest.clone()),
            emergency_override: false,
        })
    }

    fn evaluate_override(&self, input: &AdmissionView) -> Option<AdmitDecision> {
        let anns = object_annotations(input);
        let requested = anns
            .get(annotations::EMERGENCY_OVERRIDE)
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        if !requested {
            return None;
        }
        match dual_approvers(&anns) {
            Ok(()) => {
                self.metrics
                    .emergency_overrides
                    .fetch_add(1, Ordering::Relaxed);
                Some(AdmitDecision {
                    allowed: true,
                    message: Some("emergency override granted with dual approval".to_string()),
                    warnings: vec!["emergency policy override active".to_string()],
                    eval_duration: Duration::ZERO,
                    bundle_hash: self.active_hash(),
                    emergency_override: true,
                })
            }
            Err(msg) => {
                self.metrics
                    .emergency_override_rejected
                    .fetch_add(1, Ordering::Relaxed);
                Some(AdmitDecision {
                    allowed: false,
                    message: Some(msg),
                    warnings: vec![],
                    eval_duration: Duration::ZERO,
                    bundle_hash: self.active_hash(),
                    emergency_override: false,
                })
            }
        }
    }

    fn verify_and_compile(
        &self,
        bundle: &SignedPolicyBundle,
        trust_ref: Option<&PolicyTrustRootRef>,
    ) -> std::result::Result<Arc<CompiledBundle>, BundleRejectReason> {
        if bundle.body.policies.is_empty() {
            return Err(BundleRejectReason::Malformed(
                "bundle has no policies".into(),
            ));
        }
        if bundle.body.algorithm.to_ascii_lowercase() != "ed25519" {
            return Err(BundleRejectReason::Malformed(
                "only ed25519 signatures are accepted".into(),
            ));
        }
        if bundle.body.version.is_empty()
            || bundle.body.nonce.is_empty()
            || bundle.body.key_id.is_empty()
        {
            return Err(BundleRejectReason::Malformed(
                "version, nonce, and keyId are required".into(),
            ));
        }

        let digest = bundle
            .body
            .digest()
            .map_err(|e| BundleRejectReason::Malformed(e.to_string()))?;

        {
            let mut inner = self.lock_inner();
            if let Some(cached) = inner.cache.get(&digest).cloned() {
                if inner.last_verified_digest.as_deref() == Some(digest.as_str()) {
                    self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
                    return Ok(cached);
                }
            }
        }

        if bundle.signature.trim().is_empty() {
            return Err(BundleRejectReason::Unsigned);
        }

        let trust = self.trust.read().unwrap_or_else(|e| e.into_inner());
        let pubkey = if let Some(trust_ref) = trust_ref {
            trust
                .apply_ref(trust_ref, &bundle.body.key_id)
                .map_err(|_| BundleRejectReason::IncorrectTrustRoot)?
        } else {
            trust
                .public_key(&bundle.body.key_id)
                .ok_or(BundleRejectReason::IncorrectTrustRoot)?
        };

        let payload = bundle
            .body
            .canonical_bytes()
            .map_err(|e| BundleRejectReason::Malformed(e.to_string()))?;

        if let Err(e) = verify_ed25519_signature(pubkey, &bundle.signature, &payload) {
            let msg = e.to_string();
            if msg.contains("Invalid") || msg.contains("must be") || msg.contains("base64") {
                return Err(BundleRejectReason::InvalidSignature);
            }
            return Err(BundleRejectReason::Tampered);
        }
        drop(trust);

        self.metrics
            .bundle_verifications
            .fetch_add(1, Ordering::Relaxed);

        let now = self.clock.now();
        if bundle.body.not_before
            > now + chrono::Duration::from_std(MAX_CLOCK_SKEW).unwrap_or_default()
        {
            return Err(BundleRejectReason::ClockSkewed);
        }
        if bundle.body.not_after <= now {
            return Err(BundleRejectReason::Expired);
        }

        {
            let inner = self.lock_inner();
            if let Some(prev_digest) = inner
                .seen_nonces
                .get(&(bundle.body.key_id.clone(), bundle.body.nonce.clone()))
            {
                if prev_digest != &digest {
                    return Err(BundleRejectReason::Replayed);
                }
            }
        }

        let mut compiled_policies = Vec::with_capacity(bundle.body.policies.len());
        for policy in &bundle.body.policies {
            if policy.name.is_empty() || policy.expression.is_empty() {
                return Err(BundleRejectReason::Malformed(
                    "policy name and expression are required".into(),
                ));
            }
            let expr = cel::parse(&policy.expression)
                .map_err(|e| BundleRejectReason::Malformed(e.to_string()))?;
            compiled_policies.push((policy.name.clone(), expr));
        }

        let compiled = Arc::new(CompiledBundle {
            digest: digest.clone(),
            version: bundle.body.version.clone(),
            policies: compiled_policies,
            verified_at: Instant::now(),
        });

        let mut inner = self.lock_inner();
        inner.cache.put(digest.clone(), compiled.clone());
        inner.seen_nonces.insert(
            (bundle.body.key_id.clone(), bundle.body.nonce.clone()),
            digest.clone(),
        );
        inner.last_verified_digest = Some(digest);
        Ok(compiled)
    }

    fn activate(&self, compiled: Arc<CompiledBundle>) {
        let mut inner = self.lock_inner();
        if inner.current.as_ref().map(|c| c.digest.as_str()) != Some(compiled.digest.as_str()) {
            inner.previous = inner.current.clone();
        }
        inner.current = Some(compiled);
        self.enforced.store(true, Ordering::SeqCst);
        self.publish_locked(&inner);
    }

    fn publish_locked(&self, inner: &EngineInner) {
        let mut gen = self.generation.write().unwrap_or_else(|e| e.into_inner());
        gen.0 = gen.0.saturating_add(1);
        gen.1 = inner.current.clone();
        gen.2 = Instant::now();
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, EngineInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn admission_root(input: &AdmissionView) -> serde_json::Value {
    serde_json::json!({
        "operation": input.operation,
        "namespace": input.namespace,
        "name": input.name,
        "object": input.object.clone().unwrap_or(serde_json::Value::Null),
        "user": input.username,
    })
}

fn object_annotations(input: &AdmissionView) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(obj) = &input.object {
        if let Some(map) = obj
            .get("metadata")
            .and_then(|m| m.get("annotations"))
            .and_then(|a| a.as_object())
        {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    out.insert(k.clone(), s.to_string());
                }
            }
        }
    }
    out
}

fn dual_approvers(anns: &BTreeMap<String, String>) -> std::result::Result<(), String> {
    let mut identities: HashSet<String> = HashSet::new();
    if let Some(a) = anns.get(annotations::APPROVER_1) {
        let t = a.trim();
        if !t.is_empty() {
            identities.insert(t.to_string());
        }
    }
    if let Some(b) = anns.get(annotations::APPROVER_2) {
        let t = b.trim();
        if !t.is_empty() {
            identities.insert(t.to_string());
        }
    }
    if let Some(list) = anns.get(annotations::APPROVERS) {
        for part in list.split(',') {
            let t = part.trim();
            if !t.is_empty() {
                identities.insert(t.to_string());
            }
        }
    }
    if identities.len() >= 2 {
        Ok(())
    } else {
        Err(
            "emergency override rejected: dual approval is required (approval.stellar.org/approver-1 and approver-2 must be distinct identities)"
                .to_string(),
        )
    }
}

/// Deterministic p99 of a duration sample (nearest-rank).
pub fn percentile_p99(samples: &[Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let mut sorted = samples.to_vec();
    sorted.sort();
    let idx = ((samples.len() as f64) * 0.99).ceil() as usize;
    let idx = idx.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p99_is_deterministic() {
        let samples: Vec<Duration> = (1..=100).map(|i| Duration::from_micros(i)).collect();
        assert_eq!(percentile_p99(&samples), Duration::from_micros(99));
    }
}
