//! Compliance framework definitions and validation pipelines.

use serde::{Deserialize, Serialize};

/// Supported regulatory compliance frameworks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComplianceFramework {
    Soc2,
    Gdpr,
    PciDss,
}

impl ComplianceFramework {
    pub fn all() -> Vec<Self> {
        vec![Self::Soc2, Self::Gdpr, Self::PciDss]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Soc2 => "SOC 2 Type II",
            Self::Gdpr => "GDPR",
            Self::PciDss => "PCI-DSS v4.0",
        }
    }
}

/// A single compliance rule within a framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceRule {
    pub id: String,
    pub framework: ComplianceFramework,
    pub title: String,
    pub description: String,
    pub severity: RuleSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum RuleSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// Result of evaluating a single compliance rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleResult {
    pub rule: ComplianceRule,
    pub passed: bool,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// Cluster state snapshot used for compliance validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterComplianceState {
    pub mtl_enabled: bool,
    pub audit_logging_enabled: bool,
    pub pss_restricted_enforced: bool,
    pub secrets_encrypted_at_rest: bool,
    pub network_policies_enabled: bool,
    pub rbac_enabled: bool,
    pub data_retention_days: u32,
    pub pii_scrubbing_enabled: bool,
    pub encryption_in_transit: bool,
    pub access_logging_enabled: bool,
    pub vulnerability_scan_enabled: bool,
    pub backup_enabled: bool,
    pub dr_tested_within_90_days: bool,
}

/// Validation pipeline that evaluates all rules for a framework.
pub struct ValidationPipeline;

impl ValidationPipeline {
    pub fn rules_for(framework: ComplianceFramework) -> Vec<ComplianceRule> {
        match framework {
            ComplianceFramework::Soc2 => Self::soc2_rules(),
            ComplianceFramework::Gdpr => Self::gdpr_rules(),
            ComplianceFramework::PciDss => Self::pci_dss_rules(),
        }
    }

    pub fn validate(framework: ComplianceFramework, state: &ClusterComplianceState) -> Vec<RuleResult> {
        Self::rules_for(framework)
            .into_iter()
            .map(|rule| Self::evaluate_rule(&rule, state))
            .collect()
    }

    pub fn validate_all(state: &ClusterComplianceState) -> Vec<RuleResult> {
        ComplianceFramework::all()
            .into_iter()
            .flat_map(|f| Self::validate(f, state))
            .collect()
    }

    fn evaluate_rule(rule: &ComplianceRule, state: &ClusterComplianceState) -> RuleResult {
        let (passed, evidence, remediation) = match rule.id.as_str() {
            // SOC2
            "SOC2-CC6.1" => (
                state.rbac_enabled,
                format!("RBAC enabled: {}", state.rbac_enabled),
                Some("Enable Kubernetes RBAC for all operator API endpoints".to_string()),
            ),
            "SOC2-CC6.6" => (
                state.mtl_enabled,
                format!("mTLS enabled: {}", state.mtl_enabled),
                Some("Enable mTLS for operator REST API".to_string()),
            ),
            "SOC2-CC7.2" => (
                state.audit_logging_enabled,
                format!("Audit logging: {}", state.audit_logging_enabled),
                None,
            ),
            // GDPR
            "GDPR-Art32" => (
                state.secrets_encrypted_at_rest && state.encryption_in_transit,
                format!(
                    "Encryption at rest: {}, in transit: {}",
                    state.secrets_encrypted_at_rest, state.encryption_in_transit
                ),
                Some("Enable encryption at rest and in transit for all data stores".to_string()),
            ),
            "GDPR-Art17" => (
                state.data_retention_days <= 365,
                format!("Data retention: {} days", state.data_retention_days),
                Some("Configure data retention policy ≤365 days".to_string()),
            ),
            "GDPR-Art25" => (
                state.pii_scrubbing_enabled,
                format!("PII scrubbing: {}", state.pii_scrubbing_enabled),
                None,
            ),
            // PCI-DSS
            "PCI-3.4" => (
                state.secrets_encrypted_at_rest,
                format!("Secrets encrypted: {}", state.secrets_encrypted_at_rest),
                Some("Encrypt all secrets at rest using KMS".to_string()),
            ),
            "PCI-4.1" => (
                state.encryption_in_transit,
                format!("TLS enforced: {}", state.encryption_in_transit),
                None,
            ),
            "PCI-10.2" => (
                state.access_logging_enabled,
                format!("Access logging: {}", state.access_logging_enabled),
                None,
            ),
            "PCI-11.2" => (
                state.vulnerability_scan_enabled,
                format!("Vuln scanning: {}", state.vulnerability_scan_enabled),
                None,
            ),
            _ => (true, "Rule not evaluated".to_string(), None),
        };

        RuleResult {
            rule: rule.clone(),
            passed,
            evidence,
            remediation: if passed { None } else { remediation },
        }
    }

    fn soc2_rules() -> Vec<ComplianceRule> {
        vec![
            rule("SOC2-CC6.1", ComplianceFramework::Soc2, "Logical Access Controls", "RBAC must be enforced", RuleSeverity::Critical),
            rule("SOC2-CC6.6", ComplianceFramework::Soc2, "Encryption in Transit", "mTLS for API communication", RuleSeverity::High),
            rule("SOC2-CC7.2", ComplianceFramework::Soc2, "Audit Logging", "All admin actions must be logged", RuleSeverity::High),
        ]
    }

    fn gdpr_rules() -> Vec<ComplianceRule> {
        vec![
            rule("GDPR-Art32", ComplianceFramework::Gdpr, "Security of Processing", "Encryption required", RuleSeverity::Critical),
            rule("GDPR-Art17", ComplianceFramework::Gdpr, "Right to Erasure", "Data retention limits", RuleSeverity::High),
            rule("GDPR-Art25", ComplianceFramework::Gdpr, "Data Protection by Design", "PII scrubbing in logs", RuleSeverity::Medium),
        ]
    }

    fn pci_dss_rules() -> Vec<ComplianceRule> {
        vec![
            rule("PCI-3.4", ComplianceFramework::PciDss, "Render PAN Unreadable", "Encrypt secrets at rest", RuleSeverity::Critical),
            rule("PCI-4.1", ComplianceFramework::PciDss, "Strong Cryptography", "TLS for data transmission", RuleSeverity::Critical),
            rule("PCI-10.2", ComplianceFramework::PciDss, "Audit Trails", "Log all access to cardholder data", RuleSeverity::High),
            rule("PCI-11.2", ComplianceFramework::PciDss, "Vulnerability Scans", "Regular vulnerability scanning", RuleSeverity::High),
        ]
    }
}

fn rule(id: &str, framework: ComplianceFramework, title: &str, desc: &str, severity: RuleSeverity) -> ComplianceRule {
    ComplianceRule {
        id: id.to_string(),
        framework,
        title: title.to_string(),
        description: desc.to_string(),
        severity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soc2_validates_rbac() {
        let state = ClusterComplianceState {
            rbac_enabled: false,
            ..Default::default()
        };
        let results = ValidationPipeline::validate(ComplianceFramework::Soc2, &state);
        let rbac = results.iter().find(|r| r.rule.id == "SOC2-CC6.1").unwrap();
        assert!(!rbac.passed);
    }

    #[test]
    fn gdpr_validates_encryption() {
        let state = ClusterComplianceState {
            secrets_encrypted_at_rest: true,
            encryption_in_transit: true,
            ..Default::default()
        };
        let results = ValidationPipeline::validate(ComplianceFramework::Gdpr, &state);
        let enc = results.iter().find(|r| r.rule.id == "GDPR-Art32").unwrap();
        assert!(enc.passed);
    }

    #[test]
    fn validate_all_frameworks() {
        let state = ClusterComplianceState {
            mtl_enabled: true,
            audit_logging_enabled: true,
            rbac_enabled: true,
            secrets_encrypted_at_rest: true,
            encryption_in_transit: true,
            pii_scrubbing_enabled: true,
            access_logging_enabled: true,
            vulnerability_scan_enabled: true,
            data_retention_days: 90,
            ..Default::default()
        };
        let results = ValidationPipeline::validate_all(&state);
        assert!(results.iter().all(|r| r.passed));
    }
}
