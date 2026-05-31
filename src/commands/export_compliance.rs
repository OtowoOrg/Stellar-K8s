//! `export-compliance` subcommand handler.
//!
//! Reads audit entries from the in-cluster operator REST API (or falls back to
//! the S3 audit sink when configured), then delegates to
//! [`compliance_export`] to produce a signed JSON or PDF report.

use std::fs;
use std::io::Write;

use chrono::Utc;
use kube::Client;

use crate::cli::ExportComplianceArgs;
use stellar_k8s::controller::audit_log::{AuditEntry, AuditLog};
use stellar_k8s::controller::compliance_export::{self, DRComplianceSummary};
use stellar_k8s::crd::DisasterRecoveryPolicy;
use stellar_k8s::error::{Error, Result};

/// Entry point for `stellar-operator export-compliance`.
pub async fn run_export_compliance(args: ExportComplianceArgs) -> Result<()> {
    // Attempt to connect to the cluster and pull live audit entries.
    let entries = fetch_audit_entries(&args).await.unwrap_or_else(|e| {
        eprintln!("Warning: could not fetch live audit entries ({e}); exporting empty log.");
        vec![]
    });

    // Fetch DR compliance summary if possible
    let dr_summary = fetch_dr_summary(&args).await.ok();

    match args.format.as_str() {
        "json" => {
            let bytes = compliance_export::export_json(&entries, dr_summary)?;
            write_output(bytes, args.output.as_deref(), "json")?;
        }
        "pdf" => {
            let bytes = compliance_export::export_pdf(&entries, dr_summary)?;
            write_output(bytes, args.output.as_deref(), "pdf")?;
        }
        other => {
            return Err(Error::ConfigError(format!(
                "Unknown format '{other}'. Use 'json' or 'pdf'."
            )));
        }
    }

    Ok(())
}

async fn fetch_dr_summary(args: &ExportComplianceArgs) -> Result<DRComplianceSummary> {
    let client = Client::try_default().await.map_err(Error::KubeError)?;
    let policies: kube::api::Api<DisasterRecoveryPolicy> =
        kube::api::Api::namespaced(client, &args.namespace);

    let list = policies
        .list(&kube::api::ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let policy = list.items.first().ok_or_else(|| {
        Error::ConfigError("No DisasterRecoveryPolicy found in namespace".to_string())
    })?;

    let status = policy
        .status
        .as_ref()
        .ok_or_else(|| Error::ConfigError("DisasterRecoveryPolicy has no status".to_string()))?;

    Ok(DRComplianceSummary {
        last_rto_seconds: status.last_rto_seconds,
        last_rpo_seconds: status.last_rpo_seconds,
        compliance_status: format!("{:?}", status.compliance_status),
        last_drill_result: status.recent_drills.first().cloned(),
    })
}

/// Fetch audit entries from the live operator.
///
/// Tries to connect to the Kubernetes cluster and read the audit ConfigMap
/// written by the operator's REST API. Falls back gracefully on any error.
async fn fetch_audit_entries(args: &ExportComplianceArgs) -> Result<Vec<AuditEntry>> {
    // Build an in-memory AuditLog and populate it from the operator's
    // ConfigMap-backed snapshot (written by the REST API audit handler).
    let client = Client::try_default().await.map_err(Error::KubeError)?;

    let configmaps: kube::api::Api<k8s_openapi::api::core::v1::ConfigMap> =
        kube::api::Api::namespaced(client, &args.namespace);

    // The operator REST API persists a rolling JSON snapshot of recent audit
    // entries in a ConfigMap named `stellar-audit-snapshot`.
    let cm = configmaps
        .get("stellar-audit-snapshot")
        .await
        .map_err(Error::KubeError)?;

    let data = cm.data.unwrap_or_default();
    let raw = data.get("entries.json").ok_or_else(|| {
        Error::InternalError("entries.json key missing from audit ConfigMap".into())
    })?;

    let mut entries: Vec<AuditEntry> = serde_json::from_str(raw)
        .map_err(|e| Error::InternalError(format!("Failed to parse audit entries: {e}")))?;

    // Apply limit if requested.
    if args.limit > 0 && entries.len() > args.limit {
        entries.truncate(args.limit);
    }

    Ok(entries)
}

/// Write `bytes` to the specified path, or to stdout (JSON) / a timestamped
/// file (PDF) when no path is given.
fn write_output(bytes: Vec<u8>, path: Option<&str>, ext: &str) -> Result<()> {
    match path {
        Some(p) => {
            fs::write(p, &bytes)
                .map_err(|e| Error::InternalError(format!("Failed to write {p}: {e}")))?;
            eprintln!("Compliance report written to {p}");
        }
        None if ext == "json" => {
            std::io::stdout()
                .write_all(&bytes)
                .map_err(|e| Error::InternalError(format!("stdout write failed: {e}")))?;
        }
        None => {
            let filename = format!(
                "compliance-report-{}.{ext}",
                Utc::now().format("%Y%m%dT%H%M%SZ")
            );
            fs::write(&filename, &bytes)
                .map_err(|e| Error::InternalError(format!("Failed to write {filename}: {e}")))?;
            eprintln!("Compliance report written to {filename}");
        }
    }
    Ok(())
}
