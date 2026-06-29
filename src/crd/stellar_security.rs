//! StellarSecurityPolicy Custom Resource Definition
//!
//! The StellarSecurityPolicy CRD defines comprehensive security and compliance
//! framework including automated scanning, policy enforcement, and compliance auditing.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarSecurityPolicy",
    namespaced,
    status = "StellarSecurityPolicyStatus",
    shortname = "ssp",
    printcolumn = r#"{"name":"Enabled","type":"boolean","jsonPath":".spec.enabled"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Compliance","type":"string","jsonPath":".status.complianceStatus"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarSecurityPolicySpec {
    /// Enable security policies
    #[serde(default)]
    pub enabled: bool,

    /// Pod security standards configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_security_standards: Option<PodSecurityStandardsConfig>,

    /// Network policies configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_policies: Option<NetworkPoliciesConfig>,

    /// Secret management configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_management: Option<SecretManagementConfig>,

    /// RBAC configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rbac_config: Option<RBACConfig>,

    /// Compliance frameworks to enforce
    #[serde(default)]
    pub compliance_frameworks: Vec<ComplianceFramework>,

    /// Security monitoring configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_monitoring: Option<SecurityMonitoringConfig>,

    /// Automated scanning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automated_scanning: Option<AutomatedScanningConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodSecurityStandardsConfig {
    /// Pod security standard level
    pub enforce_level: Option<PodSecurityLevel>,

    /// Audit level
    pub audit_level: Option<PodSecurityLevel>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PodSecurityLevel {
    Baseline,
    Restricted,
    Privileged,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPoliciesConfig {
    /// Enable network policies
    #[serde(default)]
    pub enabled: bool,

    /// Default deny all ingress
    #[serde(default)]
    pub default_deny_ingress: bool,

    /// Default deny all egress
    #[serde(default)]
    pub default_deny_egress: bool,

    /// Auto-generate network policies
    #[serde(default)]
    pub auto_generate: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretManagementConfig {
    /// Enable secret management
    #[serde(default)]
    pub enabled: bool,

    /// Secret provider (Vault, AWSSecretsManager, or Sealed)
    pub provider: Option<SecretProvider>,

    /// Secret rotation interval
    pub rotation_interval: Option<String>,

    /// Encryption algorithm
    pub encryption_algorithm: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum SecretProvider {
    Vault,
    AWSSecretsManager,
    Sealed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RBACConfig {
    /// Enable RBAC
    #[serde(default)]
    pub enabled: bool,

    /// OIDC/SAML authentication enabled
    #[serde(default)]
    pub external_auth_enabled: bool,

    /// MFA required
    #[serde(default)]
    pub mfa_required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum ComplianceFramework {
    SOC2,
    GDPR,
    PCI_DSS,
    HIPAA,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityMonitoringConfig {
    /// Enable real-time security monitoring
    #[serde(default)]
    pub enabled: bool,

    /// SIEM integration
    pub siem_integration: Option<String>,

    /// Real-time threat detection
    #[serde(default)]
    pub threat_detection_enabled: bool,

    /// Alert threshold
    pub alert_threshold: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedScanningConfig {
    /// Enable vulnerability scanning
    #[serde(default)]
    pub vulnerability_scan_enabled: bool,

    /// Container image scan enabled
    #[serde(default)]
    pub image_scan_enabled: bool,

    /// Dependency scan enabled
    #[serde(default)]
    pub dependency_scan_enabled: bool,

    /// Scan schedule (cron format)
    pub scan_schedule: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarSecurityPolicyStatus {
    /// Conditions of this resource
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// Compliance status
    pub compliance_status: Option<ComplianceStatus>,

    /// Last audit time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_audit_time: Option<String>,

    /// Vulnerability count
    #[serde(default)]
    pub vulnerability_count: u32,

    /// Policy violations count
    #[serde(default)]
    pub policy_violations: u32,

    /// Compliance score (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compliance_score: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum ComplianceStatus {
    Compliant,
    NonCompliant,
    Unknown,
}

impl Default for StellarSecurityPolicyStatus {
    fn default() -> Self {
        Self {
            conditions: Vec::new(),
            compliance_status: Some(ComplianceStatus::Unknown),
            last_audit_time: None,
            vulnerability_count: 0,
            policy_violations: 0,
            compliance_score: Some(0),
        }
    }
}
