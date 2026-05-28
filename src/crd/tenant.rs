//! Tenant CRDs (multi-tenancy support)
//!
//! This module defines the Rust-side types for tenant management CRDs.
//!
//! Note: This file is intended to be used by controllers and REST/dashboard.

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

/// TenantUsage CRD placeholder for usage/billing metrics.
///
/// This is intentionally minimal for now.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
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

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
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

