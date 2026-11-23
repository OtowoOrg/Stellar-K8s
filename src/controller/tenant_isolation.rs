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
//! Runtime isolation boundaries for tenants (#1979).
//!
//! All three boundaries come from the single `Tenant` CR (`spec.network`,
//! `spec.node`, `spec.audit`), so the isolation posture is auditable from one
//! object. This module turns that spec into enforceable decisions:
//!
//! - **Node isolation**: [`verify_node_isolation`] checks a node against the
//!   tenant's selector and runtime attestation labels, and rejects nodes
//!   dedicated to another tenant.
//! - **Audit scoping**: [`authorize_audit_read`] / [`filter_audit_stream`]
//!   ensure a tenant can only read its own audit events, and only when the
//!   Tenant opted in via `audit.tenantVisible`.
//! - **Network segmentation**: [`network_policy_allows`] evaluates the
//!   generated `NetworkPolicy` so cross-tenant probes can be tested without a
//!   cluster.
//! - **RBAC**: [`tenant_rbac_manifests`] emits a namespace-scoped Role and
//!   RoleBinding that never grants `pods/exec`, `pods/attach`, or
//!   `persistentvolumes` access.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::crd::tenant::{TenantAuditScope, TenantSpec};

/// Label published by a node attestation agent, e.g.
/// `attestation.stellar.org/tpm: "2"` (level 2 attested by provider `tpm`).
pub const ATTESTATION_LABEL_PREFIX: &str = "attestation.stellar.org/";
/// Label that dedicates a node to one tenant.
pub const NODE_TENANT_LABEL: &str = "tenant.stellar.org/id";

/// What the node actually reports at runtime.
#[derive(Clone, Debug, Default)]
pub struct NodeInfo {
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IsolationViolation {
    /// The node does not carry a label required by `node.nodeSelector`.
    SelectorMismatch { key: String },
    /// The node is dedicated to a different tenant.
    DedicatedToOtherTenant { owner: String },
    /// No attestation label from the required provider.
    AttestationMissing { provider: String },
    /// Attestation exists but is below `node.attestation.minLevel`.
    AttestationLevelTooLow { have: u32, need: u32 },
}

impl std::fmt::Display for IsolationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SelectorMismatch { key } => write!(f, "node missing selector label {key}"),
            Self::DedicatedToOtherTenant { owner } => {
                write!(f, "node is dedicated to tenant {owner}")
            }
            Self::AttestationMissing { provider } => {
                write!(f, "node has no attestation from provider {provider}")
            }
            Self::AttestationLevelTooLow { have, need } => {
                write!(f, "node attestation level {have} below required {need}")
            }
        }
    }
}

/// Verify a node is acceptable for the tenant's workloads.
pub fn verify_node_isolation(spec: &TenantSpec, node: &NodeInfo) -> Result<(), IsolationViolation> {
    if let Some(owner) = node.labels.get(NODE_TENANT_LABEL) {
        if owner != &spec.tenant_id {
            return Err(IsolationViolation::DedicatedToOtherTenant {
                owner: owner.clone(),
            });
        }
    }
    let Some(iso) = spec.node.as_ref() else {
        return Ok(());
    };
    for (k, v) in &iso.node_selector {
        if node.labels.get(k) != Some(v) {
            return Err(IsolationViolation::SelectorMismatch { key: k.clone() });
        }
    }
    if let Some(att) = &iso.attestation {
        let key = format!("{ATTESTATION_LABEL_PREFIX}{}", att.provider);
        let Some(raw) = node.labels.get(&key) else {
            return Err(IsolationViolation::AttestationMissing {
                provider: att.provider.clone(),
            });
        };
        let have: u32 = raw.parse().unwrap_or(0);
        let need = att.min_level.unwrap_or(1);
        if have < need {
            return Err(IsolationViolation::AttestationLevelTooLow { have, need });
        }
    }
    Ok(())
}

/// Who is reading the audit stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditReader {
    /// A tenant principal.
    Tenant(String),
    /// The platform operator.
    Operator,
}

/// One audit record, owned by exactly one tenant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditRecord {
    pub tenant_id: String,
    pub message: String,
}

/// May `reader` read the audit stream that belongs to `stream_tenant`?
pub fn authorize_audit_read(
    reader: &AuditReader,
    stream_tenant: &str,
    scope: Option<&TenantAuditScope>,
) -> bool {
    match reader {
        AuditReader::Operator => true,
        AuditReader::Tenant(id) => {
            id == stream_tenant && scope.map(|s| s.tenant_visible).unwrap_or(false)
        }
    }
}

/// Return only the records `reader` may see. `scope_of` resolves a tenant's
/// audit scope (unknown tenants yield `None` and therefore no access).
pub fn filter_audit_stream<'a>(
    reader: &AuditReader,
    records: &'a [AuditRecord],
    scope_of: impl Fn(&str) -> Option<TenantAuditScope>,
) -> Vec<&'a AuditRecord> {
    records
        .iter()
        .filter(|r| authorize_audit_read(reader, &r.tenant_id, scope_of(&r.tenant_id).as_ref()))
        .collect()
}

/// Direction of a network probe relative to the policy's namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ingress,
    Egress,
}

/// Evaluate a generated `NetworkPolicy` manifest: is traffic with a peer in a
/// namespace carrying `peer_ns_labels` allowed? Default-deny applies once the
/// policy selects the direction.
pub fn network_policy_allows(
    policy: &Value,
    direction: Direction,
    peer_ns_labels: &BTreeMap<String, String>,
) -> bool {
    let spec = &policy["spec"];
    let (types_key, rules_key, peer_key) = match direction {
        Direction::Ingress => ("Ingress", "ingress", "from"),
        Direction::Egress => ("Egress", "egress", "to"),
    };
    let selected = spec["policyTypes"]
        .as_array()
        .map(|t| t.iter().any(|v| v == types_key))
        .unwrap_or(false);
    if !selected {
        return true;
    }
    let Some(rules) = spec[rules_key].as_array() else {
        return false;
    };
    rules.iter().any(|rule| {
        rule[peer_key]
            .as_array()
            .map(|peers| {
                peers.iter().any(|peer| {
                    peer["namespaceSelector"]["matchLabels"]
                        .as_object()
                        .map(|m| {
                            !m.is_empty()
                                && m.iter().all(|(k, v)| {
                                    peer_ns_labels.get(k).map(String::as_str) == v.as_str()
                                })
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

/// Verbs/resources a tenant principal must never hold, in any namespace.
pub const FORBIDDEN_TENANT_RESOURCES: &[&str] = &[
    "pods/exec",
    "pods/attach",
    "pods/portforward",
    "persistentvolumes",
    "nodes",
    "namespaces",
];

/// Namespace-scoped Role + RoleBinding for a tenant. The Role covers
/// day-to-day workload resources only; exec/attach/PV access is withheld so a
/// compromised tenant credential cannot step sideways.
pub fn tenant_rbac_manifests(spec: &TenantSpec) -> (Value, Value) {
    let role_name = format!("{}-tenant", spec.tenant_id);
    let role = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "Role",
        "metadata": { "name": role_name, "namespace": spec.namespace },
        "rules": [
            { "apiGroups": [""], "resources": ["pods", "pods/log", "services", "configmaps", "persistentvolumeclaims"],
              "verbs": ["get", "list", "watch"] },
            { "apiGroups": ["apps"], "resources": ["deployments", "statefulsets"],
              "verbs": ["get", "list", "watch", "create", "update", "patch"] },
            { "apiGroups": ["stellar.org"], "resources": ["stellarnodes"],
              "verbs": ["get", "list", "watch", "create", "update", "patch", "delete"] }
        ]
    });
    let binding = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "RoleBinding",
        "metadata": { "name": format!("{}-tenant", spec.tenant_id), "namespace": spec.namespace },
        "roleRef": { "apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": role_name },
        "subjects": [{ "apiGroup": "rbac.authorization.k8s.io", "kind": "Group",
                       "name": format!("tenant:{}", spec.tenant_id) }]
    });
    (role, binding)
}

/// Would the tenant's Role allow `verb` on `resource`?
pub fn role_allows(role: &Value, resource: &str, verb: &str) -> bool {
    role["rules"]
        .as_array()
        .map(|rules| {
            rules.iter().any(|r| {
                let res = r["resources"].as_array();
                let verbs = r["verbs"].as_array();
                res.map(|a| a.iter().any(|v| v == resource))
                    .unwrap_or(false)
                    && verbs.map(|a| a.iter().any(|v| v == verb)).unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
