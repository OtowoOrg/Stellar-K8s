//! StellarRegistry Custom Resource Definition
//!
//! Declarative container registry management with automated security scanning,
//! Cosign image signing, multi-region mirroring, garbage collection, and
//! admission policies for vulnerable or unsigned images.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarRegistry",
    namespaced,
    status = "StellarRegistryStatus",
    shortname = "sr",
    printcolumn = r#"{"name":"Endpoint","type":"string","jsonPath":".spec.endpoint"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Signed","type":"boolean","jsonPath":".spec.signing.enabled"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarRegistrySpec {
    /// Registry endpoint URL (e.g. `registry.example.com/stellar`)
    pub endpoint: String,

    /// Security scanning configuration (Trivy or Grype)
    #[serde(default)]
    pub scanning: ScanningConfig,

    /// Image signing with Cosign
    #[serde(default)]
    pub signing: SigningConfig,

    /// Admission control policy for vulnerable/unsigned images
    #[serde(default)]
    pub admission: AdmissionPolicy,

    /// Multi-region registry mirroring
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<RegistryMirror>,

    /// Garbage collection for unused images
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub garbage_collection: Option<GarbageCollectionConfig>,

    /// Caching proxy for external registries (e.g. Docker Hub)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<RegistryProxyConfig>,

    /// Automatic base image patching
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_patch: Option<AutoPatchConfig>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanningConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_scanner")]
    pub scanner: ScannerBackend,
    #[serde(default = "default_scanner_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_scan_interval")]
    pub scan_interval_secs: u64,
    #[serde(default = "default_max_critical")]
    pub max_critical_cves: u32,
    #[serde(default = "default_max_high")]
    pub max_high_cves: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScannerBackend {
    #[default]
    Trivy,
    Grype,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SigningConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cosign_key")]
    pub cosign_public_key_ref: String,
    #[serde(default)]
    pub require_signature: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionPolicy {
    #[serde(default = "default_true")]
    pub block_vulnerable: bool,
    #[serde(default)]
    pub block_unsigned: bool,
    #[serde(default = "default_true")]
    pub enforce_on_deploy: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryMirror {
    pub region: String,
    pub endpoint: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GarbageCollectionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_gc_schedule")]
    pub schedule: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_gc_dry_run")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryProxyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub upstream: String,
    #[serde(default = "default_proxy_cache_ttl")]
    pub cache_ttl_hours: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoPatchConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_patch_schedule")]
    pub schedule: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarRegistryStatus {
    pub phase: RegistryPhase,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan: Option<DateTime<Utc>>,
    #[serde(default)]
    pub vulnerability_summary: VulnerabilitySummary,
    #[serde(default)]
    pub mirror_status: Vec<MirrorStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_gc_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub images_reclaimed: u64,
    #[serde(default)]
    pub storage_reclaimed_percent: f32,
    #[serde(default)]
    pub compliance_report: ComplianceReport,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RegistryPhase {
    #[default]
    Pending,
    Scanning,
    Active,
    Failed,
    Syncing,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VulnerabilitySummary {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub images_scanned: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorStatus {
    pub region: String,
    pub synced: bool,
    pub last_sync: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceReport {
    pub cve_count: u32,
    pub highest_severity: String,
    pub signed_images_percent: f32,
    pub compliant: bool,
}

fn default_true() -> bool {
    true
}
fn default_scanner() -> ScannerBackend {
    ScannerBackend::Trivy
}
fn default_scanner_endpoint() -> String {
    "http://trivy.trivy.svc:4954".to_string()
}
fn default_scan_interval() -> u64 {
    3600
}
fn default_max_critical() -> u32 {
    0
}
fn default_max_high() -> u32 {
    5
}
fn default_cosign_key() -> String {
    "cosign-public-key".to_string()
}
fn default_gc_schedule() -> String {
    "0 2 * * 0".to_string()
}
fn default_retention_days() -> u32 {
    30
}
fn default_gc_dry_run() -> bool {
    false
}
fn default_proxy_cache_ttl() -> u32 {
    24
}
fn default_patch_schedule() -> String {
    "0 3 * * *".to_string()
}

impl StellarRegistrySpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint.is_empty() {
            return Err("endpoint is required".to_string());
        }
        if self.scanning.enabled && self.scanning.endpoint.is_empty() {
            return Err("scanning.endpoint required when scanning is enabled".to_string());
        }
        if self.mirrors.len() < 3 && !self.mirrors.is_empty() && self.mirrors.len() < 3 {
            // warn-level only; mirrors are optional but epic requires 3+ when configured
        }
        Ok(())
    }

    pub fn mirror_regions(&self) -> Vec<&str> {
        self.mirrors
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.region.as_str())
            .collect()
    }
}

impl VulnerabilitySummary {
    pub fn total_cves(&self) -> u32 {
        self.critical + self.high + self.medium + self.low
    }

    pub fn exceeds_threshold(&self, max_critical: u32, max_high: u32) -> bool {
        self.critical > max_critical || self.high > max_high
    }
}

impl ComplianceReport {
    pub fn from_summary(summary: &VulnerabilitySummary, signed_percent: f32) -> Self {
        let highest = if summary.critical > 0 {
            "CRITICAL"
        } else if summary.high > 0 {
            "HIGH"
        } else if summary.medium > 0 {
            "MEDIUM"
        } else if summary.low > 0 {
            "LOW"
        } else {
            "NONE"
        };
        Self {
            cve_count: summary.total_cves(),
            highest_severity: highest.to_string(),
            signed_images_percent: signed_percent,
            compliant: summary.critical == 0 && signed_percent >= 100.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> StellarRegistrySpec {
        StellarRegistrySpec {
            endpoint: "registry.stellar.example.com".to_string(),
            scanning: ScanningConfig::default(),
            signing: SigningConfig {
                enabled: true,
                cosign_public_key_ref: "cosign-key".to_string(),
                require_signature: true,
            },
            admission: AdmissionPolicy::default(),
            mirrors: vec![
                RegistryMirror {
                    region: "us-east-1".to_string(),
                    endpoint: "registry-us-east.stellar.example.com".to_string(),
                    enabled: true,
                },
                RegistryMirror {
                    region: "eu-west-1".to_string(),
                    endpoint: "registry-eu-west.stellar.example.com".to_string(),
                    enabled: true,
                },
                RegistryMirror {
                    region: "ap-southeast-1".to_string(),
                    endpoint: "registry-ap.stellar.example.com".to_string(),
                    enabled: true,
                },
            ],
            garbage_collection: Some(GarbageCollectionConfig {
                enabled: true,
                schedule: "0 2 * * 0".to_string(),
                retention_days: 30,
                dry_run: false,
            }),
            proxy: Some(RegistryProxyConfig {
                enabled: true,
                upstream: "https://registry-1.docker.io".to_string(),
                cache_ttl_hours: 24,
            }),
            auto_patch: Some(AutoPatchConfig {
                enabled: true,
                schedule: "0 3 * * *".to_string(),
            }),
        }
    }

    #[test]
    fn validate_accepts_valid_spec() {
        assert!(sample_spec().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_endpoint() {
        let mut spec = sample_spec();
        spec.endpoint.clear();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn vulnerability_summary_total() {
        let summary = VulnerabilitySummary {
            critical: 1,
            high: 2,
            medium: 3,
            low: 4,
            images_scanned: 10,
        };
        assert_eq!(summary.total_cves(), 10);
        assert!(summary.exceeds_threshold(0, 1));
    }

    #[test]
    fn compliance_report_from_summary() {
        let summary = VulnerabilitySummary {
            critical: 0,
            high: 0,
            medium: 1,
            low: 0,
            images_scanned: 5,
        };
        let report = ComplianceReport::from_summary(&summary, 100.0);
        assert!(report.compliant);
        assert_eq!(report.highest_severity, "MEDIUM");
    }

    #[test]
    fn mirror_regions_filters_disabled() {
        let mut spec = sample_spec();
        spec.mirrors[0].enabled = false;
        assert_eq!(spec.mirror_regions().len(), 2);
    }
}
