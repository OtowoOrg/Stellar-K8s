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
//! Tests for the node- and audit-isolation surface added to `TenantSpec`.

use super::tenant::{
    TenantAuditScope, TenantNodeAttestation, TenantNodeIsolation, TenantQuotaHard, TenantSpec,
    TenantTaintEffect, TenantToleration, TenantTolerationOperator,
};
use std::collections::BTreeMap;

fn base_spec() -> TenantSpec {
    TenantSpec {
        tenant_id: "acme".to_string(),
        namespace: "acme-ns".to_string(),
        quota: TenantQuotaHard {
            cpu: Some("2".to_string()),
            memory: Some("4Gi".to_string()),
        },
        ..Default::default()
    }
}

#[test]
fn validate_accepts_minimal_spec() {
    assert!(base_spec().validate().is_ok());
}

#[test]
fn validate_rejects_empty_tenant_id() {
    let mut spec = base_spec();
    spec.tenant_id = "   ".to_string();
    let err = spec.validate().unwrap_err();
    assert!(err.contains("tenantId"), "unexpected error: {err}");
}

#[test]
fn validate_rejects_empty_namespace() {
    let mut spec = base_spec();
    spec.namespace = String::new();
    let err = spec.validate().unwrap_err();
    assert!(err.contains("namespace"), "unexpected error: {err}");
}

#[test]
fn validate_rejects_empty_node_isolation() {
    let mut spec = base_spec();
    spec.node = Some(TenantNodeIsolation::default());
    let err = spec.validate().unwrap_err();
    assert!(
        err.contains("nodeSelector") || err.contains("tolerations"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_accepts_node_selector_only() {
    let mut spec = base_spec();
    let mut selector = BTreeMap::new();
    selector.insert("dedicated".to_string(), "tenant-acme".to_string());
    spec.node = Some(TenantNodeIsolation {
        node_selector: selector,
        ..Default::default()
    });
    assert!(spec.validate().is_ok());
}

#[test]
fn validate_accepts_tolerations_only() {
    let mut spec = base_spec();
    spec.node = Some(TenantNodeIsolation {
        tolerations: vec![TenantToleration {
            key: "dedicated".to_string(),
            operator: TenantTolerationOperator::Exists,
            value: None,
            effect: TenantTaintEffect::NoSchedule,
        }],
        ..Default::default()
    });
    assert!(spec.validate().is_ok());
}

#[test]
fn validate_rejects_equal_toleration_without_value() {
    let mut spec = base_spec();
    spec.node = Some(TenantNodeIsolation {
        tolerations: vec![TenantToleration {
            key: "dedicated".to_string(),
            operator: TenantTolerationOperator::Equal,
            value: None,
            effect: TenantTaintEffect::NoSchedule,
        }],
        ..Default::default()
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("Equal"), "unexpected error: {err}");
}

#[test]
fn validate_rejects_empty_toleration_key() {
    let mut spec = base_spec();
    spec.node = Some(TenantNodeIsolation {
        tolerations: vec![TenantToleration {
            key: String::new(),
            operator: TenantTolerationOperator::Exists,
            value: None,
            effect: TenantTaintEffect::NoSchedule,
        }],
        ..Default::default()
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("toleration key"), "unexpected error: {err}");
}

#[test]
fn validate_rejects_empty_attestation_provider() {
    let mut spec = base_spec();
    let mut selector = BTreeMap::new();
    selector.insert("dedicated".to_string(), "tenant-acme".to_string());
    spec.node = Some(TenantNodeIsolation {
        node_selector: selector,
        attestation: Some(TenantNodeAttestation {
            provider: "  ".to_string(),
            min_level: Some(2),
        }),
        ..Default::default()
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("attestation"), "unexpected error: {err}");
}

#[test]
fn validate_rejects_empty_audit_policy_ref() {
    let mut spec = base_spec();
    spec.audit = Some(TenantAuditScope {
        policy_ref: "".to_string(),
        ..Default::default()
    });
    let err = spec.validate().unwrap_err();
    assert!(err.contains("policyRef"), "unexpected error: {err}");
}

#[test]
fn validate_accepts_full_audit_scope() {
    let mut spec = base_spec();
    spec.audit = Some(TenantAuditScope {
        policy_ref: "acme-audit".to_string(),
        destination: Some("s3://audit/acme".to_string()),
        tenant_visible: true,
        retention_hours: 720,
    });
    assert!(spec.validate().is_ok());
}

#[test]
fn node_placement_manifest_is_none_without_node() {
    assert!(base_spec().node_placement_manifest().is_none());
}

#[test]
fn node_placement_manifest_includes_selector_and_tolerations() {
    let mut selector = BTreeMap::new();
    selector.insert("dedicated".to_string(), "tenant-acme".to_string());

    let mut spec = base_spec();
    spec.node = Some(TenantNodeIsolation {
        node_selector: selector,
        tolerations: vec![TenantToleration {
            key: "dedicated".to_string(),
            operator: TenantTolerationOperator::Equal,
            value: Some("tenant-acme".to_string()),
            effect: TenantTaintEffect::NoSchedule,
        }],
        ..Default::default()
    });

    let manifest = spec.node_placement_manifest().expect("manifest");
    assert!(manifest.get("nodeSelector").is_some());
    assert!(manifest.get("tolerations").is_some());
}

#[test]
fn audit_scope_serialises_camel_case() {
    let scope = TenantAuditScope {
        policy_ref: "acme-audit".to_string(),
        destination: None,
        tenant_visible: true,
        retention_hours: 24,
    };
    let json = serde_json::to_value(&scope).unwrap();
    assert_eq!(json["policyRef"], "acme-audit");
    assert_eq!(json["tenantVisible"], true);
    assert_eq!(json["retentionHours"], 24);
}

#[test]
fn toleration_operator_uses_pascal_case_values() {
    let v = serde_json::to_value(TenantTolerationOperator::Equal).unwrap();
    assert_eq!(v, serde_json::Value::String("Equal".to_string()));
    let v = serde_json::to_value(TenantTolerationOperator::Exists).unwrap();
    assert_eq!(v, serde_json::Value::String("Exists".to_string()));
}

#[test]
fn full_spec_round_trips_through_json() {
    let mut selector = BTreeMap::new();
    selector.insert("dedicated".to_string(), "tenant-acme".to_string());

    let mut spec = base_spec();
    spec.node = Some(TenantNodeIsolation {
        node_selector: selector,
        tolerations: vec![TenantToleration {
            key: "dedicated".to_string(),
            operator: TenantTolerationOperator::Equal,
            value: Some("tenant-acme".to_string()),
            effect: TenantTaintEffect::NoExecute,
        }],
        attestation: Some(TenantNodeAttestation {
            provider: "tpm".to_string(),
            min_level: Some(2),
        }),
    });
    spec.audit = Some(TenantAuditScope {
        policy_ref: "acme-audit".to_string(),
        destination: Some("s3://audit/acme".to_string()),
        tenant_visible: true,
        retention_hours: 720,
    });

    let json = serde_json::to_string(&spec).unwrap();
    let back: TenantSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(spec, back);
}
