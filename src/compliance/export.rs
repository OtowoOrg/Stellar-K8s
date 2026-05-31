//! Compliance report export in PDF, JSON, and CSV formats.

use std::io::BufWriter;

use printpdf::{Mm, PdfDocument};
use serde::{Deserialize, Serialize};

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

    layer.use_text("Stellar-K8s Compliance Report", 18.0, Mm(15.0), Mm(y), &font_bold);
    y -= 10.0;
    layer.use_text(
        format!("Generated: {}", report.generated_at.format("%Y-%m-%dT%H:%M:%SZ")),
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
            format!("  {} expected={} actual={}", drift.field, drift.expected, drift.actual),
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
}
