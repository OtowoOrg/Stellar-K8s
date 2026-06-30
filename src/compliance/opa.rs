//! OPA/Rego-style policy-as-code framework.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Rego-like expression (evaluated as JSON path checks for now)
    pub rego_expression: String,
    pub framework: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Policy {
    pub fn new(id: &str, name: &str, framework: &str, rego_expression: &str) -> Self {
        let now = Utc::now();
        Self {
            id: id.to_string(),
            name: name.to_string(),
            version: "1.0.0".to_string(),
            rego_expression: rego_expression.to_string(),
            framework: framework.to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Built-in policies for Stellar-K8s.
pub fn default_policies() -> Vec<Policy> {
    vec![
        Policy::new(
            "soc2-tls-required",
            "TLS Required for All Services",
            "SOC2",
            "spec.tls.enabled == true",
        ),
        Policy::new(
            "soc2-resource-limits",
            "Resource Limits Must Be Set",
            "SOC2",
            "spec.resources.limits.cpu != null && spec.resources.limits.memory != null",
        ),
        Policy::new(
            "iso27001-network-policy",
            "Network Policy Must Be Defined",
            "ISO27001",
            "metadata.annotations['network-policy'] != null",
        ),
        Policy::new(
            "iso27001-image-pinned",
            "Container Images Must Be Pinned",
            "ISO27001",
            "spec.version matches '^v[0-9]+\\.[0-9]+\\.[0-9]+$'",
        ),
        Policy::new(
            "pci-encryption-at-rest",
            "Encryption At Rest Required",
            "PCI-DSS",
            "spec.storage.encrypted == true",
        ),
    ]
}
