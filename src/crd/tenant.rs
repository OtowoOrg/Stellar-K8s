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
//! Tenant CRDs (multi-tenancy support)
//!
//! This module defines the Rust-side types for tenant management CRDs.
//!
//! Note: This file is intended to be used by controllers and REST/dashboard.

use std::collections::BTreeMap;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-resource quota specification in Kubernetes units.
///
/// This is intentionally minimal (CPU/memory) to map cleanly into
/// `spec.hard` fields on K8s `ResourceQuota`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantQuotaHard {
    /// CPU quota (e.g. "2", "500m")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,

    /// Memory quota (e.g. "8Gi", "512Mi")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// Tenant specification for namespace isolation + quota + onboarding.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantSpec {
    /// Stable identifier for the tenant.
    pub tenant_id: String,

    /// Namespace ownership/selection.
    ///
    /// For this initial implementation, a tenant owns exactly one namespace.
    /// (Extension: allow multiple namespaces or label-based selection.)
    pub namespace: String,

    /// Optional isolation network settings.
    ///
    /// When set, tenant namespaces/pods should be labeled so NetworkPolicies
    /// can isolate traffic between tenants.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<TenantNetworkIsolation>,

    /// Optional node-pool isolation settings.
    ///
    /// When set, tenant workloads are pinned to the labelled node pool via
    /// `nodeSelector`, and scheduled onto tainted nodes via `tolerations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<TenantNodeIsolation>,

    /// Optional per-tenant audit scoping.
    ///
    /// When set, the operator wires tenant audit events into the named
    /// policy and exposes them only to the tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<TenantAuditScope>,

    /// Hard quota enforcement for the tenant namespace.
    pub quota: TenantQuotaHard,

    /// Billing / usage configuration.
    ///
    /// The operator can export aggregated usage metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing: Option<TenantBillingSpec>,

    /// When true, the operator will attempt to clean up tenant-owned
    /// isolation resources and optionally RBAC on deletion.
    #[serde(default)]
    pub cleanup_on_delete: bool,
}

/// Network isolation settings for a tenant.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantNetworkIsolation {
    /// Optional key/value used by NetworkPolicies selectors.
    ///
    /// Example: tenant.stellar.org/id = <tenant_id>
    #[serde(default = "default_tenant_label_key")]
    pub label_key: String,

    /// Namespace label value for this tenant.
    pub label_value: String,
}

fn default_tenant_label_key() -> String {
    "tenant.stellar.org/id".to_string()
}

/// Node-pool isolation settings for a tenant.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantNodeIsolation {
    /// Label selector for the node pool dedicated to this tenant.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_selector: BTreeMap<String, String>,

    /// Tolerations allowing tenant workloads onto tainted nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerations: Vec<TenantToleration>,

    /// Optional runtime attestation requirement for the node pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<TenantNodeAttestation>,
}

/// Toleration for tenant workloads on tainted nodes.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantToleration {
    pub key: String,
    pub operator: TenantTolerationOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub effect: TenantTaintEffect,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum TenantTolerationOperator {
    #[default]
    Equal,
    Exists,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum TenantTaintEffect {
    #[default]
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

/// Runtime attestation requirement for a tenant's node pool.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantNodeAttestation {
    /// Attestation provider (e.g. "tpm", "sev-snp", "nitro").
    pub provider: String,
    /// Minimum required attestation level, if the provider defines one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_level: Option<u32>,
}

/// Per-tenant audit scope.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantAuditScope {
    /// Name of the audit policy or ConfigMap reference used for this tenant.
    pub policy_ref: String,
    /// Optional log destination (e.g. a sink identifier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    /// Whether audit events are visible to the tenant itself.
    #[serde(default)]
    pub tenant_visible: bool,
    /// Retention window in hours; 0 means the cluster default applies.
    #[serde(default)]
    pub retention_hours: u32,
}

/// Billing/usage configuration for a tenant.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantBillingSpec {
    /// Billing unit selector (e.g. "cpuSeconds", "memorySeconds").
    #[serde(default)]
    pub usage_units: Vec<String>,

    /// Optional external billing integration endpoint/DSN.
    ///
    /// The operator may export usage to this endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

/// Tenant lifecycle conditions.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantCondition {
    pub type_: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

/// TenantStatus reflects onboarding/offboarding progress.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantStatus {
    /// Current lifecycle phase.
    pub phase: String,

    /// Conditions provide detailed reasons for non-ready phases.
    #[serde(default)]
    pub conditions: Vec<TenantCondition>,
}

/// Tenant CRD.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "Tenant",
    namespaced,
    status = "TenantStatus",
    shortname = "tenant"
)]
pub struct TenantSpecCrd {
    pub spec: TenantSpec,
    pub status: Option<TenantStatus>,
}

impl TenantSpecCrd {
    pub fn tenant_label_key(&self) -> &str {
        self.spec
            .network
            .as_ref()
            .map(|n| n.label_key.as_str())
            .unwrap_or("tenant.stellar.org/id")
    }

    pub fn tenant_label_value(&self) -> &str {
        self.spec
            .network
            .as_ref()
            .map(|n| n.label_value.as_str())
            .unwrap_or(self.spec.tenant_id.as_str())
    }
}

impl TenantSpec {
    /// Namespace label (key, value) that identifies this tenant's namespaces.
    pub fn network_label(&self) -> (String, String) {
        let key = self
            .network
            .as_ref()
            .map(|n| n.label_key.clone())
            .unwrap_or_else(|| "tenant.stellar.org/id".to_string());
        let value = self
            .network
            .as_ref()
            .map(|n| n.label_value.clone())
            .unwrap_or_else(|| self.tenant_id.clone());
        (key, value)
    }

    /// Build the namespace labels required for tenant-aware selectors.
    pub fn namespace_labels(&self) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        let label_key = self
            .network
            .as_ref()
            .map(|network| network.label_key.as_str())
            .unwrap_or("tenant.stellar.org/id");
        let label_value = self
            .network
            .as_ref()
            .map(|network| network.label_value.as_str())
            .unwrap_or(self.tenant_id.as_str());
        labels.insert(label_key.to_string(), label_value.to_string());
        labels.insert("stellar.org/tenant".to_string(), self.tenant_id.clone());
        labels
    }

    /// Build a ResourceQuota manifest for the tenant namespace.
    pub fn resource_quota_manifest(&self) -> serde_json::Value {
        let mut hard = serde_json::Map::new();
        if let Some(cpu) = &self.quota.cpu {
            hard.insert(
                "limits.cpu".to_string(),
                serde_json::Value::String(cpu.clone()),
            );
            hard.insert(
                "requests.cpu".to_string(),
                serde_json::Value::String(cpu.clone()),
            );
        }
        if let Some(memory) = &self.quota.memory {
            hard.insert(
                "limits.memory".to_string(),
                serde_json::Value::String(memory.clone()),
            );
            hard.insert(
                "requests.memory".to_string(),
                serde_json::Value::String(memory.clone()),
            );
        }
        serde_json::json!({
            "apiVersion": "v1",
            "kind": "ResourceQuota",
            "metadata": { "name": format!("{}-quota", self.tenant_id), "namespace": self.namespace },
            "spec": { "hard": hard }
        })
    }

    /// Build a default-deny policy that permits traffic only within a tenant.
    pub fn network_policy_manifest(&self) -> serde_json::Value {
        let label_key = self
            .network
            .as_ref()
            .map(|network| network.label_key.as_str())
            .unwrap_or("tenant.stellar.org/id");
        let label_value = self
            .network
            .as_ref()
            .map(|network| network.label_value.as_str())
            .unwrap_or(self.tenant_id.as_str());
        serde_json::json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": { "name": format!("{}-isolation", self.tenant_id), "namespace": self.namespace },
            "spec": {
                "podSelector": {},
                "policyTypes": ["Ingress", "Egress"],
                "ingress": [{ "from": [{ "namespaceSelector": { "matchLabels": { label_key: label_value } } }] }],
                "egress": [{ "to": [{ "namespaceSelector": { "matchLabels": { label_key: label_value } } }] }]
            }
        })
    }

    /// Validate the tenant spec. Returns a human-readable error on the first
    /// violation found, so callers can surface it in a status condition.
    pub fn validate(&self) -> Result<(), String> {
        if self.tenant_id.trim().is_empty() {
            return Err("tenantId must not be empty".to_string());
        }
        if self.namespace.trim().is_empty() {
            return Err("namespace must not be empty".to_string());
        }

        if let Some(node) = &self.node {
            if node.node_selector.is_empty() && node.tolerations.is_empty() {
                return Err(
                    "node isolation requires at least one of nodeSelector or tolerations"
                        .to_string(),
                );
            }
            for tol in &node.tolerations {
                if tol.key.trim().is_empty() {
                    return Err("toleration key must not be empty".to_string());
                }
                if matches!(tol.operator, TenantTolerationOperator::Equal) && tol.value.is_none() {
                    return Err(format!(
                        "toleration for key '{}' uses Equal operator but has no value",
                        tol.key
                    ));
                }
            }
            if let Some(att) = &node.attestation {
                if att.provider.trim().is_empty() {
                    return Err("node isolation attestation provider must not be empty".to_string());
                }
            }
        }

        if let Some(audit) = &self.audit {
            if audit.policy_ref.trim().is_empty() {
                return Err("audit policyRef must not be empty".to_string());
            }
        }

        Ok(())
    }

    /// Build the pod-template fragment that pins tenant workloads to their
    /// dedicated node pool. Returns `None` when node isolation is not set.
    pub fn node_placement_manifest(&self) -> Option<serde_json::Value> {
        let node = self.node.as_ref()?;
        let mut out = serde_json::Map::new();
        if !node.node_selector.is_empty() {
            out.insert(
                "nodeSelector".to_string(),
                serde_json::to_value(&node.node_selector).ok()?,
            );
        }
        if !node.tolerations.is_empty() {
            out.insert(
                "tolerations".to_string(),
                serde_json::to_value(&node.tolerations).ok()?,
            );
        }
        Some(serde_json::Value::Object(out))
    }
}

/// TenantUsage CRD placeholder for usage/billing metrics.
///
/// This is intentionally minimal for now.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TenantUsageSpec {
    pub tenant_id: String,
    pub namespace: String,
    pub window_seconds: u64,

    /// Aggregated usage in CPU-seconds / memory-bytes-seconds etc.
    /// The controller decides which units map to these fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_usage_seconds: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_usage_bytes_seconds: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantUsageStatus {
    pub phase: String,
    pub last_updated_at: Option<String>,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "TenantUsage",
    namespaced,
    status = "TenantUsageStatus",
    shortname = "tusage"
)]
pub struct TenantUsageCrd {
    pub spec: TenantUsageSpec,
    pub status: Option<TenantUsageStatus>,
}
