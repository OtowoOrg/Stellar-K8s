//! Compliance Audit Exporter
//!
//! Exports the operator's internal audit log, configuration snapshot, and
//! reconciliation summary into a signed, tamper-evident package.
//!
//! # Formats
//!
//! - **JSON** – a self-contained envelope with all audit entries, a config
//!   snapshot, and an ed25519 signature over the SHA-256 digest of the payload.
//! - **PDF**  – a human-readable report generated with `printpdf`, containing
//!   the same information in a structured layout.
//!
//! # Signing
//!
//! The JSON export embeds a `signature` field:
//!
//! ```json
//! {
//!   "payload": { ... },
//!   "sha256":  "hex-encoded digest of canonical JSON payload",
//!   "signature": "hex-encoded ed25519 signature over sha256 bytes"
//! }
//! ```
//!
//! The signing key is an ephemeral ed25519 key generated at export time.
//! The corresponding public key is embedded in the envelope so verifiers can
//! check authenticity without a separate key-distribution step.
//!
//! For production use, replace the ephemeral key with a key loaded from a
//! Kubernetes Secret or KMS.

use std::io::BufWriter;

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer, SigningKey};
use printpdf::{Mm, PdfDocument};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::controller::audit_log::AuditEntry;
use crate::error::{Error, Result};

// ── Public types ─────────────────────────────────────────────────────────────

/// Summary statistics derived from the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSummary {
    pub total_actions: usize,
    pub successful_actions: usize,
    pub failed_actions: usize,
    pub unique_actors: Vec<String>,
    pub action_counts: std::collections::BTreeMap<String, usize>,
}

/// The inner payload of a compliance export (before signing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompliancePayload {
    pub exported_at: DateTime<Utc>,
    pub operator_version: String,
    pub summary: AuditSummary,
    pub entries: Vec<AuditEntry>,
}

/// Signed JSON envelope returned by [`export_json`].
#[derive(Debug, Serialize, Deserialize)]
pub struct SignedExport {
    pub payload: CompliancePayload,
    /// Hex-encoded SHA-256 digest of the canonical JSON serialisation of `payload`.
    pub sha256: String,
    /// Hex-encoded ed25519 signature over the raw SHA-256 bytes.
    pub signature: String,
    /// Hex-encoded ed25519 public key that can be used to verify `signature`.
    pub public_key: String,
}

// ── Core functions ────────────────────────────────────────────────────────────

/// Build an [`AuditSummary`] from a slice of entries.
pub fn summarise(entries: &[AuditEntry]) -> AuditSummary {
    use std::collections::{BTreeMap, BTreeSet};

    let mut actors: BTreeSet<String> = BTreeSet::new();
    let mut action_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = 0usize;

    for e in entries {
        actors.insert(e.actor.clone());
        *action_counts
            .entry(format!("{:?}", e.action))
            .or_insert(0) += 1;
        if !e.success {
            failed += 1;
        }
    }

    AuditSummary {
        total_actions: entries.len(),
        successful_actions: entries.len() - failed,
        failed_actions: failed,
        unique_actors: actors.into_iter().collect(),
        action_counts,
    }
}

/// Export audit entries as a signed JSON envelope.
///
/// Returns the raw bytes of the JSON document.
pub fn export_json(entries: &[AuditEntry]) -> Result<Vec<u8>> {
    let payload = CompliancePayload {
        exported_at: Utc::now(),
        operator_version: env!("CARGO_PKG_VERSION").to_string(),
        summary: summarise(entries),
        entries: entries.to_vec(),
    };

    // Canonical serialisation of the payload (deterministic key order via serde).
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| Error::InternalError(format!("JSON serialisation failed: {e}")))?;

    // SHA-256 digest.
    let digest = Sha256::digest(&payload_bytes);
    let sha256_hex = hex::encode(digest.as_slice());

    // ed25519 signature over the raw digest bytes.
    let signing_key = SigningKey::generate(&mut OsRng);
    let sig = signing_key.sign(digest.as_slice());
    let sig_hex = hex::encode(sig.to_bytes());
    let pubkey_hex = hex::encode(signing_key.verifying_key().to_bytes());

    let envelope = SignedExport {
        payload,
        sha256: sha256_hex,
        signature: sig_hex,
        public_key: pubkey_hex,
    };

    serde_json::to_vec_pretty(&envelope)
        .map_err(|e| Error::InternalError(format!("Envelope serialisation failed: {e}")))
}

/// Export audit entries as a PDF document.
///
/// Returns the raw bytes of the PDF file.
pub fn export_pdf(entries: &[AuditEntry]) -> Result<Vec<u8>> {
    let summary = summarise(entries);
    let exported_at = Utc::now().to_rfc3339();

    let (doc, page1, layer1) =
        PdfDocument::new("Compliance Audit Report", Mm(210.0), Mm(297.0), "Layer 1");

    let font = doc
        .add_builtin_font(printpdf::BuiltinFont::Helvetica)
        .map_err(|e| Error::InternalError(format!("PDF font error: {e}")))?;
    let font_bold = doc
        .add_builtin_font(printpdf::BuiltinFont::HelveticaBold)
        .map_err(|e| Error::InternalError(format!("PDF font error: {e}")))?;

    let layer = doc.get_page(page1).get_layer(layer1);

    // ── Title ────────────────────────────────────────────────────────────────
    layer.use_text(
        "Stellar-K8s Compliance Audit Report",
        18.0,
        Mm(15.0),
        Mm(277.0),
        &font_bold,
    );
    layer.use_text(
        format!("Exported: {exported_at}"),
        10.0,
        Mm(15.0),
        Mm(268.0),
        &font,
    );
    layer.use_text(
        format!("Operator version: {}", env!("CARGO_PKG_VERSION")),
        10.0,
        Mm(15.0),
        Mm(262.0),
        &font,
    );

    // ── Summary ───────────────────────────────────────────────────────────────
    layer.use_text("Summary", 14.0, Mm(15.0), Mm(250.0), &font_bold);
    layer.use_text(
        format!("Total actions:      {}", summary.total_actions),
        10.0,
        Mm(15.0),
        Mm(242.0),
        &font,
    );
    layer.use_text(
        format!("Successful actions: {}", summary.successful_actions),
        10.0,
        Mm(15.0),
        Mm(236.0),
        &font,
    );
    layer.use_text(
        format!("Failed actions:     {}", summary.failed_actions),
        10.0,
        Mm(15.0),
        Mm(230.0),
        &font,
    );
    layer.use_text(
        format!("Unique actors:      {}", summary.unique_actors.join(", ")),
        10.0,
        Mm(15.0),
        Mm(224.0),
        &font,
    );

    // ── Action breakdown ──────────────────────────────────────────────────────
    layer.use_text("Action Breakdown", 12.0, Mm(15.0), Mm(212.0), &font_bold);
    let mut y = 204.0f32;
    for (action, count) in &summary.action_counts {
        if y < 20.0 {
            break; // avoid overflow on first page (entries section handles pagination)
        }
        layer.use_text(
            format!("  {action}: {count}"),
            9.0,
            Mm(15.0),
            Mm(y),
            &font,
        );
        y -= 6.0;
    }

    // ── Audit entries (one page per ~30 entries) ──────────────────────────────
    layer.use_text("Audit Log Entries", 12.0, Mm(15.0), Mm(y - 8.0), &font_bold);
    y -= 18.0;

    let mut current_layer = layer;
    let mut current_page_y = y;

    for entry in entries {
        // Start a new page when we run out of space.
        if current_page_y < 20.0 {
            let (new_page, new_layer) = doc.add_page(Mm(210.0), Mm(297.0), "Layer 1");
            current_layer = doc.get_page(new_page).get_layer(new_layer);
            current_page_y = 280.0;
        }

        let line = format!(
            "[{}] {} | {} | {}/{} | {}",
            if entry.success { "OK" } else { "FAIL" },
            entry.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
            format!("{:?}", entry.action),
            entry.namespace,
            entry.resource,
            entry.actor,
        );
        current_layer.use_text(line, 7.5, Mm(15.0), Mm(current_page_y), &font);
        current_page_y -= 5.5;

        if let Some(err) = &entry.error {
            if current_page_y < 20.0 {
                let (new_page, new_layer) = doc.add_page(Mm(210.0), Mm(297.0), "Layer 1");
                current_layer = doc.get_page(new_page).get_layer(new_layer);
                current_page_y = 280.0;
            }
            current_layer.use_text(
                format!("  Error: {err}"),
                7.0,
                Mm(18.0),
                Mm(current_page_y),
                &font,
            );
            current_page_y -= 5.0;
        }
    }

    // Serialise to bytes.
    let mut buf = BufWriter::new(Vec::new());
    doc.save(&mut buf)
        .map_err(|e| Error::InternalError(format!("PDF save failed: {e}")))?;
    buf.into_inner()
        .map_err(|e| Error::InternalError(format!("PDF buffer flush failed: {e}")))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::audit_log::{AdminAction, AuditEntry};

    fn sample_entries() -> Vec<AuditEntry> {
        vec![
            AuditEntry::new(
                AdminAction::NodeCreate,
                "ci-bot",
                "validator-1",
                "stellar-system",
                Some("created via CI"),
            ),
            {
                let mut e = AuditEntry::new(
                    AdminAction::NodeDelete,
                    "admin",
                    "validator-2",
                    "stellar-system",
                    None,
                );
                e.success = false;
                e.error = Some("permission denied".into());
                e
            },
            AuditEntry::new(
                AdminAction::ConfigUpdate,
                "operator",
                "stellar-operator-config",
                "stellar-system",
                Some(r#"{"enable_dr":"true"}"#),
            ),
        ]
    }

    // ── summarise ─────────────────────────────────────────────────────────────

    #[test]
    fn summarise_counts_correctly() {
        let entries = sample_entries();
        let s = summarise(&entries);
        assert_eq!(s.total_actions, 3);
        assert_eq!(s.successful_actions, 2);
        assert_eq!(s.failed_actions, 1);
        assert!(s.unique_actors.contains(&"ci-bot".to_string()));
        assert!(s.unique_actors.contains(&"admin".to_string()));
        assert!(s.unique_actors.contains(&"operator".to_string()));
    }

    #[test]
    fn summarise_empty() {
        let s = summarise(&[]);
        assert_eq!(s.total_actions, 0);
        assert_eq!(s.failed_actions, 0);
        assert!(s.unique_actors.is_empty());
    }

    // ── export_json ───────────────────────────────────────────────────────────

    #[test]
    fn export_json_is_valid_and_signed() {
        let entries = sample_entries();
        let bytes = export_json(&entries).expect("export_json failed");

        let envelope: SignedExport =
            serde_json::from_slice(&bytes).expect("envelope not valid JSON");

        // Payload integrity
        assert_eq!(envelope.payload.entries.len(), 3);
        assert_eq!(
            envelope.payload.operator_version,
            env!("CARGO_PKG_VERSION")
        );

        // SHA-256 field is a 64-char hex string
        assert_eq!(envelope.sha256.len(), 64);

        // Signature and public key are non-empty hex strings
        assert!(!envelope.signature.is_empty());
        assert!(!envelope.public_key.is_empty());

        // Verify the signature manually
        use ed25519_dalek::{Signature, VerifyingKey};
        let pubkey_bytes: [u8; 32] = hex::decode(&envelope.public_key)
            .expect("pubkey not hex")
            .try_into()
            .expect("pubkey wrong length");
        let verifying_key =
            VerifyingKey::from_bytes(&pubkey_bytes).expect("invalid verifying key");

        let sig_bytes: [u8; 64] = hex::decode(&envelope.signature)
            .expect("sig not hex")
            .try_into()
            .expect("sig wrong length");
        let signature = Signature::from_bytes(&sig_bytes);

        let digest_bytes = hex::decode(&envelope.sha256).expect("sha256 not hex");
        verifying_key
            .verify_strict(&digest_bytes, &signature)
            .expect("signature verification failed");
    }

    #[test]
    fn export_json_empty_entries() {
        let bytes = export_json(&[]).expect("export_json failed on empty");
        let envelope: SignedExport = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(envelope.payload.entries.len(), 0);
        assert_eq!(envelope.payload.summary.total_actions, 0);
    }

    #[test]
    fn export_json_tamper_detection() {
        use ed25519_dalek::{Signature, VerifyingKey};

        let bytes = export_json(&sample_entries()).unwrap();
        let mut envelope: SignedExport = serde_json::from_slice(&bytes).unwrap();

        // Tamper with the payload
        envelope.payload.summary.total_actions = 999;

        // Re-serialise the tampered payload and recompute digest
        let tampered_payload_bytes = serde_json::to_vec(&envelope.payload).unwrap();
        let tampered_digest = Sha256::digest(&tampered_payload_bytes);

        // The original signature should NOT verify against the tampered digest
        let pubkey_bytes: [u8; 32] = hex::decode(&envelope.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes).unwrap();
        let sig_bytes: [u8; 64] = hex::decode(&envelope.signature)
            .unwrap()
            .try_into()
            .unwrap();
        let signature = Signature::from_bytes(&sig_bytes);

        assert!(
            verifying_key
                .verify_strict(tampered_digest.as_slice(), &signature)
                .is_err(),
            "tampered payload should fail signature verification"
        );
    }

    // ── export_pdf ────────────────────────────────────────────────────────────

    #[test]
    fn export_pdf_produces_pdf_bytes() {
        let bytes = export_pdf(&sample_entries()).expect("export_pdf failed");
        // PDF files start with the magic bytes %PDF
        assert!(bytes.starts_with(b"%PDF"), "output is not a PDF");
        assert!(bytes.len() > 1024, "PDF is suspiciously small");
    }

    #[test]
    fn export_pdf_empty_entries() {
        let bytes = export_pdf(&[]).expect("export_pdf failed on empty");
        assert!(bytes.starts_with(b"%PDF"));
    }
}
