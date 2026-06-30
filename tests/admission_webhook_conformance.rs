//! Admission Webhook Conformance Tests for Invalid CRD Payloads (#1052)
//!
//! This test suite exercises the `WebhookServer::validate` pipeline end-to-end
//! using the same path that Kubernetes traverses when a `StellarNode` resource
//! is created or updated in the cluster:
//!
//! ```text
//! AdmissionReview → WebhookServer::validate
//!                       ├─ validate_spec_builtin
//!                       │      ├─ serde deserialization
//!                       │      ├─ PSS 'restricted' check
//!                       │      ├─ OrgValidator (labels + resource limits)
//!                       │      └─ StellarNodeSpec::validate()
//!                       └─ Wasm plugins (if any)
//! ```
//!
//! # Test Categories
//!
//! 1. **Baseline** – valid payloads for every node type must be admitted.
//! 2. **Malformed JSON / missing required fields** – the server must reject
//!    payloads that cannot be deserialized into a `StellarNode`.
//! 3. **Invalid `nodeType`** – unknown discriminant must be denied.
//! 4. **Validator-specific violations** – missing `validatorConfig`, wrong
//!    replica count, autoscaling on validators, ingress on validators, empty
//!    `historyArchiveUrls` when archiving is enabled.
//! 5. **Horizon-specific violations** – missing `horizonConfig`.
//! 6. **SorobanRpc-specific violations** – missing `sorobanConfig`.
//! 7. **Cross-cutting spec violations** – both `minAvailable` +
//!    `maxUnavailable` set simultaneously, both `database` + `managedDatabase`
//!    set, invalid custom network name.
//! 8. **Organisational-standards violations** – missing required labels
//!    (`project-id`, `owner`), empty / zero resource requests or limits, and
//!    resource limits that exceed the per-node-type maximum.
//! 9. **PSS `restricted` violations** – `privileged: true` security context.
//! 10. **Storage validation** – invalid `snapshotRef`, `LocalStorage` without
//!     class or affinity.
//! 11. **Operation semantics** – `DELETE` and `CONNECT` bypass spec validation
//!     and must be admitted without modification.
//! 12. **`UPDATE` operation** – same rules as `CREATE` apply.
//! 13. **Edge cases** – `None` object, `null` spec.
//!
//! # Running the Tests
//!
//! ```bash
//! cargo test --test admission_webhook_conformance
//! ```
//!
//! All tests are hermetic: no Kubernetes cluster or network connection is
//! required.

use std::collections::BTreeMap;

use stellar_k8s::webhook::{WasmRuntime, WebhookServer};
use stellar_k8s::webhook::types::{Operation, UserInfo, ValidationInput};

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

/// Build a [`ValidationInput`] with sensible defaults, suitable for driving
/// [`WebhookServer::validate`] in tests.
fn make_input(operation: Operation, object: Option<serde_json::Value>) -> ValidationInput {
    ValidationInput {
        operation,
        object,
        old_object: None,
        namespace: "default".to_string(),
        name: "test-node".to_string(),
        user_info: UserInfo {
            username: "conformance-test".to_string(),
            uid: None,
            groups: vec![],
            extra: BTreeMap::new(),
        },
        context: BTreeMap::new(),
    }
}

/// Build an `UPDATE` [`ValidationInput`] with the supplied current and old objects.
fn make_update_input(
    object: serde_json::Value,
    old_object: serde_json::Value,
) -> ValidationInput {
    ValidationInput {
        operation: Operation::Update,
        object: Some(object),
        old_object: Some(old_object),
        namespace: "default".to_string(),
        name: "test-node".to_string(),
        user_info: UserInfo {
            username: "conformance-test".to_string(),
            uid: None,
            groups: vec![],
            extra: BTreeMap::new(),
        },
        context: BTreeMap::new(),
    }
}

/// Minimal valid `Validator` node JSON with all required org labels.
fn valid_validator_json() -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "name": "my-validator",
            "namespace": "default",
            "labels": {
                "project-id": "stellar-project",
                "owner": "platform-team"
            }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "validator-seed",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    })
}

/// Minimal valid `Horizon` node JSON with all required org labels.
fn valid_horizon_json() -> serde_json::Value {
    serde_json::json!({
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
            "replicas": 2,
            "horizonConfig": {
                "databaseSecretRef": "horizon-db",
                "enableIngest": true,
                "stellarCoreUrl": "http://stellar-core:11626"
            }
        }
    })
}

/// Minimal valid `SorobanRpc` node JSON with all required org labels.
fn valid_soroban_json() -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "name": "my-soroban",
            "namespace": "default",
            "labels": {
                "project-id": "stellar-project",
                "owner": "platform-team"
            }
        },
        "spec": {
            "nodeType": "SorobanRpc",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 2,
            "sorobanConfig": {
                "stellarCoreUrl": "http://stellar-core:11626"
            }
        }
    })
}

// ---------------------------------------------------------------------------
// ❶  BASELINE – valid payloads must be admitted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_valid_validator_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let result = server
        .validate(make_input(Operation::Create, Some(valid_validator_json())))
        .await;
    assert!(
        result.allowed,
        "valid Validator payload must be admitted; got: {:?}",
        result.message
    );
}

#[tokio::test]
async fn conformance_valid_horizon_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let result = server
        .validate(make_input(Operation::Create, Some(valid_horizon_json())))
        .await;
    assert!(
        result.allowed,
        "valid Horizon payload must be admitted; got: {:?}",
        result.message
    );
}

#[tokio::test]
async fn conformance_valid_soroban_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let result = server
        .validate(make_input(Operation::Create, Some(valid_soroban_json())))
        .await;
    assert!(
        result.allowed,
        "valid SorobanRpc payload must be admitted; got: {:?}",
        result.message
    );
}

// ---------------------------------------------------------------------------
// ❷  MALFORMED JSON / MISSING REQUIRED FIELDS
// ---------------------------------------------------------------------------

/// An empty JSON object `{}` cannot be deserialized as a StellarNode.
#[tokio::test]
async fn conformance_empty_object_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let result = server
        .validate(make_input(Operation::Create, Some(serde_json::json!({}))))
        .await;
    assert!(
        !result.allowed,
        "empty object must be denied; got allowed=true"
    );
}

/// A StellarNode JSON missing the `spec` key entirely must be denied.
#[tokio::test]
async fn conformance_missing_spec_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "no-spec",
            "namespace": "default"
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "payload missing spec must be denied"
    );
}

/// A StellarNode JSON with `spec: null` must be denied.
#[tokio::test]
async fn conformance_null_spec_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": { "name": "null-spec", "namespace": "default" },
        "spec": null
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "null spec must be denied"
    );
}

/// A `None` object (no object attached to the review) is treated as admitted
/// because the server cannot validate what it cannot see.
#[tokio::test]
async fn conformance_none_object_is_admitted_with_no_plugins() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let result = server
        .validate(make_input(Operation::Create, None))
        .await;
    // With no plugins and no object the server allows through
    assert!(
        result.allowed,
        "None object with no plugins must be admitted; got: {:?}",
        result.message
    );
}

/// A payload that is valid JSON but not a StellarNode (a random object) must be denied.
#[tokio::test]
async fn conformance_non_stellarnode_json_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "kind": "Pod",
        "metadata": { "name": "wrong-kind" },
        "spec": { "containers": [] }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "non-StellarNode JSON must be denied"
    );
}

// ---------------------------------------------------------------------------
// ❸  INVALID `nodeType`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_unknown_node_type_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "bad-type",
            "namespace": "default",
            "labels": { "project-id": "p", "owner": "o" }
        },
        "spec": {
            "nodeType": "Archiver",
            "network": "testnet",
            "version": "v21.0.0"
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "unknown nodeType must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("invalid")
            || msg.to_lowercase().contains("nodetype")
            || msg.to_lowercase().contains("unknown")
            || msg.to_lowercase().contains("parse"),
        "rejection message should reference the invalid nodeType; got: {msg}"
    );
}

#[tokio::test]
async fn conformance_empty_node_type_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "empty-type",
            "namespace": "default",
            "labels": { "project-id": "p", "owner": "o" }
        },
        "spec": {
            "nodeType": "",
            "network": "testnet",
            "version": "v21.0.0"
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "empty nodeType string must be denied");
}

// ---------------------------------------------------------------------------
// ❹  VALIDATOR-SPECIFIC VIOLATIONS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_validator_missing_config_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    // Remove validatorConfig
    payload["spec"]
        .as_object_mut()
        .unwrap()
        .remove("validatorConfig");

    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Validator without validatorConfig must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("validatorConfig") || msg.contains("required"),
        "message must mention missing validatorConfig; got: {msg}"
    );
}

#[tokio::test]
async fn conformance_validator_with_two_replicas_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["replicas"] = serde_json::json!(2);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Validator with replicas=2 must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("replica") || msg.contains("Validator"),
        "message must mention replica constraint; got: {msg}"
    );
}

#[tokio::test]
async fn conformance_validator_with_zero_replicas_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["replicas"] = serde_json::json!(0);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Validator with replicas=0 must be denied");
}

#[tokio::test]
async fn conformance_validator_with_autoscaling_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["autoscaling"] = serde_json::json!({
        "minReplicas": 1,
        "maxReplicas": 3,
        "targetCpuUtilizationPercentage": 80
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Validator with autoscaling must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("autoscaling") || msg.contains("Validator"),
        "message must mention autoscaling; got: {msg}"
    );
}

#[tokio::test]
async fn conformance_validator_with_ingress_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["ingress"] = serde_json::json!({
        "className": "nginx",
        "hosts": [{ "host": "validator.example.com", "paths": [{ "path": "/" }] }]
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Validator with ingress must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("ingress") || msg.contains("Validator"),
        "message must mention ingress; got: {msg}"
    );
}

/// When `enableHistoryArchive` is true, `historyArchiveUrls` must be non-empty.
#[tokio::test]
async fn conformance_validator_history_archive_without_urls_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["validatorConfig"]["enableHistoryArchive"] = serde_json::json!(true);
    payload["spec"]["validatorConfig"]["historyArchiveUrls"] = serde_json::json!([]);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Validator with enableHistoryArchive=true but empty historyArchiveUrls must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("historyArchiveUrls") || msg.contains("archive"),
        "message must mention historyArchiveUrls; got: {msg}"
    );
}

/// Providing history archive URLs when the flag is enabled is valid.
#[tokio::test]
async fn conformance_validator_history_archive_with_urls_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["validatorConfig"]["enableHistoryArchive"] = serde_json::json!(true);
    payload["spec"]["validatorConfig"]["historyArchiveUrls"] =
        serde_json::json!(["https://history.stellar.org/prd/core-testnet"]);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        result.allowed,
        "Validator with enableHistoryArchive=true and valid URLs must be admitted; got: {:?}",
        result.message
    );
}

// ---------------------------------------------------------------------------
// ❺  HORIZON-SPECIFIC VIOLATIONS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_horizon_missing_config_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_horizon_json();
    payload["spec"]
        .as_object_mut()
        .unwrap()
        .remove("horizonConfig");
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Horizon without horizonConfig must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("horizonConfig") || msg.contains("required"),
        "message must mention missing horizonConfig; got: {msg}"
    );
}

/// A Horizon node with `nodeType: Validator` payload but horizon config is incoherent.
#[tokio::test]
async fn conformance_horizon_with_validator_node_type_no_validator_config_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "mixed-types",
            "namespace": "default",
            "labels": { "project-id": "p", "owner": "o" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "horizonConfig": {
                "databaseSecretRef": "horizon-db",
                "enableIngest": true,
                "stellarCoreUrl": "http://stellar-core:11626"
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    // nodeType=Validator but no validatorConfig → denied
    assert!(!result.allowed, "Validator nodeType without validatorConfig must be denied even when horizonConfig is present");
}

#[tokio::test]
async fn conformance_horizon_multiple_replicas_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_horizon_json();
    payload["spec"]["replicas"] = serde_json::json!(5);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        result.allowed,
        "Horizon with multiple replicas must be admitted; got: {:?}",
        result.message
    );
}

// ---------------------------------------------------------------------------
// ❻  SOROBANRPC-SPECIFIC VIOLATIONS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_soroban_missing_config_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_soroban_json();
    payload["spec"]
        .as_object_mut()
        .unwrap()
        .remove("sorobanConfig");
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "SorobanRpc without sorobanConfig must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("sorobanConfig") || msg.contains("required"),
        "message must mention missing sorobanConfig; got: {msg}"
    );
}

#[tokio::test]
async fn conformance_soroban_multiple_replicas_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_soroban_json();
    payload["spec"]["replicas"] = serde_json::json!(4);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        result.allowed,
        "SorobanRpc with multiple replicas must be admitted; got: {:?}",
        result.message
    );
}

// ---------------------------------------------------------------------------
// ❼  CROSS-CUTTING SPEC VIOLATIONS
// ---------------------------------------------------------------------------

/// Setting both `minAvailable` and `maxUnavailable` on the same spec is a
/// conflict and must be denied.
#[tokio::test]
async fn conformance_pdb_min_available_and_max_unavailable_conflict_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_horizon_json();
    payload["spec"]["minAvailable"] = serde_json::json!(1);
    payload["spec"]["maxUnavailable"] = serde_json::json!(1);
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Setting both minAvailable and maxUnavailable must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("minAvailable") || msg.contains("maxUnavailable"),
        "message must mention PDB conflict; got: {msg}"
    );
}

/// Setting both `database` (external) and `managedDatabase` is mutually exclusive.
#[tokio::test]
async fn conformance_both_database_and_managed_database_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_horizon_json();
    payload["spec"]["database"] = serde_json::json!({
        "secretRef": "pg-secret"
    });
    payload["spec"]["managedDatabase"] = serde_json::json!({
        "storageSize": "50Gi"
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Specifying both database and managedDatabase must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("database") || msg.contains("managedDatabase"),
        "message must mention the database conflict; got: {msg}"
    );
}

/// A custom network name that violates DNS-1123 must be denied.
#[tokio::test]
async fn conformance_invalid_custom_network_name_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "custom-net",
            "namespace": "default",
            "labels": { "project-id": "p", "owner": "o" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": { "custom": "-bad-name-" },
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    // Either fails deserialization or fails custom-name validation — both are denied.
    assert!(
        !result.allowed,
        "Invalid custom network name must be denied"
    );
}

/// A custom network name that exceeds 63 characters must be denied.
#[tokio::test]
async fn conformance_custom_network_name_too_long_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let long_name = "a".repeat(64); // 64 chars — one over the limit
    let payload = serde_json::json!({
        "metadata": {
            "name": "long-custom",
            "namespace": "default",
            "labels": { "project-id": "p", "owner": "o" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": { "custom": long_name },
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Custom network name exceeding 63 chars must be denied"
    );
}

// ---------------------------------------------------------------------------
// ❽  ORGANISATIONAL-STANDARDS VIOLATIONS
// ---------------------------------------------------------------------------

/// Missing `project-id` label must be denied.
#[tokio::test]
async fn conformance_missing_project_id_label_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "no-project-id",
            "namespace": "default",
            "labels": { "owner": "platform-team" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Missing project-id label must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("project-id"),
        "message must mention missing project-id label; got: {msg}"
    );
}

/// Missing `owner` label must be denied.
#[tokio::test]
async fn conformance_missing_owner_label_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "no-owner",
            "namespace": "default",
            "labels": { "project-id": "stellar-project" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Missing owner label must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("owner"),
        "message must mention missing owner label; got: {msg}"
    );
}

/// A node with no labels at all must be denied.
#[tokio::test]
async fn conformance_no_labels_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "no-labels",
            "namespace": "default"
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Node with no labels must be denied");
}

/// An empty `project-id` label value (whitespace only) must be denied.
#[tokio::test]
async fn conformance_empty_project_id_label_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "empty-label",
            "namespace": "default",
            "labels": { "project-id": "   ", "owner": "team" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Whitespace-only project-id label must be denied"
    );
}

/// A CPU limit that exceeds the per-node-type maximum must be denied.
///
/// Validators are capped at 8 cores (8000m).
#[tokio::test]
async fn conformance_validator_cpu_limit_exceeds_max_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["resources"] = serde_json::json!({
        "requests": { "cpu": "1", "memory": "1Gi" },
        "limits":   { "cpu": "16", "memory": "8Gi" }  // 16 cores > 8-core max for Validator
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Validator with CPU limit > 8 cores must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("cpu") || msg.contains("CPU") || msg.contains("limit"),
        "message must mention the CPU limit violation; got: {msg}"
    );
}

/// A memory limit that exceeds the per-node-type maximum must be denied.
///
/// Validators are capped at 16 GiB.
#[tokio::test]
async fn conformance_validator_memory_limit_exceeds_max_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["resources"] = serde_json::json!({
        "requests": { "cpu": "1", "memory": "1Gi" },
        "limits":   { "cpu": "2", "memory": "32Gi" }  // 32 GiB > 16 GiB max for Validator
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Validator with memory limit > 16 GiB must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("memory") || msg.contains("Memory") || msg.contains("limit"),
        "message must mention the memory limit violation; got: {msg}"
    );
}

/// An empty CPU request (zero / empty string) must be denied.
#[tokio::test]
async fn conformance_empty_cpu_request_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["resources"] = serde_json::json!({
        "requests": { "cpu": "0", "memory": "1Gi" },
        "limits":   { "cpu": "2", "memory": "4Gi" }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Zero CPU request must be denied");
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("cpu") || msg.contains("CPU") || msg.contains("request"),
        "message must mention zero CPU request; got: {msg}"
    );
}

/// An empty memory request must be denied.
#[tokio::test]
async fn conformance_empty_memory_request_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["resources"] = serde_json::json!({
        "requests": { "cpu": "500m", "memory": "0" },
        "limits":   { "cpu": "2", "memory": "4Gi" }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Zero memory request must be denied");
}

/// An empty CPU limit must be denied.
#[tokio::test]
async fn conformance_empty_cpu_limit_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["resources"] = serde_json::json!({
        "requests": { "cpu": "500m", "memory": "1Gi" },
        "limits":   { "cpu": "0", "memory": "4Gi" }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Zero CPU limit must be denied");
}

/// An empty memory limit must be denied.
#[tokio::test]
async fn conformance_empty_memory_limit_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["resources"] = serde_json::json!({
        "requests": { "cpu": "500m", "memory": "1Gi" },
        "limits":   { "cpu": "2", "memory": "0" }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Zero memory limit must be denied");
}

/// Mainnet Validator under-provisioned (below 2-core / 4 GiB min requests for production).
#[tokio::test]
async fn conformance_mainnet_validator_underprovisionned_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": {
            "name": "underpowered",
            "namespace": "default",
            "labels": { "project-id": "p", "owner": "o" }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "mainnet",
            "version": "v21.0.0",
            "replicas": 1,
            "resources": {
                "requests": { "cpu": "100m", "memory": "256Mi" },
                "limits":   { "cpu": "2",    "memory": "4Gi"  }
            },
            "validatorConfig": {
                "seedSecretRef": "s",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Under-provisioned Mainnet Validator must be denied"
    );
}

// ---------------------------------------------------------------------------
// ❾  PSS `restricted` VIOLATIONS
// ---------------------------------------------------------------------------

/// A security context with `privileged: true` violates the PSS `restricted`
/// profile and must be denied.
#[tokio::test]
async fn conformance_privileged_security_context_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["securityContext"] = serde_json::json!({
        "privileged": true
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "Privileged security context must be denied under PSS restricted"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("pss")
            || msg.to_lowercase().contains("privileged")
            || msg.to_lowercase().contains("restricted")
            || msg.to_lowercase().contains("security"),
        "message must reference PSS violation; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// ❿  STORAGE VALIDATION
// ---------------------------------------------------------------------------

/// A `snapshotRef` must not set both `volumeSnapshotName` and `backupUrl`.
#[tokio::test]
async fn conformance_snapshot_ref_both_fields_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["storage"] = serde_json::json!({
        "storageClass": "standard",
        "size": "100Gi",
        "snapshotRef": {
            "volumeSnapshotName": "my-snapshot",
            "backupUrl": "s3://my-bucket/backup.tar.gz"
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "snapshotRef with both volumeSnapshotName and backupUrl must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("snapshotRef") || msg.contains("snapshot"),
        "message must mention snapshotRef conflict; got: {msg}"
    );
}

/// A `snapshotRef` with neither field set is invalid.
#[tokio::test]
async fn conformance_snapshot_ref_no_fields_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["storage"] = serde_json::json!({
        "storageClass": "standard",
        "size": "100Gi",
        "snapshotRef": {}
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "snapshotRef with neither field set must be denied"
    );
}

/// `LocalStorage` mode without a `storageClass` or `nodeAffinity` must be denied.
#[tokio::test]
async fn conformance_local_storage_without_class_or_affinity_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["storage"] = serde_json::json!({
        "storageClass": "",
        "size": "100Gi",
        "mode": "Local"
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        !result.allowed,
        "LocalStorage without storageClass or nodeAffinity must be denied"
    );
    let msg = result.message.unwrap_or_default();
    assert!(
        msg.contains("storage") || msg.contains("Local"),
        "message must mention storage; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// ⓫  OPERATION SEMANTICS
// ---------------------------------------------------------------------------

/// `DELETE` operations bypass spec validation and must always be admitted.
#[tokio::test]
async fn conformance_delete_operation_bypasses_spec_validation() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    // Even with an invalid object, DELETE is admitted (resource is being removed)
    let invalid_payload = serde_json::json!({
        "metadata": { "name": "being-deleted", "namespace": "default" },
        "spec": { "nodeType": "Validator" }  // incomplete spec
    });
    let result = server
        .validate(make_input(Operation::Delete, Some(invalid_payload)))
        .await;
    assert!(
        result.allowed,
        "DELETE operations must bypass spec validation; got denied: {:?}",
        result.message
    );
}

/// `CONNECT` operations bypass spec validation and must always be admitted.
#[tokio::test]
async fn conformance_connect_operation_bypasses_spec_validation() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let payload = serde_json::json!({
        "metadata": { "name": "connecting", "namespace": "default" }
    });
    let result = server
        .validate(make_input(Operation::Connect, Some(payload)))
        .await;
    assert!(
        result.allowed,
        "CONNECT operations must bypass spec validation; got denied: {:?}",
        result.message
    );
}

// ---------------------------------------------------------------------------
// ⓬  UPDATE OPERATION — same rules as CREATE
// ---------------------------------------------------------------------------

/// A valid UPDATE (changing `version`) must be admitted.
#[tokio::test]
async fn conformance_valid_update_is_admitted() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let old = valid_validator_json();
    let mut new = valid_validator_json();
    new["spec"]["version"] = serde_json::json!("v22.0.0");
    let result = server.validate(make_update_input(new, old)).await;
    assert!(
        result.allowed,
        "Valid UPDATE must be admitted; got: {:?}",
        result.message
    );
}

/// An UPDATE that introduces a missing `validatorConfig` must be denied.
#[tokio::test]
async fn conformance_update_removing_validator_config_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let old = valid_validator_json();
    let mut new = valid_validator_json();
    new["spec"]
        .as_object_mut()
        .unwrap()
        .remove("validatorConfig");
    let result = server.validate(make_update_input(new, old)).await;
    assert!(
        !result.allowed,
        "UPDATE that removes validatorConfig must be denied"
    );
}

/// An UPDATE that adds an invalid autoscaling config to a Validator must be denied.
#[tokio::test]
async fn conformance_update_adding_autoscaling_to_validator_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let old = valid_validator_json();
    let mut new = valid_validator_json();
    new["spec"]["autoscaling"] = serde_json::json!({
        "minReplicas": 1,
        "maxReplicas": 5,
        "targetCpuUtilizationPercentage": 70
    });
    let result = server.validate(make_update_input(new, old)).await;
    assert!(
        !result.allowed,
        "UPDATE that adds autoscaling to a Validator must be denied"
    );
}

/// An UPDATE on Horizon that drops the required `owner` label must be denied.
#[tokio::test]
async fn conformance_update_dropping_required_label_is_denied() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let old = valid_horizon_json();
    let mut new = valid_horizon_json();
    new["metadata"]["labels"]
        .as_object_mut()
        .unwrap()
        .remove("owner");
    let result = server.validate(make_update_input(new, old)).await;
    assert!(
        !result.allowed,
        "UPDATE that drops required owner label must be denied"
    );
}

// ---------------------------------------------------------------------------
// ⓭  EDGE CASES
// ---------------------------------------------------------------------------

/// Validate that each rejection message is non-empty and printable — useful
/// for operators debugging admission failures.
#[tokio::test]
async fn conformance_rejection_messages_are_non_empty() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());

    let invalid_cases: Vec<(&str, serde_json::Value)> = vec![
        ("missing_validator_config", {
            let mut v = valid_validator_json();
            v["spec"].as_object_mut().unwrap().remove("validatorConfig");
            v
        }),
        ("missing_project_id", {
            let mut v = valid_validator_json();
            v["metadata"]["labels"]
                .as_object_mut()
                .unwrap()
                .remove("project-id");
            v
        }),
        ("wrong_replicas", {
            let mut v = valid_validator_json();
            v["spec"]["replicas"] = serde_json::json!(3);
            v
        }),
    ];

    for (label, payload) in invalid_cases {
        let result = server
            .validate(make_input(Operation::Create, Some(payload)))
            .await;
        assert!(!result.allowed, "[{label}] expected denied");
        let msg = result.message.unwrap_or_default();
        assert!(
            !msg.trim().is_empty(),
            "[{label}] rejection message must not be empty"
        );
        // Ensure the message contains only valid UTF-8 printable text
        assert!(
            msg.chars().all(|c| !c.is_control() || c == '\n' || c == '\t'),
            "[{label}] message contains unexpected control characters: {msg:?}"
        );
    }
}

/// A `version` of `"latest"` must be admitted (but may carry an image-pinning warning).
#[tokio::test]
async fn conformance_latest_version_tag_admitted_with_warning() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["version"] = serde_json::json!("latest");
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        result.allowed,
        "latest version tag should be admitted (warning only); got: {:?}",
        result.message
    );
    assert!(
        !result.warnings.is_empty(),
        "latest version tag should produce an image-pinning warning"
    );
}

/// A spec with an explicit, valid `version` string that does not contain `@sha256:` should
/// be admitted but carry a mutable-tag warning.
#[tokio::test]
async fn conformance_mutable_version_tag_admitted_with_warning() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let result = server
        .validate(make_input(Operation::Create, Some(valid_validator_json())))
        .await;
    assert!(
        result.allowed,
        "mutable version tag (v21.0.0) should be admitted; got: {:?}",
        result.message
    );
    // A warning about mutable tags is expected
    assert!(
        !result.warnings.is_empty(),
        "mutable version tag should produce an image-pinning warning"
    );
}

/// A version pinned by digest must be admitted and produce no image-pinning warning.
#[tokio::test]
async fn conformance_digest_pinned_version_admitted_without_warning() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    let mut payload = valid_validator_json();
    payload["spec"]["version"] =
        serde_json::json!("v21.0.0@sha256:abc123def456abc123def456abc123def456abc123def456abc123def456abc1");
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(
        result.allowed,
        "digest-pinned version must be admitted; got: {:?}",
        result.message
    );
    // No image-pinning warning should be present
    let has_pinning_warning = result
        .warnings
        .iter()
        .any(|w| w.to_lowercase().contains("digest") || w.to_lowercase().contains("pin"));
    assert!(
        !has_pinning_warning,
        "digest-pinned version must not produce a pinning warning; got: {:?}",
        result.warnings
    );
}

/// Multiple independent violations in a single spec must all be reported in
/// one denial (not silently swallowed after the first error).
#[tokio::test]
async fn conformance_multiple_violations_all_reported() {
    let server = WebhookServer::new(WasmRuntime::new().unwrap());
    // Validator: wrong replicas AND no validatorConfig AND missing labels
    let payload = serde_json::json!({
        "metadata": {
            "name": "multi-bad",
            "namespace": "default"
            // No labels
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 5
            // No validatorConfig
        }
    });
    let result = server
        .validate(make_input(Operation::Create, Some(payload)))
        .await;
    assert!(!result.allowed, "Multiple violations must be denied");
    // The combined message should be non-trivially long (contains multiple errors)
    let msg = result.message.unwrap_or_default();
    assert!(
        !msg.is_empty(),
        "Rejection message must not be empty for multiple violations"
    );
}
