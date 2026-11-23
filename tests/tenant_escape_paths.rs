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
//! Tenant escape-path suite (#1979). Runs on every release (see
//! `.github/workflows/tenant-escape-paths.yml`).
//!
//! For every ordered pair of distinct tenants (attacker -> victim) it tries
//! the four escape paths and requires each to be blocked: cross-tenant pod
//! exec, volume/PV access, audit-log read, and network access. It also checks
//! node-level isolation via runtime attestation.

use std::collections::BTreeMap;

use stellar_k8s::controller::tenant_isolation::{
    authorize_audit_read, filter_audit_stream, network_policy_allows, role_allows,
    tenant_rbac_manifests, verify_node_isolation, AuditReader, AuditRecord, Direction,
    IsolationViolation, NodeInfo, FORBIDDEN_TENANT_RESOURCES,
};
use stellar_k8s::crd::tenant::{
    TenantAuditScope, TenantNodeAttestation, TenantNodeIsolation, TenantQuotaHard, TenantSpec,
};

fn tenant(id: &str) -> TenantSpec {
    TenantSpec {
        tenant_id: id.to_string(),
        namespace: format!("ns-{id}"),
        node: Some(TenantNodeIsolation {
            node_selector: BTreeMap::from([("pool".to_string(), format!("pool-{id}"))]),
            tolerations: vec![],
            attestation: Some(TenantNodeAttestation {
                provider: "tpm".to_string(),
                min_level: Some(2),
            }),
        }),
        audit: Some(TenantAuditScope {
            policy_ref: "default".to_string(),
            destination: None,
            tenant_visible: true,
            retention_hours: 24,
        }),
        quota: TenantQuotaHard::default(),
        ..Default::default()
    }
}

fn tenants() -> Vec<TenantSpec> {
    (0..5).map(|i| tenant(&format!("t{i}"))).collect()
}

fn ns_labels(t: &TenantSpec) -> BTreeMap<String, String> {
    t.namespace_labels()
}

#[test]
fn cross_tenant_network_probes_are_blocked_100_percent() {
    let all = tenants();
    let mut probes = 0;
    for attacker in &all {
        for victim in &all {
            let policy = victim.network_policy_manifest();
            let ingress = network_policy_allows(&policy, Direction::Ingress, &ns_labels(attacker));
            let egress_from_attacker = network_policy_allows(
                &attacker.network_policy_manifest(),
                Direction::Egress,
                &ns_labels(victim),
            );
            if attacker.tenant_id == victim.tenant_id {
                assert!(
                    ingress && egress_from_attacker,
                    "same-tenant traffic must work"
                );
            } else {
                probes += 2;
                assert!(
                    !ingress,
                    "{} reached {} (ingress)",
                    attacker.tenant_id, victim.tenant_id
                );
                assert!(
                    !egress_from_attacker,
                    "{} egressed to {}",
                    attacker.tenant_id, victim.tenant_id
                );
            }
        }
    }
    assert_eq!(probes, 5 * 4 * 2);
}

#[test]
fn cross_tenant_pod_exec_and_volume_access_are_not_granted() {
    for t in tenants() {
        let (role, binding) = tenant_rbac_manifests(&t);
        // Role and binding are namespace-scoped to the tenant's own namespace.
        assert_eq!(role["kind"], "Role");
        assert_eq!(role["metadata"]["namespace"], t.namespace);
        assert_eq!(binding["kind"], "RoleBinding");
        assert_eq!(binding["metadata"]["namespace"], t.namespace);
        assert_eq!(
            binding["subjects"][0]["name"],
            format!("tenant:{}", t.tenant_id)
        );
        for resource in FORBIDDEN_TENANT_RESOURCES {
            for verb in ["get", "list", "create", "update", "patch", "delete"] {
                assert!(
                    !role_allows(&role, resource, verb),
                    "{} may {verb} {resource}",
                    t.tenant_id
                );
            }
        }
    }
}

#[test]
fn tenant_cannot_read_another_tenants_audit_stream() {
    let all = tenants();
    let records: Vec<AuditRecord> = all
        .iter()
        .map(|t| AuditRecord {
            tenant_id: t.tenant_id.clone(),
            message: format!("secret event of {}", t.tenant_id),
        })
        .collect();
    let scope_of = |id: &str| {
        all.iter()
            .find(|t| t.tenant_id == id)
            .and_then(|t| t.audit.clone())
    };

    for reader in &all {
        let who = AuditReader::Tenant(reader.tenant_id.clone());
        let visible = filter_audit_stream(&who, &records, scope_of);
        assert_eq!(
            visible.len(),
            1,
            "{} saw {} streams",
            reader.tenant_id,
            visible.len()
        );
        assert_eq!(visible[0].tenant_id, reader.tenant_id);
    }

    // The operator sees everything.
    let op = filter_audit_stream(&AuditReader::Operator, &records, scope_of);
    assert_eq!(op.len(), all.len());
}

#[test]
fn audit_stream_stays_hidden_when_tenant_visibility_is_off() {
    let mut t = tenant("t0");
    t.audit.as_mut().unwrap().tenant_visible = false;
    assert!(!authorize_audit_read(
        &AuditReader::Tenant("t0".to_string()),
        "t0",
        t.audit.as_ref()
    ));
    assert!(!authorize_audit_read(
        &AuditReader::Tenant("t0".to_string()),
        "t0",
        None
    ));
}

fn node(labels: &[(&str, &str)]) -> NodeInfo {
    NodeInfo {
        name: "n1".to_string(),
        labels: labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn node_isolation_requires_selector_and_runtime_attestation() {
    let t = tenant("t0");
    let good = node(&[("pool", "pool-t0"), ("attestation.stellar.org/tpm", "2")]);
    assert_eq!(verify_node_isolation(&t, &good), Ok(()));

    assert_eq!(
        verify_node_isolation(
            &t,
            &node(&[("pool", "shared"), ("attestation.stellar.org/tpm", "2")])
        ),
        Err(IsolationViolation::SelectorMismatch {
            key: "pool".to_string()
        })
    );
    assert_eq!(
        verify_node_isolation(&t, &node(&[("pool", "pool-t0")])),
        Err(IsolationViolation::AttestationMissing {
            provider: "tpm".to_string()
        })
    );
    assert_eq!(
        verify_node_isolation(
            &t,
            &node(&[("pool", "pool-t0"), ("attestation.stellar.org/tpm", "1")])
        ),
        Err(IsolationViolation::AttestationLevelTooLow { have: 1, need: 2 })
    );
}

#[test]
fn node_dedicated_to_another_tenant_is_rejected() {
    let t = tenant("t0");
    let n = node(&[
        ("pool", "pool-t0"),
        ("attestation.stellar.org/tpm", "2"),
        ("tenant.stellar.org/id", "t1"),
    ]);
    assert_eq!(
        verify_node_isolation(&t, &n),
        Err(IsolationViolation::DedicatedToOtherTenant {
            owner: "t1".to_string()
        })
    );
}
