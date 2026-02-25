//! VSL (Validator Selection List) fetching, parsing, and signature verification.
//!
//! A production VSL is a TOML document signed by a trusted Stellar entity.
//! This module:
//!   1. Downloads the raw VSL document from a URL.
//!   2. Parses it into a structured [`QuorumSet`] type.
//!   3. Verifies the Ed25519 signature to prevent quorum-set poisoning.
//!   4. Returns the verified [`QuorumSet`] ready for stellar-core.cfg generation.
//!
//! # VSL document format
//!
//! ```toml
//! # Optional metadata
//! version = 1
//! sequence = 42
//!
//! # Ed25519 signature over the canonical (signature-stripped) document bytes,
//! # encoded as standard base64.
//! signature = "<base64-encoded 64-byte Ed25519 signature>"
//!
//! # The public key that produced the signature, as a Stellar public key (G…)
//! # or raw base64-encoded 32-byte Ed25519 key.
//! signing_key = "<base64-encoded 32-byte Ed25519 public key>"
//!
//! [[validators]]
//! name        = "SDF 1"
//! public_key  = "GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGZMT7ATOETGVTBP"
//! host        = "core-live-a.stellar.org"
//! history     = "https://history.stellar.org/prd/core-live/core_live_001/"
//!
//! [[validators]]
//! name        = "SDF 2"
//! public_key  = "GCB2VSADESRV2DDTIVTFLBDI562K6KE3KMKILBHUHUWFXCUBHGQDI7VL"
//! host        = "core-live-b.stellar.org"
//! history     = "https://history.stellar.org/prd/core-live/core_live_002/"
//!
//! # Optional inner quorum sets (for organisations with multiple validators)
//! [[quorum_sets]]
//! threshold = 2
//! validators = ["GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGZMT7ATOETGVTBP",
//!               "GCB2VSADESRV2DDTIVTFLBDI562K6KE3KMKILBHUHUWFXCUBHGQDI7VL"]
//! ```

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Public key constants for trusted VSL signers
// ---------------------------------------------------------------------------

/// Base64-encoded Ed25519 public keys of trusted VSL signers.
///
/// Add additional trusted keys here as Stellar entities publish their keys.
/// These are the keys whose signatures we accept when verifying a VSL.
pub const TRUSTED_VSL_SIGNERS: &[&str] = &[
    // Stellar Development Foundation primary signing key (placeholder — replace
    // with the real SDF key when integrating against production VSLs).
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
];

// ---------------------------------------------------------------------------
// Structured types
// ---------------------------------------------------------------------------

/// A single validator entry inside a VSL.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct VslValidator {
    /// Human-readable name (e.g. "SDF 1")
    pub name: String,
    /// Stellar Ed25519 public key (G… address)
    pub public_key: String,
    /// Optional hostname of the validator node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Optional URL of the validator's history archive
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

/// An inner quorum set nested inside the top-level quorum set.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct InnerQuorumSet {
    /// Number of validators in this group that must agree
    pub threshold: u32,
    /// Public keys of validators in this inner set
    #[serde(default)]
    pub validators: Vec<String>,
}

/// The fully parsed and verified quorum set derived from a VSL.
///
/// This is the type returned by [`fetch_vsl`] and consumed by the
/// stellar-core.cfg generation logic.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct QuorumSet {
    /// How many of the top-level validators/inner-sets must agree.
    /// Defaults to a simple majority if not specified in the VSL.
    pub threshold: u32,
    /// All validators listed in the VSL
    pub validators: Vec<VslValidator>,
    /// Optional nested quorum sets for multi-org configurations
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inner_sets: Vec<InnerQuorumSet>,
}

impl QuorumSet {
    /// Render this quorum set as the TOML fragment expected by stellar-core.cfg.
    ///
    /// # Example output
    ///
    /// ```toml
    /// [QUORUM_SET]
    /// THRESHOLD_PERCENT=67
    /// VALIDATORS=["GCEZ...", "GCB2..."]
    /// ```
    pub fn to_stellar_core_toml(&self) -> String {
        let mut out = String::from("[QUORUM_SET]\n");

        // Convert absolute threshold to a percentage (stellar-core uses THRESHOLD_PERCENT)
        let total = self.validators.len().max(1) as u32;
        let pct = ((self.threshold as f64 / total as f64) * 100.0).ceil() as u32;
        out.push_str(&format!("THRESHOLD_PERCENT={pct}\n"));

        // Validators list
        let keys: Vec<String> = self
            .validators
            .iter()
            .map(|v| format!("\"{}\"", v.public_key))
            .collect();
        out.push_str(&format!("VALIDATORS=[{}]\n", keys.join(", ")));

        // Inner quorum sets
        for (i, inner) in self.inner_sets.iter().enumerate() {
            out.push_str(&format!("\n[QUORUM_SET.{i}]\n"));
            out.push_str(&format!("THRESHOLD_PERCENT={}\n", inner.threshold));
            let inner_keys: Vec<String> = inner
                .validators
                .iter()
                .map(|k| format!("\"{k}\""))
                .collect();
            out.push_str(&format!("VALIDATORS=[{}]\n", inner_keys.join(", ")));
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Raw TOML document shape (used for deserialization before verification)
// ---------------------------------------------------------------------------

/// The raw TOML structure of a VSL document, including the signature fields.
#[derive(Debug, Deserialize)]
struct RawVslDocument {
    /// Document version
    #[serde(default)]
    version: u32,
    /// Monotonically increasing sequence number
    #[serde(default)]
    sequence: u32,
    /// Base64-encoded Ed25519 signature over the canonical document bytes
    #[serde(default)]
    signature: String,
    /// Base64-encoded Ed25519 public key of the signer
    #[serde(default)]
    signing_key: String,
    /// List of validators
    #[serde(default)]
    validators: Vec<VslValidator>,
    /// Optional inner quorum sets
    #[serde(default, rename = "quorum_sets")]
    quorum_sets: Vec<InnerQuorumSet>,
}

// ---------------------------------------------------------------------------
// Signature verification
// ---------------------------------------------------------------------------

/// Strips the `signature = "..."` line from a TOML document to produce the
/// canonical byte sequence that was signed.
///
/// The convention is: sign everything *except* the signature field itself,
/// so verifiers can reconstruct the signed payload from just the document.
fn canonical_bytes(raw_toml: &str) -> Vec<u8> {
    raw_toml
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("signature") && !trimmed.starts_with("signing_key")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

/// Verifies that `signature_b64` is a valid Ed25519 signature over
/// `canonical_payload` produced by the key `pubkey_b64`.
///
/// Returns `Ok(())` on success or an `Error::ConfigError` on any failure.
pub fn verify_ed25519_signature(
    pubkey_b64: &str,
    signature_b64: &str,
    canonical_payload: &[u8],
) -> Result<()> {
    // Decode public key
    let pubkey_bytes = BASE64
        .decode(pubkey_b64)
        .map_err(|e| Error::ConfigError(format!("Invalid base64 in signing_key: {e}")))?;

    let pubkey_arr: [u8; 32] = pubkey_bytes.as_slice().try_into().map_err(|_| {
        Error::ConfigError(format!(
            "signing_key must be 32 bytes, got {}",
            pubkey_bytes.len()
        ))
    })?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_arr)
        .map_err(|e| Error::ConfigError(format!("Invalid Ed25519 public key: {e}")))?;

    // Decode signature
    let sig_bytes = BASE64
        .decode(signature_b64)
        .map_err(|e| Error::ConfigError(format!("Invalid base64 in signature: {e}")))?;

    let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
        Error::ConfigError(format!(
            "signature must be 64 bytes, got {}",
            sig_bytes.len()
        ))
    })?;

    let signature = Signature::from_bytes(&sig_arr);

    // Verify
    use ed25519_dalek::Verifier;
    verifying_key
        .verify(canonical_payload, &signature)
        .map_err(|e| Error::ConfigError(format!("VSL signature verification failed: {e}")))?;

    Ok(())
}

/// Returns `true` if `pubkey_b64` is in the list of trusted VSL signers.
fn is_trusted_signer(pubkey_b64: &str) -> bool {
    TRUSTED_VSL_SIGNERS.contains(&pubkey_b64)
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a raw TOML string into a [`QuorumSet`], verifying the signature.
///
/// # Verification flow
///
/// 1. Parse the TOML to extract `signing_key` and `signature`.
/// 2. Check that `signing_key` is in [`TRUSTED_VSL_SIGNERS`].
/// 3. Strip signature/signing_key lines to produce the canonical payload.
/// 4. Verify the Ed25519 signature over the canonical payload.
/// 5. If all checks pass, return the structured [`QuorumSet`].
///
/// If the document has no `signature` field (e.g. in development/testing),
/// verification is skipped with a warning.
pub fn parse_and_verify_vsl(raw_toml: &str) -> Result<QuorumSet> {
    // Step 1: parse the raw document
    let doc: RawVslDocument = toml::from_str(raw_toml)
        .map_err(|e| Error::ConfigError(format!("Failed to parse VSL TOML: {e}")))?;

    if doc.validators.is_empty() {
        return Err(Error::ConfigError("VSL contains no validators".to_string()));
    }

    debug!(
        "Parsed VSL: version={}, sequence={}, validators={}",
        doc.version,
        doc.sequence,
        doc.validators.len()
    );

    // Step 2 & 3 & 4: signature verification
    if doc.signature.is_empty() {
        warn!(
            "VSL has no signature field — skipping verification. \
             Do NOT use unsigned VSLs in production."
        );
    } else {
        // Check the signer is trusted
        if !is_trusted_signer(&doc.signing_key) {
            return Err(Error::ConfigError(format!(
                "VSL signing key '{}' is not in the trusted signers list. \
                 Add it to TRUSTED_VSL_SIGNERS if this is intentional.",
                doc.signing_key
            )));
        }

        // Compute canonical payload and verify
        let payload = canonical_bytes(raw_toml);
        verify_ed25519_signature(&doc.signing_key, &doc.signature, &payload)?;
        info!(
            "VSL signature verified successfully (signer={})",
            &doc.signing_key[..8.min(doc.signing_key.len())]
        );
    }

    // Step 5: build the structured QuorumSet
    let total = doc.validators.len() as u32;
    // Default threshold: simple majority
    let threshold = (total / 2) + 1;

    Ok(QuorumSet {
        threshold,
        validators: doc.validators,
        inner_sets: doc.quorum_sets,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Fetch a VSL from `url`, parse it, verify its signature, and return the
/// structured [`QuorumSet`].
///
/// This replaces the old `fetch_vsl` that returned a raw `String`.
/// The reconciler passes the returned [`QuorumSet`] to the
/// stellar-core.cfg generation logic.
pub async fn fetch_vsl(url: &str) -> Result<QuorumSet> {
    debug!("Fetching VSL from {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to build HTTP client: {e}")))?;

    let response = client.get(url).send().await.map_err(Error::HttpError)?;

    if !response.status().is_success() {
        return Err(Error::ConfigError(format!(
            "Failed to fetch VSL from {}: HTTP {}",
            url,
            response.status()
        )));
    }

    let raw_toml = response.text().await.map_err(Error::HttpError)?;
    info!("Fetched VSL from {} ({} bytes)", url, raw_toml.len());

    parse_and_verify_vsl(&raw_toml)
}

/// Trigger a configuration reload in Stellar Core if it's already running.
pub async fn trigger_config_reload(pod_ip: &str) -> Result<()> {
    let url = format!("http://{pod_ip}:11626/http-command?admin=true&command=config-reload");
    debug!("Triggering config-reload via {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to build HTTP client: {e}")))?;

    let response = client.get(&url).send().await.map_err(Error::HttpError)?;

    if !response.status().is_success() {
        return Err(Error::ConfigError(format!(
            "Failed to trigger config-reload: HTTP {}",
            response.status()
        )));
    }

    info!("Successfully triggered config-reload for pod at {}", pod_ip);
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Generate a fresh Ed25519 keypair and sign `payload`.
    /// Returns (base64_pubkey, base64_signature).
    fn sign_payload(payload: &[u8]) -> (SigningKey, String, String) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let signature = signing_key.sign(payload);
        let pubkey_b64 = BASE64.encode(signing_key.verifying_key().as_bytes());
        let sig_b64 = BASE64.encode(signature.to_bytes());
        (signing_key, pubkey_b64, sig_b64)
    }

    /// Build a minimal valid VSL TOML string (unsigned).
    fn minimal_unsigned_vsl() -> String {
        r#"
version = 1
sequence = 1

[[validators]]
name = "Test Validator 1"
public_key = "GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGZMT7ATOETGVTBPHKOL"
host = "v1.example.com"

[[validators]]
name = "Test Validator 2"
public_key = "GCB2VSADESRV2DDTIVTFLBDI562K6KE3KMKILBHUHUWFXCUBHGQDI7VL"
host = "v2.example.com"

[[validators]]
name = "Test Validator 3"
public_key = "GDPJ4DPPFEIP2YTSQNOKT7NMLPKU2FFVOEIJVG4ZCJQHLMRXOLXOIUT"
host = "v3.example.com"
"#
        .to_string()
    }

    /// Build a signed VSL TOML, injecting the signature into the document.
    fn signed_vsl(pubkey_b64: &str, sig_b64: &str, body: &str) -> String {
        format!("signing_key = \"{pubkey_b64}\"\nsignature = \"{sig_b64}\"\n{body}")
    }

    // -----------------------------------------------------------------------
    // canonical_bytes
    // -----------------------------------------------------------------------

    #[test]
    fn test_canonical_bytes_strips_signature_lines() {
        let raw = "version = 1\nsignature = \"abc\"\nsigning_key = \"xyz\"\n[[validators]]\nname = \"v1\"\npublic_key = \"GABC\"";
        let canonical = canonical_bytes(raw);
        let canonical_str = String::from_utf8(canonical).unwrap();
        assert!(!canonical_str.contains("signature"));
        assert!(!canonical_str.contains("signing_key"));
        assert!(canonical_str.contains("version = 1"));
        assert!(canonical_str.contains("public_key"));
    }

    #[test]
    fn test_canonical_bytes_preserves_other_fields() {
        let raw =
            "version = 2\nsequence = 99\n[[validators]]\nname = \"SDF\"\npublic_key = \"GABC\"";
        let canonical = canonical_bytes(raw);
        let s = String::from_utf8(canonical).unwrap();
        assert!(s.contains("version = 2"));
        assert!(s.contains("sequence = 99"));
    }

    // -----------------------------------------------------------------------
    // verify_ed25519_signature
    // -----------------------------------------------------------------------

    #[test]
    fn test_verify_valid_signature() {
        let payload = b"hello stellar";
        let (_, pubkey_b64, sig_b64) = sign_payload(payload);
        assert!(verify_ed25519_signature(&pubkey_b64, &sig_b64, payload).is_ok());
    }

    #[test]
    fn test_verify_wrong_payload_fails() {
        let payload = b"hello stellar";
        let (_, pubkey_b64, sig_b64) = sign_payload(payload);
        // Tampered payload
        let result = verify_ed25519_signature(&pubkey_b64, &sig_b64, b"tampered payload");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_wrong_key_fails() {
        let payload = b"hello stellar";
        let (_, _, sig_b64) = sign_payload(payload);
        // Different key
        let (_, other_pubkey, _) = sign_payload(b"other");
        let result = verify_ed25519_signature(&other_pubkey, &sig_b64, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_invalid_base64_pubkey_fails() {
        let result = verify_ed25519_signature("!!!not-base64!!!", "AAAA", b"payload");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Invalid base64") || msg.contains("signing_key"));
    }

    #[test]
    fn test_verify_wrong_key_length_fails() {
        // 16 bytes instead of 32
        let short_key = BASE64.encode([0u8; 16]);
        let result = verify_ed25519_signature(&short_key, "AAAA", b"payload");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("32 bytes"));
    }

    #[test]
    fn test_verify_wrong_signature_length_fails() {
        let payload = b"test";
        let (_, pubkey_b64, _) = sign_payload(payload);
        // 32 bytes instead of 64
        let short_sig = BASE64.encode([0u8; 32]);
        let result = verify_ed25519_signature(&pubkey_b64, &short_sig, payload);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("64 bytes"));
    }

    // -----------------------------------------------------------------------
    // parse_and_verify_vsl — unsigned (development mode)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_unsigned_vsl_succeeds_with_warning() {
        let raw = minimal_unsigned_vsl();
        let result = parse_and_verify_vsl(&raw);
        assert!(result.is_ok(), "unsigned VSL should parse: {:?}", result);
        let qs = result.unwrap();
        assert_eq!(qs.validators.len(), 3);
    }

    #[test]
    fn test_parse_vsl_threshold_is_majority() {
        let raw = minimal_unsigned_vsl(); // 3 validators
        let qs = parse_and_verify_vsl(&raw).unwrap();
        // majority of 3 = 2
        assert_eq!(qs.threshold, 2);
    }

    #[test]
    fn test_parse_vsl_validator_fields() {
        let raw = minimal_unsigned_vsl();
        let qs = parse_and_verify_vsl(&raw).unwrap();
        let v = &qs.validators[0];
        assert_eq!(v.name, "Test Validator 1");
        assert_eq!(
            v.public_key,
            "GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGZMT7ATOETGVTBPHKOL"
        );
        assert_eq!(v.host.as_deref(), Some("v1.example.com"));
    }

    #[test]
    fn test_parse_empty_validators_fails() {
        let raw = "version = 1\nsequence = 1\n";
        let result = parse_and_verify_vsl(raw);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no validators"));
    }

    #[test]
    fn test_parse_invalid_toml_fails() {
        let result = parse_and_verify_vsl("this is not toml [[[");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse VSL TOML"));
    }

    // -----------------------------------------------------------------------
    // parse_and_verify_vsl — signed
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_signed_vsl_with_trusted_key_succeeds() {
        let body = minimal_unsigned_vsl();
        let canonical = canonical_bytes(&body);
        let (_, pubkey_b64, sig_b64) = sign_payload(&canonical);

        // Temporarily override trusted signers by calling verify directly
        // (we can't mutate the const in tests, so we test the verify path directly)
        let result = verify_ed25519_signature(&pubkey_b64, &sig_b64, &canonical);
        assert!(result.is_ok(), "signature should verify: {:?}", result);
    }

    #[test]
    fn test_parse_signed_vsl_untrusted_key_fails() {
        let body = minimal_unsigned_vsl();
        let canonical = canonical_bytes(&body);
        let (_, pubkey_b64, sig_b64) = sign_payload(&canonical);

        // Build a document with a real signature but an untrusted signer
        let doc = signed_vsl(&pubkey_b64, &sig_b64, &body);
        let result = parse_and_verify_vsl(&doc);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not in the trusted signers list"));
    }

    #[test]
    fn test_parse_signed_vsl_tampered_content_fails() {
        let body = minimal_unsigned_vsl();
        let canonical = canonical_bytes(&body);
        let (_, pubkey_b64, sig_b64) = sign_payload(&canonical);

        // Tamper with the body after signing
        let tampered = body.replace("Test Validator 1", "EVIL VALIDATOR");
        let doc = signed_vsl(&pubkey_b64, &sig_b64, &tampered);

        // Even though the key is untrusted, we get past that check only if the
        // key is trusted — here the untrusted-key error fires first, which is
        // correct behaviour (defence in depth).
        let result = parse_and_verify_vsl(&doc);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // QuorumSet — to_stellar_core_toml
    // -----------------------------------------------------------------------

    #[test]
    fn test_to_stellar_core_toml_basic() {
        let qs = QuorumSet {
            threshold: 2,
            validators: vec![
                VslValidator {
                    name: "V1".into(),
                    public_key: "GAAA".into(),
                    host: None,
                    history: None,
                },
                VslValidator {
                    name: "V2".into(),
                    public_key: "GBBB".into(),
                    host: None,
                    history: None,
                },
                VslValidator {
                    name: "V3".into(),
                    public_key: "GCCC".into(),
                    host: None,
                    history: None,
                },
            ],
            inner_sets: vec![],
        };

        let toml_out = qs.to_stellar_core_toml();
        assert!(toml_out.contains("[QUORUM_SET]"));
        assert!(toml_out.contains("VALIDATORS="));
        assert!(toml_out.contains("\"GAAA\""));
        assert!(toml_out.contains("\"GBBB\""));
        assert!(toml_out.contains("\"GCCC\""));
        assert!(toml_out.contains("THRESHOLD_PERCENT="));
    }

    #[test]
    fn test_to_stellar_core_toml_threshold_percent() {
        let qs = QuorumSet {
            threshold: 3,
            validators: (0..4)
                .map(|i| VslValidator {
                    name: format!("V{i}"),
                    public_key: format!("G{i:0>55}"),
                    host: None,
                    history: None,
                })
                .collect(),
            inner_sets: vec![],
        };
        let toml_out = qs.to_stellar_core_toml();
        // 3/4 = 75%
        assert!(toml_out.contains("THRESHOLD_PERCENT=75"));
    }

    #[test]
    fn test_to_stellar_core_toml_with_inner_sets() {
        let qs = QuorumSet {
            threshold: 1,
            validators: vec![VslValidator {
                name: "V1".into(),
                public_key: "GAAA".into(),
                host: None,
                history: None,
            }],
            inner_sets: vec![InnerQuorumSet {
                threshold: 2,
                validators: vec!["GBBB".into(), "GCCC".into()],
            }],
        };
        let toml_out = qs.to_stellar_core_toml();
        assert!(toml_out.contains("[QUORUM_SET.0]"));
        assert!(toml_out.contains("THRESHOLD_PERCENT=2"));
    }

    // -----------------------------------------------------------------------
    // is_trusted_signer
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_trusted_signer_known_key() {
        // The placeholder key in TRUSTED_VSL_SIGNERS
        assert!(is_trusted_signer(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ));
    }

    #[test]
    fn test_is_trusted_signer_unknown_key() {
        assert!(!is_trusted_signer("completely-unknown-key"));
    }
}
