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
//! Adversarial and performance tests for the signed CEL policy engine (#1483).

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use stellar_k8s::controller::reconcile_policy_bundle;
use stellar_k8s::crd::secret_policy::KmsProvider;
use stellar_k8s::crd::stellar_policy_bundle::annotations;
use stellar_k8s::crd::{CelPolicySpec, PolicyTrustRootRef, StellarPolicyBundleSpec};
use stellar_k8s::policy_engine::{
    percentile_p99, AdmissionView, CanonicalBundle, FixedClock, PolicyEngine, SignedPolicyBundle,
    TrustRoot, EVAL_P99_SLO, PROPAGATION_SLO,
};
use stellar_k8s::webhook::types::{Operation, UserInfo, ValidationInput};
use stellar_k8s::webhook::{WasmRuntime, WebhookServer};

const FIXED_NOW: &str = "2026-09-25T22:00:00Z";

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(FIXED_NOW)
        .expect("fixed now")
        .with_timezone(&Utc)
}

fn keypair() -> (SigningKey, String) {
    let sk = SigningKey::generate(&mut OsRng);
    let pk = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        sk.verifying_key().as_bytes(),
    );
    (sk, pk)
}

fn trust(key_id: &str, pk: &str) -> TrustRoot {
    let mut t = TrustRoot::empty();
    t.insert(key_id, pk);
    t
}

fn body(version: &str, nonce: &str, key_id: &str, expr: &str) -> CanonicalBundle {
    let now = fixed_now();
    CanonicalBundle {
        algorithm: "ed25519".to_string(),
        key_id: key_id.to_string(),
        nonce: nonce.to_string(),
        not_before: now - chrono::Duration::minutes(1),
        not_after: now + chrono::Duration::hours(1),
        policies: vec![CelPolicySpec {
            name: "allow-testnet".to_string(),
            expression: expr.to_string(),
        }],
        version: version.to_string(),
    }
}

fn engine_with(trust: TrustRoot) -> PolicyEngine {
    PolicyEngine::with_clock(trust, Arc::new(FixedClock(fixed_now())))
}

fn view_ok() -> AdmissionView {
    AdmissionView {
        operation: "CREATE".to_string(),
        namespace: "stellar".to_string(),
        name: "node-a".to_string(),
        username: "tester".to_string(),
        object: Some(serde_json::json!({
            "apiVersion": "stellar.org/v1alpha1",
            "kind": "StellarNode",
            "metadata": { "name": "node-a", "namespace": "stellar", "annotations": {} },
            "spec": { "network": "Testnet", "nodeType": "Validator" }
        })),
    }
}

fn deny_view() -> AdmissionView {
    let mut v = view_ok();
    v.object = Some(serde_json::json!({
        "metadata": { "name": "node-a", "annotations": {} },
        "spec": { "network": "Mainnet", "nodeType": "Validator" }
    }));
    v
}

#[test]
fn valid_signed_bundle_loads_and_admits() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let signed = SignedPolicyBundle::sign(
        body("v1", "n1", "root-1", r#"object.spec.network == "Testnet""#),
        &sk,
    )
    .unwrap();
    let hash = engine.load_bundle(&signed).unwrap();
    assert!(!hash.is_empty());
    assert_eq!(engine.active_hash().as_deref(), Some(hash.as_str()));
    assert!(engine.admit(&view_ok()).unwrap().allowed);
    assert!(!engine.admit(&deny_view()).unwrap().allowed);
}

#[test]
fn unsigned_bundle_rejected_fail_closed() {
    let (_sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let mut unsigned = SignedPolicyBundle {
        body: body("v1", "n-unsigned", "root-1", "true"),
        signature: String::new(),
    };
    unsigned.signature.clear();
    let err = engine.load_bundle(&unsigned).unwrap_err().to_string();
    assert!(err.contains("unsigned"), "{err}");
    engine.set_enforced(true);
    assert!(!engine.admit(&view_ok()).unwrap().allowed);
}

#[test]
fn tampered_bundle_rejected() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let mut signed =
        SignedPolicyBundle::sign(body("v1", "n-tamper", "root-1", "true"), &sk).unwrap();
    signed.body.policies[0].expression = "false".to_string();
    let err = engine.load_bundle(&signed).unwrap_err().to_string();
    assert!(
        err.contains("tampered") || err.contains("invalid") || err.contains("signature"),
        "{err}"
    );
}

#[test]
fn invalid_signature_rejected() {
    let (_sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let signed = SignedPolicyBundle {
        body: body("v1", "n-badsig", "root-1", "true"),
        signature: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0u8; 64]),
    };
    let err = engine.load_bundle(&signed).unwrap_err().to_string();
    assert!(
        err.contains("invalid") || err.contains("tampered") || err.contains("signature"),
        "{err}"
    );
}

#[test]
fn expired_bundle_rejected() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let mut b = body("v1", "n-exp", "root-1", "true");
    b.not_after = fixed_now() - chrono::Duration::seconds(1);
    let signed = SignedPolicyBundle::sign(b, &sk).unwrap();
    let err = engine.load_bundle(&signed).unwrap_err().to_string();
    assert!(err.contains("expired"), "{err}");
}

#[test]
fn clock_skewed_bundle_rejected() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let mut b = body("v1", "n-skew", "root-1", "true");
    b.not_before = fixed_now() + chrono::Duration::hours(2);
    b.not_after = fixed_now() + chrono::Duration::hours(3);
    let signed = SignedPolicyBundle::sign(b, &sk).unwrap();
    let err = engine.load_bundle(&signed).unwrap_err().to_string();
    assert!(err.contains("clock-skewed"), "{err}");
}

#[test]
fn replayed_bundle_rejected() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let first = SignedPolicyBundle::sign(body("v1", "same-nonce", "root-1", "true"), &sk).unwrap();
    engine.load_bundle(&first).unwrap();
    let second = SignedPolicyBundle::sign(
        body(
            "v2",
            "same-nonce",
            "root-1",
            r#"object.spec.network == "Testnet""#,
        ),
        &sk,
    )
    .unwrap();
    let err = engine.load_bundle(&second).unwrap_err().to_string();
    assert!(err.contains("replayed"), "{err}");
}

#[test]
fn incorrect_trust_root_rejected() {
    let (sk, _pk) = keypair();
    let (_other, other_pk) = keypair();
    let engine = engine_with(trust("other-root", &other_pk));
    let signed = SignedPolicyBundle::sign(body("v1", "n-wrong", "root-1", "true"), &sk).unwrap();
    let err = engine.load_bundle(&signed).unwrap_err().to_string();
    assert!(
        err.contains("trust root") || err.contains("incorrect"),
        "{err}"
    );
}

#[test]
fn malformed_bundle_rejected() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let mut b = body("v1", "n-mal", "root-1", "this is not cel ===");
    b.policies[0].expression = "???".to_string();
    let signed = SignedPolicyBundle::sign(b, &sk).unwrap();
    let err = engine.load_bundle(&signed).unwrap_err().to_string();
    assert!(err.contains("malformed"), "{err}");
}

#[test]
fn cache_hit_skips_reverify_same_hash() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let signed = SignedPolicyBundle::sign(body("v1", "n-cache", "root-1", "true"), &sk).unwrap();
    engine.load_bundle(&signed).unwrap();
    let verifies = engine
        .metrics
        .bundle_verifications
        .load(std::sync::atomic::Ordering::Relaxed);
    engine.load_bundle(&signed).unwrap();
    let verifies_after = engine
        .metrics
        .bundle_verifications
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(verifies, verifies_after);
    assert!(
        engine
            .metrics
            .cache_hits
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
    );
}

#[test]
fn digest_change_re_verifies() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let first = SignedPolicyBundle::sign(body("v1", "n-a", "root-1", "true"), &sk).unwrap();
    engine.load_bundle(&first).unwrap();
    let verifies = engine
        .metrics
        .bundle_verifications
        .load(std::sync::atomic::Ordering::Relaxed);
    let second = SignedPolicyBundle::sign(body("v2", "n-b", "root-1", "true"), &sk).unwrap();
    engine.load_bundle(&second).unwrap();
    let verifies_after = engine
        .metrics
        .bundle_verifications
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(verifies_after > verifies);
}

#[test]
fn rollback_restores_previous_bundle() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let v1 = SignedPolicyBundle::sign(body("v1", "n-r1", "root-1", "true"), &sk).unwrap();
    let h1 = engine.load_bundle(&v1).unwrap();
    let v2 = SignedPolicyBundle::sign(
        body(
            "v2",
            "n-r2",
            "root-1",
            r#"object.spec.network == "Testnet""#,
        ),
        &sk,
    )
    .unwrap();
    let h2 = engine.load_bundle(&v2).unwrap();
    assert_ne!(h1, h2);
    let rolled = engine.rollback().unwrap();
    assert_eq!(rolled, h1);
    assert_eq!(engine.active_hash().as_deref(), Some(h1.as_str()));
    assert!(
        engine
            .metrics
            .rollbacks
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
    );
}

#[test]
fn emergency_override_without_dual_approval_rejected() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let signed = SignedPolicyBundle::sign(
        body(
            "v1",
            "n-ov1",
            "root-1",
            r#"object.spec.network == "Testnet""#,
        ),
        &sk,
    )
    .unwrap();
    engine.load_bundle(&signed).unwrap();
    let mut view = deny_view();
    view.object = Some(serde_json::json!({
        "metadata": {
            "annotations": {
                annotations::EMERGENCY_OVERRIDE: "true",
                annotations::APPROVER_1: "alice"
            }
        },
        "spec": { "network": "Mainnet" }
    }));
    let decision = engine.admit(&view).unwrap();
    assert!(!decision.allowed);
    assert!(decision.message.unwrap().contains("dual approval"));
}

#[test]
fn emergency_override_with_dual_approval_succeeds() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let signed = SignedPolicyBundle::sign(
        body(
            "v1",
            "n-ov2",
            "root-1",
            r#"object.spec.network == "Testnet""#,
        ),
        &sk,
    )
    .unwrap();
    engine.load_bundle(&signed).unwrap();
    let mut view = deny_view();
    view.object = Some(serde_json::json!({
        "metadata": {
            "annotations": {
                annotations::EMERGENCY_OVERRIDE: "true",
                annotations::APPROVER_1: "alice",
                annotations::APPROVER_2: "bob"
            }
        },
        "spec": { "network": "Mainnet" }
    }));
    let decision = engine.admit(&view).unwrap();
    assert!(decision.allowed);
    assert!(decision.emergency_override);
}

#[test]
fn kms_trust_root_ref_must_match_inventory() {
    let (sk, pk) = keypair();
    let mut root = trust("root-1", &pk);
    root.bind_kms("root-1", KmsProvider::Aws, "kms-alias/policy");
    let engine = engine_with(root);
    let signed = SignedPolicyBundle::sign(body("v1", "n-kms", "root-1", "true"), &sk).unwrap();
    let spec = StellarPolicyBundleSpec {
        version: signed.body.version.clone(),
        policies: signed.body.policies.clone(),
        not_before: signed.body.not_before,
        not_after: signed.body.not_after,
        nonce: signed.body.nonce.clone(),
        key_id: signed.body.key_id.clone(),
        algorithm: signed.body.algorithm.clone(),
        signature: signed.signature.clone(),
        trust_root_ref: Some(PolicyTrustRootRef {
            secret_name: Some("policy-trust".to_string()),
            secret_key: Some("publicKey".to_string()),
            kms_provider: Some(KmsProvider::Aws),
            kms_key_id: Some("kms-alias/policy".to_string()),
        }),
        rollback_target: None,
    };
    assert!(reconcile_policy_bundle(&engine, &spec).is_ok());

    let mut bad = spec.clone();
    bad.nonce = "n-kms-bad".to_string();
    bad.trust_root_ref.as_mut().unwrap().kms_key_id = Some("wrong-key".to_string());
    let resign = SignedPolicyBundle::sign(
        CanonicalBundle {
            algorithm: bad.algorithm.clone(),
            key_id: bad.key_id.clone(),
            nonce: bad.nonce.clone(),
            not_after: bad.not_after,
            not_before: bad.not_before,
            policies: bad.policies.clone(),
            version: "v1-bad".to_string(),
        },
        &sk,
    )
    .unwrap();
    bad.signature = resign.signature;
    bad.version = resign.body.version;
    assert!(reconcile_policy_bundle(&engine, &bad).is_err());
}

#[test]
fn reconcile_rollback_target_uses_previous_bundle() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let v1 = SignedPolicyBundle::sign(body("v1", "n-cr1", "root-1", "true"), &sk).unwrap();
    let spec1 = StellarPolicyBundleSpec {
        version: v1.body.version.clone(),
        policies: v1.body.policies.clone(),
        not_before: v1.body.not_before,
        not_after: v1.body.not_after,
        nonce: v1.body.nonce.clone(),
        key_id: v1.body.key_id.clone(),
        algorithm: v1.body.algorithm.clone(),
        signature: v1.signature.clone(),
        trust_root_ref: None,
        rollback_target: None,
    };
    let h1 = reconcile_policy_bundle(&engine, &spec1).unwrap();
    let v2 = SignedPolicyBundle::sign(body("v2", "n-cr2", "root-1", "true"), &sk).unwrap();
    let spec2 = StellarPolicyBundleSpec {
        version: v2.body.version.clone(),
        policies: v2.body.policies.clone(),
        not_before: v2.body.not_before,
        not_after: v2.body.not_after,
        nonce: v2.body.nonce.clone(),
        key_id: v2.body.key_id.clone(),
        algorithm: v2.body.algorithm.clone(),
        signature: v2.signature.clone(),
        trust_root_ref: None,
        rollback_target: None,
    };
    reconcile_policy_bundle(&engine, &spec2).unwrap();
    let mut rb = spec2;
    rb.rollback_target = Some(h1.clone());
    let rolled = reconcile_policy_bundle(&engine, &rb).unwrap();
    assert_eq!(rolled, h1);
}

#[test]
fn webhook_denies_when_signed_policy_fails() {
    let (sk, pk) = keypair();
    let engine = Arc::new(engine_with(trust("root-1", &pk)));
    let signed = SignedPolicyBundle::sign(
        body(
            "v1",
            "n-wh",
            "root-1",
            r#"object.spec.network == "Testnet""#,
        ),
        &sk,
    )
    .unwrap();
    engine.load_bundle(&signed).unwrap();
    let server = WebhookServer::new(WasmRuntime::new().unwrap()).with_policy_engine(engine);
    let input = ValidationInput {
        operation: Operation::Create,
        object: Some(serde_json::json!({
            "metadata": {
                "name": "my-horizon",
                "namespace": "default",
                "labels": {
                    "project-id": "stellar-project",
                    "owner": "platform-team"
                }
            },
            "spec": {
                "nodeType": "Horizon",
                "network": "testnet",
                "version": "v21.0.0",
                "replicas": 1,
                "replicas": 2,
                "horizonConfig": {
                    "databaseSecretRef": "horizon-db",
                    "enableIngest": true,
                    "stellarCoreUrl": "http://stellar-core:11626"
                }
            }
        })),
        old_object: None,
        namespace: "default".to_string(),
        name: "my-horizon".to_string(),
        user_info: UserInfo {
            username: "t".to_string(),
            uid: None,
            groups: vec![],
            extra: std::collections::BTreeMap::new(),
        },
        context: std::collections::BTreeMap::new(),
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(server.validate(input));
    assert!(!result.allowed, "{:?}", result.message);
}

#[test]
fn propagation_across_replicas_within_slo() {
    let (sk, pk) = keypair();
    let publisher = engine_with(trust("root-1", &pk));
    let bus = publisher.share_generation();
    let replicas: Vec<PolicyEngine> = (0..8).map(|_| engine_with(trust("root-1", &pk))).collect();
    let signed = SignedPolicyBundle::sign(body("v9", "n-prop", "root-1", "true"), &sk).unwrap();
    let start = Instant::now();
    let hash = publisher.load_bundle(&signed).unwrap();
    let mut lags = Vec::new();
    for replica in &replicas {
        let mut seen = None;
        let deadline = Instant::now() + PROPAGATION_SLO;
        while Instant::now() < deadline {
            if let Some(lag) = replica.sync_from(&bus) {
                if replica.active_hash().as_deref() == Some(hash.as_str()) {
                    seen = Some(lag);
                    break;
                }
            }
        }
        lags.push(seen.expect("replica did not observe bundle within SLO"));
    }
    let worst = start.elapsed();
    assert!(
        worst < PROPAGATION_SLO,
        "cluster-wide propagation {worst:?} exceeded {:?}",
        PROPAGATION_SLO
    );
    assert!(lags.iter().all(|l| *l < PROPAGATION_SLO));
}

#[test]
fn eval_p99_under_5ms_with_500_policies() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let now = fixed_now();
    let policies: Vec<CelPolicySpec> = (0..500)
        .map(|i| CelPolicySpec {
            name: format!("p{i}"),
            expression: format!(r#"object.metadata.name != "deny-{i}""#),
        })
        .collect();
    let canonical = CanonicalBundle {
        algorithm: "ed25519".to_string(),
        key_id: "root-1".to_string(),
        nonce: "n-perf".to_string(),
        not_before: now - chrono::Duration::minutes(1),
        not_after: now + chrono::Duration::hours(1),
        policies,
        version: "perf".to_string(),
    };
    let signed = SignedPolicyBundle::sign(canonical, &sk).unwrap();
    engine.load_bundle(&signed).unwrap();

    let view = view_ok();
    let mut samples = Vec::with_capacity(400);
    for _ in 0..50 {
        let _ = engine.admit(&view).unwrap();
    }
    for _ in 0..400 {
        let d = engine.admit(&view).unwrap().eval_duration;
        samples.push(d);
    }
    let p99 = percentile_p99(&samples);
    eprintln!(
        "policy engine p99={:?} (target {:?}) n=400 policies=500",
        p99, EVAL_P99_SLO
    );
    assert!(
        p99 < EVAL_P99_SLO,
        "p99 evaluation latency {p99:?} exceeded {:?}",
        EVAL_P99_SLO
    );
}

#[test]
fn metrics_expose_required_series() {
    let (sk, pk) = keypair();
    let engine = engine_with(trust("root-1", &pk));
    let signed = SignedPolicyBundle::sign(body("v1", "n-met", "root-1", "true"), &sk).unwrap();
    engine.load_bundle(&signed).unwrap();
    let _ = engine.admit(&view_ok()).unwrap();
    let text = engine.render_prometheus();
    for needle in [
        "stellar_policy_eval_latency_ns",
        "stellar_policy_bundle_verifications_total",
        "stellar_policy_rejected_bundles_total",
        "stellar_policy_active_bundle_info",
        "stellar_policy_rollbacks_total",
        "stellar_policy_emergency_overrides_total",
        "stellar_policy_propagation_latency_ns_total",
    ] {
        assert!(text.contains(needle), "missing {needle} in:\n{text}");
    }
}
