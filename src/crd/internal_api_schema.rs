//! Declarative internal API schema policy and pinned client deployment.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Kubernetes resource carrying deploy-time schema and generated-client pins.
///
/// The `CustomResource` derive generates the `InternalApiSchema` wrapper
/// (`metadata` + `spec` + `status`) from this spec type, matching the pattern
/// used by the other `stellar.org` CRDs in this module.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "InternalApiSchema",
    namespaced,
    status = "InternalApiSchemaStatus",
    shortname = "iaschema",
    printcolumn = r#"{"name":"Subject","type":"string","jsonPath":".spec.subject"}"#,
    printcolumn = r#"{"name":"Version","type":"integer","jsonPath":".spec.schemaVersion"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct InternalApiSchemaSpec {
    /// Stable subject key in the central registry.
    pub subject: String,
    /// Exact registry version. Consumers must never use a floating `latest` pin.
    pub schema_version: u32,
    /// Registry snapshot mounted into the operator.
    pub registry_path: String,
    /// Consumers whose deploys are gated by this policy.
    #[serde(default)]
    pub consumers: Vec<ConsumerDeploymentPolicy>,
    /// Reject deployment when impact analysis reports an unauthorized break.
    #[serde(default = "default_true")]
    pub enforce_compatibility: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerDeploymentPolicy {
    pub name: String,
    /// Generated client reference. Must remain version-pinned (no latest/main).
    pub client_reference: String,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InternalApiSchemaStatus {
    pub phase: SchemaDeploymentPhase,
    #[serde(default)]
    pub compatibility_verified: bool,
    #[serde(default)]
    /// Serialized consumer impact report (JSON).
    pub impact_report: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum SchemaDeploymentPhase {
    #[default]
    Pending,
    Verified,
    Rejected,
}
fn default_true() -> bool {
    true
}
impl InternalApiSchemaSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.subject.trim().is_empty() || self.schema_version == 0 {
            return Err("subject and non-zero schemaVersion are required".into());
        }
        if self.registry_path.trim().is_empty() {
            return Err("registryPath is required".into());
        }
        for c in &self.consumers {
            if c.client_reference.contains(":latest") || c.client_reference.ends_with("/main") {
                return Err(format!("clientReference for {} must be pinned", c.name));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_floating_client() {
        let mut s = InternalApiSchemaSpec {
            subject: "events".into(),
            schema_version: 1,
            registry_path: "registry.json".into(),
            consumers: vec![ConsumerDeploymentPolicy {
                name: "h".into(),
                client_reference: "h:latest".into(),
            }],
            enforce_compatibility: true,
        };
        assert!(s.validate().is_err());
        s.consumers[0].client_reference = "h@v1".into();
        assert!(s.validate().is_ok());
    }
}
