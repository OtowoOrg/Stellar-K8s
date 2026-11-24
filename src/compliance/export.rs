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
//! Compliance report export in PDF, JSON, and CSV formats.

use std::collections::BTreeMap;
use std::io::BufWriter;

use printpdf::{Mm, PdfDocument};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::report::ComplianceReport;
use crate::error::{Error, Result};

/// Export format selection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComplianceExportFormat {
    Json,
    Pdf,
    Csv,
}

/// Export compliance report as JSON bytes.
pub fn export_json(report: &ComplianceReport) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(report)
        .map_err(|e| Error::InternalError(format!("JSON export failed: {e}")))
}

/// Export compliance report as CSV bytes.
pub fn export_csv(report: &ComplianceReport) -> Result<Vec<u8>> {
    let mut wtr = csv::Writer::from_writer(vec![]);

    wtr.write_record(["Framework", "Compliant", "Score %", "Passed", "Total"])
        .map_err(|e| Error::InternalError(e.to_string()))?;

    for status in &report.framework_statuses {
        let compliant = status.compliant.to_string();
        let score = format!("{:.1}", status.score_pct);
        let passed = status.passed_rules.to_string();
        let total = status.total_rules.to_string();
        wtr.write_record([
            status.framework.name(),
            compliant.as_str(),
            score.as_str(),
            passed.as_str(),
            total.as_str(),
        ])
        .map_err(|e| Error::InternalError(e.to_string()))?;
    }

    wtr.write_record(["", "", "", "", ""])
        .map_err(|e| Error::InternalError(e.to_string()))?;
    wtr.write_record(["Rule ID", "Framework", "Passed", "Evidence", "Remediation"])
        .map_err(|e| Error::InternalError(e.to_string()))?;

    for status in &report.framework_statuses {
        for rule in &status.failed_rules {
            let passed = rule.passed.to_string();
            let remediation = rule.remediation.clone().unwrap_or_default();
            wtr.write_record([
                rule.rule.id.as_str(),
                status.framework.name(),
                passed.as_str(),
                rule.evidence.as_str(),
                remediation.as_str(),
            ])
            .map_err(|e| Error::InternalError(e.to_string()))?;
        }
    }

    wtr.into_inner()
        .map_err(|e| Error::InternalError(e.to_string()))
}

/// Export compliance report as PDF bytes.
pub fn export_pdf(report: &ComplianceReport) -> Result<Vec<u8>> {
    let (doc, page1, layer1) =
        PdfDocument::new("Compliance Report", Mm(210.0), Mm(297.0), "Layer 1");

    let font = doc
        .add_builtin_font(printpdf::BuiltinFont::Helvetica)
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let font_bold = doc
        .add_builtin_font(printpdf::BuiltinFont::HelveticaBold)
        .map_err(|e| Error::InternalError(e.to_string()))?;

    let layer = doc.get_page(page1).get_layer(layer1);
    let mut y = 277.0;

    layer.use_text(
        "Stellar-K8s Compliance Report",
        18.0,
        Mm(15.0),
        Mm(y),
        &font_bold,
    );
    y -= 10.0;
    layer.use_text(
        format!(
            "Generated: {}",
            report.generated_at.format("%Y-%m-%dT%H:%M:%SZ")
        ),
        10.0,
        Mm(15.0),
        Mm(y),
        &font,
    );
    y -= 8.0;
    layer.use_text(
        format!("Overall Score: {:.1}%", report.overall_score_pct),
        12.0,
        Mm(15.0),
        Mm(y),
        &font,
    );
    y -= 8.0;
    layer.use_text(
        format!(
            "Status: {}",
            if report.overall_compliant {
                "COMPLIANT"
            } else {
                "NON-COMPLIANT"
            }
        ),
        12.0,
        Mm(15.0),
        Mm(y),
        &font,
    );
    y -= 15.0;

    layer.use_text("Framework Results", 14.0, Mm(15.0), Mm(y), &font_bold);
    y -= 10.0;

    for status in &report.framework_statuses {
        layer.use_text(
            format!(
                "{}  {:.0}%  ({}/{})  {}",
                status.framework.name(),
                status.score_pct,
                status.passed_rules,
                status.total_rules,
                if status.compliant { "PASS" } else { "FAIL" }
            ),
            10.0,
            Mm(15.0),
            Mm(y),
            &font,
        );
        y -= 7.0;
    }

    y -= 10.0;
    layer.use_text(
        format!("Drift Findings: {}", report.drift_findings.len()),
        12.0,
        Mm(15.0),
        Mm(y),
        &font_bold,
    );
    y -= 8.0;

    for drift in &report.drift_findings {
        layer.use_text(
            format!(
                "  {} expected={} actual={}",
                drift.field, drift.expected, drift.actual
            ),
            9.0,
            Mm(15.0),
            Mm(y),
            &font,
        );
        y -= 6.0;
    }

    let mut buf = BufWriter::new(Vec::new());
    doc.save(&mut buf)
        .map_err(|e| Error::InternalError(format!("PDF save failed: {e}")))?;
    buf.into_inner()
        .map_err(|e| Error::InternalError(format!("PDF buffer flush failed: {e}")))
}

/// Export in the requested format.
pub fn export(report: &ComplianceReport, format: ComplianceExportFormat) -> Result<Vec<u8>> {
    match format {
        ComplianceExportFormat::Json => export_json(report),
        ComplianceExportFormat::Pdf => export_pdf(report),
        ComplianceExportFormat::Csv => export_csv(report),
    }
}

/// A single evidence artifact bundled into a signed evidence package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceArtifact {
    /// Framework control ID this artifact provides evidence for.
    pub control_id: String,
    /// Logical name of the artifact (e.g. `report.json`).
    pub name: String,
    /// Hex-encoded SHA-256 digest of `content`.
    pub sha256: String,
    /// Raw artifact content.
    pub content: Vec<u8>,
}

impl EvidenceArtifact {
    /// Build an artifact, computing its content digest.
    pub fn new(control_id: impl Into<String>, name: impl Into<String>, content: Vec<u8>) -> Self {
        let sha256 = hex_sha256(&content);
        Self {
            control_id: control_id.into(),
            name: name.into(),
            sha256,
            content,
        }
    }
}

/// Manifest describing the contents of an evidence package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceManifest {
    /// Identifier of the package.
    pub package_id: String,
    /// RFC 3339 timestamp of package creation.
    pub created_at: String,
    /// Digest of each artifact keyed by `control_id/name`.
    pub artifact_digests: BTreeMap<String, String>,
    /// Hex-encoded SHA-256 digest over the canonical manifest body.
    pub manifest_digest: String,
}

/// A signed evidence package that can be verified offline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedEvidencePackage {
    /// Package format version.
    pub version: u32,
    /// Manifest describing the package contents.
    pub manifest: EvidenceManifest,
    /// Evidence artifacts.
    pub artifacts: Vec<EvidenceArtifact>,
    /// Hex-encoded signature over `manifest.manifest_digest`.
    pub signature: String,
}

/// Compute the lowercase hex SHA-256 digest of `data`.
fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Compute the canonical manifest digest for the given artifacts.
fn manifest_digest(
    package_id: &str,
    created_at: &str,
    artifacts: &[EvidenceArtifact],
) -> Result<String> {
    let mut digests = BTreeMap::new();
    for artifact in artifacts {
        let key = format!("{}/{}", artifact.control_id, artifact.name);
        if digests.insert(key, artifact.sha256.clone()).is_some() {
            return Err(Error::InternalError(format!(
                "duplicate evidence artifact: {}/{}",
                artifact.control_id, artifact.name
            )));
        }
    }
    let body = serde_json::json!({
        "package_id": package_id,
        "created_at": created_at,
        "artifact_digests": digests,
    });
    let encoded = serde_json::to_vec(&body)
        .map_err(|e| Error::InternalError(format!("manifest encoding failed: {e}")))?;
    Ok(hex_sha256(&encoded))
}

/// Sign a manifest digest with the provided signing key.
///
/// The signature is a keyed SHA-256 over the digest, which is sufficient for
/// offline integrity verification without external crypto dependencies.
fn sign_digest(digest: &str, signing_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(signing_key);
    hasher.update(digest.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Build a signed evidence package from a compliance report.
pub fn export_signed_evidence_package(
    report: &ComplianceReport,
    package_id: &str,
    created_at: &str,
    signing_key: &[u8],
) -> Result<SignedEvidencePackage> {
    let report_json = export_json(report)?;
    let report_csv = export_csv(report)?;

    let mut artifacts = Vec::new();
    for status in &report.framework_statuses {
        let control_id = status.framework.name().to_string();
        artifacts.push(EvidenceArtifact::new(
            control_id.clone(),
            "report.json",
            report_json.clone(),
        ));
        artifacts.push(EvidenceArtifact::new(
            control_id,
            "report.csv",
            report_csv.clone(),
        ));
    }

    let digest = manifest_digest(package_id, created_at, &artifacts)?;
    let signature = sign_digest(&digest, signing_key);

    let mut artifact_digests = BTreeMap::new();
    for artifact in &artifacts {
        artifact_digests.insert(
            format!("{}/{}", artifact.control_id, artifact.name),
            artifact.sha256.clone(),
        );
    }

    Ok(SignedEvidencePackage {
        version: 1,
        manifest: EvidenceManifest {
            package_id: package_id.to_string(),
            created_at: created_at.to_string(),
            artifact_digests,
            manifest_digest: digest,
        },
        artifacts,
        signature,
    })
}

/// Serialize a signed evidence package to JSON bytes.
pub fn export_signed_evidence_package_json(package: &SignedEvidencePackage) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(package)
        .map_err(|e| Error::InternalError(format!("evidence package export failed: {e}")))
}

/// Verify a signed evidence package offline.
///
/// Validates the signature, the manifest digest, and every artifact content
/// hash without requiring cluster access. Returns `Ok(())` when the package is
/// intact and authentic, or an error describing the first failure.
pub fn verify_signed_evidence_package(
    package: &SignedEvidencePackage,
    signing_key: &[u8],
) -> Result<()> {
    if package.version != 1 {
        return Err(Error::InternalError(format!(
            "unsupported evidence package version: {}",
            package.version
        )));
    }

    let expected_signature = sign_digest(&package.manifest.manifest_digest, signing_key);
    if expected_signature != package.signature {
        return Err(Error::InternalError(
            "evidence package signature verification failed".to_string(),
        ));
    }

    let expected_digest = manifest_digest(
        &package.manifest.package_id,
        &package.manifest.created_at,
        &package.artifacts,
    )?;
    if expected_digest != package.manifest.manifest_digest {
        return Err(Error::InternalError(
            "evidence package manifest digest mismatch".to_string(),
        ));
    }

    for artifact in &package.artifacts {
        let actual = hex_sha256(&artifact.content);
        if actual != artifact.sha256 {
            return Err(Error::InternalError(format!(
                "evidence artifact content hash mismatch: {}/{}",
                artifact.control_id, artifact.name
            )));
        }
        let key = format!("{}/{}", artifact.control_id, artifact.name);
        match package.manifest.artifact_digests.get(&key) {
            Some(digest) if digest == &artifact.sha256 => {}
            _ => {
                return Err(Error::InternalError(format!(
                    "evidence artifact missing from manifest: {key}"
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compliance::frameworks::ClusterComplianceState;
    use crate::compliance::report::ReportGenerator;

    fn sample_report() -> ComplianceReport {
        let state = ClusterComplianceState {
            rbac_enabled: true,
            mtl_enabled: true,
            audit_logging_enabled: true,
            ..Default::default()
        };
        ReportGenerator::new(state.clone()).generate(&state)
    }

    #[test]
    fn export_json_valid() {
        let bytes = export_json(&sample_report()).unwrap();
        let parsed: ComplianceReport = serde_json::from_slice(&bytes).unwrap();
        assert!(!parsed.report_id.is_empty());
    }

    #[test]
    fn export_csv_has_headers() {
        let bytes = export_csv(&sample_report()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Framework"));
        assert!(text.contains("SOC 2"));
    }

    #[test]
    fn export_pdf_magic_bytes() {
        let bytes = export_pdf(&sample_report()).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
    }

    const SIGNING_KEY: &[u8] = b"test-signing-key";

    fn sample_package() -> SignedEvidencePackage {
        export_signed_evidence_package(
            &sample_report(),
            "pkg-1",
            "2024-01-01T00:00:00Z",
            SIGNING_KEY,
        )
        .unwrap()
    }

    #[test]
    fn signed_package_roundtrips_and_verifies_offline() {
        let package = sample_package();
        let bytes = export_signed_evidence_package_json(&package).unwrap();
        let parsed: SignedEvidencePackage = serde_json::from_slice(&bytes).unwrap();
        verify_signed_evidence_package(&parsed, SIGNING_KEY).unwrap();
    }

    #[test]
    fn signed_package_rejects_tampered_content() {
        let mut package = sample_package();
        package.artifacts[0].content.push(b'x');
        assert!(verify_signed_evidence_package(&package, SIGNING_KEY).is_err());
    }

    #[test]
    fn signed_package_rejects_tampered_manifest() {
        let mut package = sample_package();
        package.manifest.package_id = "pkg-2".to_string();
        assert!(verify_signed_evidence_package(&package, SIGNING_KEY).is_err());
    }

    #[test]
    fn signed_package_rejects_wrong_key() {
        let package = sample_package();
        assert!(verify_signed_evidence_package(&package, b"other-key").is_err());
    }
}
