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
//! Tests for the NamespaceTeardown state-machine CRD (#1499).

use super::namespace_teardown::{
    ArchiveSpec, NamespaceTeardownSpec, TeardownCondition, TeardownPhase, TeardownStepStatus,
};

fn base_spec() -> NamespaceTeardownSpec {
    NamespaceTeardownSpec {
        target_namespace: "acme-ns".to_string(),
        ..Default::default()
    }
}

fn sha256_hex() -> String {
    "a".repeat(64)
}

// ── validate() ─────────────────────────────────────────────────────────────

#[test]
fn validate_accepts_minimal_spec() {
    assert!(base_spec().validate().is_ok());
}

#[test]
fn validate_rejects_empty_target_namespace() {
    let mut spec = base_spec();
    spec.target_namespace = "  ".to_string();
    let err = spec.validate().unwrap_err();
    assert!(err.contains("targetNamespace"), "unexpected: {err}");
}

#[test]
fn validate_rejects_archive_without_destination() {
    let mut spec = base_spec();
    spec.archive = Some(ArchiveSpec {
        destination: String::new(),
        ..Default::default()
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("destination"), "unexpected: {err}");
}

#[test]
fn validate_rejects_bad_checksum_length() {
    let mut spec = base_spec();
    spec.archive = Some(ArchiveSpec {
        destination: "s3://backups/acme".to_string(),
        expected_checksum: Some("deadbeef".to_string()),
        verify: true,
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("SHA-256"), "unexpected: {err}");
}

#[test]
fn validate_rejects_non_hex_checksum() {
    let mut spec = base_spec();
    spec.archive = Some(ArchiveSpec {
        destination: "s3://backups/acme".to_string(),
        expected_checksum: Some("z".repeat(64)),
        verify: true,
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("SHA-256"), "unexpected: {err}");
}

#[test]
fn validate_rejects_verify_true_without_checksum() {
    let mut spec = base_spec();
    spec.archive = Some(ArchiveSpec {
        destination: "s3://backups/acme".to_string(),
        expected_checksum: None,
        verify: true,
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("expectedChecksum"), "unexpected: {err}");
}

#[test]
fn validate_accepts_verify_false_without_checksum() {
    let mut spec = base_spec();
    spec.archive = Some(ArchiveSpec {
        destination: "s3://backups/acme".to_string(),
        expected_checksum: None,
        verify: false,
    });
    assert!(spec.validate().is_ok());
}

#[test]
fn validate_rejects_empty_credential_ref() {
    let mut spec = base_spec();
    spec.credential_refs = vec!["acme-db-key".to_string(), "  ".to_string()];
    let err = spec.validate().unwrap_err();
    assert!(err.contains("credentialRefs"), "unexpected: {err}");
}

#[test]
fn validate_rejects_empty_dns_record() {
    let mut spec = base_spec();
    spec.dns_records = vec!["api.acme.example".to_string(), "".to_string()];
    let err = spec.validate().unwrap_err();
    assert!(err.contains("dnsRecords"), "unexpected: {err}");
}

// ── step_order() ───────────────────────────────────────────────────────────

#[test]
fn step_order_skips_optional_steps_when_absent() {
    let spec = base_spec();
    let order = spec.step_order();
    assert_eq!(
        order,
        vec![
            TeardownPhase::Pending,
            TeardownPhase::Deleting,
            TeardownPhase::Complete,
        ]
    );
}

#[test]
fn step_order_includes_all_steps_when_present() {
    let spec = NamespaceTeardownSpec {
        target_namespace: "acme-ns".to_string(),
        archive: Some(ArchiveSpec {
            destination: "s3://backups/acme".to_string(),
            expected_checksum: Some(sha256_hex()),
            verify: true,
        }),
        credential_refs: vec!["acme-db-key".to_string()],
        dns_records: vec!["api.acme.example".to_string()],
        attestation_sink: Some("attestations/acme".to_string()),
        step_timeout_seconds: 300,
    };

    let order = spec.step_order();
    assert_eq!(
        order,
        vec![
            TeardownPhase::Pending,
            TeardownPhase::Archiving,
            TeardownPhase::RevokingCredentials,
            TeardownPhase::CleaningDns,
            TeardownPhase::Attesting,
            TeardownPhase::Deleting,
            TeardownPhase::Complete,
        ]
    );
}

// ── is_terminal() ──────────────────────────────────────────────────────────

#[test]
fn is_terminal_recognises_complete_and_failed() {
    assert!(NamespaceTeardownSpec::is_terminal(&TeardownPhase::Complete));
    assert!(NamespaceTeardownSpec::is_terminal(&TeardownPhase::Failed));
    assert!(!NamespaceTeardownSpec::is_terminal(&TeardownPhase::Pending));
    assert!(!NamespaceTeardownSpec::is_terminal(&TeardownPhase::Archiving));
    assert!(!NamespaceTeardownSpec::is_terminal(&TeardownPhase::Deleting));
}

// ── serde ──────────────────────────────────────────────────────────────────

#[test]
fn phase_serialises_pascal_case() {
    let v = serde_json::to_value(TeardownPhase::RevokingCredentials).unwrap();
    assert_eq!(v, serde_json::Value::String("RevokingCredentials".to_string()));
    let v = serde_json::to_value(TeardownPhase::CleaningDns).unwrap();
    assert_eq!(v, serde_json::Value::String("CleaningDns".to_string()));
}

#[test]
fn condition_serialises_camel_case() {
    let c = TeardownCondition {
        step: "Archiving".to_string(),
        status: TeardownStepStatus::Failed,
        reason: Some("ChecksumMismatch".to_string()),
        message: Some("expected aaaa..., got bbbb...".to_string()),
        last_transition: Some("2026-01-01T00:00:00Z".to_string()),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["step"], "Archiving");
    assert_eq!(v["status"], "Failed");
    assert_eq!(v["reason"], "ChecksumMismatch");
    assert_eq!(v["lastTransition"], "2026-01-01T00:00:00Z");
}

#[test]
fn spec_round_trips_through_json() {
    let spec = NamespaceTeardownSpec {
        target_namespace: "acme-ns".to_string(),
        archive: Some(ArchiveSpec {
            destination: "s3://backups/acme".to_string(),
            expected_checksum: Some(sha256_hex()),
            verify: true,
        }),
        credential_refs: vec!["acme-db-key".to_string()],
        dns_records: vec!["api.acme.example".to_string()],
        attestation_sink: Some("attestations/acme".to_string()),
        step_timeout_seconds: 300,
    };

    let json = serde_json::to_string(&spec).unwrap();
    let back: NamespaceTeardownSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(spec, back);
}
