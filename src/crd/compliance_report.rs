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
//! ComplianceReport CRD for Regulated Stellar Validators (#1581)
//!
//! Declaratively schedules and manages automated regulatory compliance reports
//! (uptime evidence, transaction processing performance, and key custody attestation)
//! for validators operating in regulated jurisdictions.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Custom Resource Definition for Validator Compliance Reporting
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "compliance.stellar.org",
    version = "v1alpha1",
    kind = "ComplianceReport",
    namespaced,
    status = "ComplianceReportStatus",
    shortname = "compreport",
    printcolumn = r#"{"name":"Validator","type":"string","jsonPath":".spec.validatorRef"}"#,
    printcolumn = r#"{"name":"Schedule","type":"string","jsonPath":".spec.schedule"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Verdict","type":"string","jsonPath":".status.regulatoryVerdict"}"#,
    printcolumn = r#"{"name":"Uptime","type":"string","jsonPath":".status.uptimeEvidence.uptimePercentage"}"#,
    printcolumn = r#"{"name":"LastGenerated","type":"date","jsonPath":".status.lastGeneratedAt"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceReportSpec {
    /// Name of the target StellarNode validator to audit.
    pub validator_ref: String,

    /// Frequency schedule for report generation ("Daily", "Weekly", or standard cron).
    #[serde(default = "default_schedule")]
    pub schedule: String,

    /// Reporting time window in days (default: 7).
    #[serde(default = "default_period_days")]
    pub period_days: u32,

    /// Whether to collect uptime and availability evidence.
    #[serde(default = "default_true")]
    pub include_uptime_evidence: bool,

    /// Whether to generate hardware/cloud key custody attestation (HSM/KMS).
    #[serde(default = "default_true")]
    pub include_key_custody: bool,

    /// Whether to collect ledger close and transaction processing evidence.
    #[serde(default = "default_true")]
    pub include_tx_processing: bool,

    /// Export formats to generate for auditors.
    #[serde(default = "default_export_formats")]
    pub formats: Vec<ComplianceReportFormat>,

    /// Destination storage for generated compliance artifacts.
    #[serde(default)]
    pub destination: ReportDestinationConfig,

    /// Optional verification parameters for KMS/HSM custody assertion.
    #[serde(default)]
    pub hsm_kms_verification: Option<HsmKmsVerificationSpec>,
}

fn default_schedule() -> String {
    "Weekly".to_string()
}

fn default_period_days() -> u32 {
    7
}

fn default_true() -> bool {
    true
}

fn default_export_formats() -> Vec<ComplianceReportFormat> {
    vec![ComplianceReportFormat::Json, ComplianceReportFormat::Pdf]
}

/// Supported compliance export formats.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ComplianceReportFormat {
    Json,
    Pdf,
    Csv,
}

/// Destination configuration for compliance report artifacts.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReportDestinationConfig {
    /// Store report as a Kubernetes ConfigMap in the validator namespace.
    #[serde(default = "default_true")]
    pub save_config_map: bool,

    /// Optional prefix for ConfigMap name. Defaults to "compliance-report-".
    #[serde(default)]
    pub config_map_prefix: Option<String>,

    /// Optional S3 / object storage bucket to upload the artifacts.
    #[serde(default)]
    pub object_storage_bucket: Option<String>,

    /// Optional object storage path prefix.
    #[serde(default)]
    pub object_storage_prefix: Option<String>,
}

impl Default for ReportDestinationConfig {
    fn default() -> Self {
        Self {
            save_config_map: true,
            config_map_prefix: Some("compliance-report-".to_string()),
            object_storage_bucket: None,
            object_storage_prefix: None,
        }
    }
}

/// Specification for expected KMS / HSM custody parameters.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HsmKmsVerificationSpec {
    /// Expected KMS Provider (e.g. "AWS-KMS", "Azure-KeyVault", "GCP-CloudKMS", "Hardware-HSM-PKCS11").
    pub expected_provider: Option<String>,

    /// Expected Key Identifier or ARN.
    pub expected_key_id: Option<String>,

    /// Enforce FIPS 140-2 Level 3 / HSM hardware backing.
    #[serde(default)]
    pub enforce_fips_level3: bool,
}

/// Phase of the compliance report lifecycle.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum ComplianceReportPhase {
    #[default]
    Pending,
    Scheduled,
    Generating,
    Generated,
    Failed,
}

/// Observed compliance status and generated evidence.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceReportStatus {
    /// Current lifecycle phase.
    pub phase: ComplianceReportPhase,

    /// Timestamp of last successful report generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_generated_at: Option<DateTime<Utc>>,

    /// Next scheduled execution timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_scheduled_at: Option<DateTime<Utc>>,

    /// Audit window start timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_start: Option<DateTime<Utc>>,

    /// Audit window end timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_end: Option<DateTime<Utc>>,

    /// Final regulatory verdict based on evidence against SLA.
    #[serde(default)]
    pub regulatory_verdict: String,

    /// Evidence verifying validator availability and uptime SLA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_evidence: Option<ValidatorUptimeEvidence>,

    /// Cryptographic attestation of validator key custody in HSM/KMS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_custody_attestation: Option<KeyCustodyAttestation>,

    /// Evidence verifying transaction and ledger processing continuity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_processing_evidence: Option<TxProcessingEvidence>,

    /// List of generated report artifacts and locations.
    #[serde(default)]
    pub artifacts: Vec<GeneratedArtifactRef>,

    /// Informative status conditions.
    #[serde(default)]
    pub conditions: Vec<ComplianceCondition>,
}

/// Detailed validator uptime evidence.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorUptimeEvidence {
    /// Measured uptime availability percentage (0.0 to 100.0).
    pub uptime_percentage: f64,

    /// Total duration evaluated in seconds.
    pub total_seconds: u64,

    /// Total accumulated downtime or unreachability in seconds.
    pub downtime_seconds: u64,

    /// Count of monitoring health samples evaluated.
    pub monitored_samples: u64,

    /// Whether the validator satisfied the regulatory SLA (typically >= 99.0%).
    pub met_sla: bool,

    /// Timestamp of longest uninterrupted uptime streak.
    pub longest_streak_hours: f64,
}

/// Cryptographic attestation of validator secret keys managed via HSM/KMS.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyCustodyAttestation {
    /// Name of key management provider (e.g., AWS KMS, Azure Key Vault, GCP KMS, Hardware HSM).
    pub provider: String,

    /// Key resource identifier or ARN.
    pub key_id: String,

    /// Hardware Security Module (HSM) backing verified.
    pub hsm_backed: bool,

    /// Cryptographic key type and algorithm (e.g., "ed25519-dalek", "secp256k1").
    pub algorithm: String,

    /// Multi-party or multi-role authorization policy active.
    pub multi_party_authorized: bool,

    /// SHA-256 digest of attestation bundle and KMS policy.
    pub attestation_digest: String,

    /// Timestamp of attestation verification.
    pub attested_at: DateTime<Utc>,
}

/// Evidence of consensus participation and transaction processing.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TxProcessingEvidence {
    /// Total number of ledgers closed during reporting period.
    pub ledgers_closed: u64,

    /// Total transactions validated and processed.
    pub tx_count_processed: u64,

    /// Average ledger close latency in milliseconds.
    pub avg_ledger_close_time_ms: f64,

    /// Consensus participation rate percentage.
    pub consensus_participation_rate: f64,
}

/// Reference to a generated compliance artifact.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedArtifactRef {
    /// Format of the artifact ("JSON", "PDF", "CSV").
    pub format: String,

    /// Storage type ("ConfigMap", "ObjectStorage", "Secret").
    pub storage_type: String,

    /// Reference name or URI to the artifact.
    pub location_ref: String,

    /// Hex-encoded SHA-256 checksum of artifact bytes.
    pub sha256_checksum: String,

    /// Artifact payload size in bytes.
    pub size_bytes: usize,
}

/// Status condition for compliance report.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceCondition {
    pub type_: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_transition_time: Option<DateTime<Utc>>,
}
