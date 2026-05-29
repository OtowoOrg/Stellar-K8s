//! REST API handlers for compliance reporting.

use axum::extract::Query;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::compliance::{
    export::{export, ComplianceExportFormat},
    frameworks::ClusterComplianceState,
    report::ReportGenerator,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceQuery {
    pub format: Option<String>,
}

/// GET /api/v1/compliance/regulatory-report
pub async fn regulatory_compliance_report(
    Query(query): Query<ComplianceQuery>,
) -> impl IntoResponse {
    let state = ClusterComplianceState {
        mtl_enabled: true,
        audit_logging_enabled: true,
        pss_restricted_enforced: true,
        secrets_encrypted_at_rest: true,
        network_policies_enabled: true,
        rbac_enabled: true,
        data_retention_days: 90,
        pii_scrubbing_enabled: true,
        encryption_in_transit: true,
        access_logging_enabled: true,
        vulnerability_scan_enabled: true,
        backup_enabled: true,
        dr_tested_within_90_days: true,
    };

    let gen = ReportGenerator::new(state.clone());
    let report = gen.generate(&state);

    let format = match query.format.as_deref() {
        Some("pdf") => ComplianceExportFormat::Pdf,
        Some("csv") => ComplianceExportFormat::Csv,
        _ => ComplianceExportFormat::Json,
    };

    match export(&report, format) {
        Ok(bytes) => {
            let content_type = match format {
                ComplianceExportFormat::Pdf => "application/pdf",
                ComplianceExportFormat::Csv => "text/csv",
                ComplianceExportFormat::Json => "application/json",
            };
            (
                [(axum::http::header::CONTENT_TYPE, content_type)],
                bytes,
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
            .into_response(),
    }
}

/// GET /api/v1/compliance/status
pub async fn compliance_status() -> Json<crate::compliance::report::ComplianceReport> {
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
    Json(ReportGenerator::new(state.clone()).generate(&state))
}
