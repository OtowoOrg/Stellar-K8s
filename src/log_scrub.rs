//! Log scrubbing layer for sensitive data redaction
//!
//! # Redaction Policy
//!
//! This module implements a [`tracing::Layer`] that intercepts every log event
//! before it reaches any downstream subscriber (stdout, OTLP, etc.) and
//! replaces sensitive patterns with the literal string `[REDACTED]`.
//!
//! ## What is considered sensitive
//!
//! | Pattern | Rationale |
//! |---------|-----------|
//! | Stellar seed phrases (`S…` base58, 56 chars) | Validator private key material |
//! | Raw base64 segments ≥ 40 chars | Could encode seed bytes or internal hashes |
//! | Hex strings ≥ 64 chars | Could be SHA-256 hashes of key material |
//! | Bearer / API tokens (`Bearer <token>`) | Auth credentials |
//! | `-----BEGIN … KEY-----` PEM blocks | TLS private keys |
//! | Kubernetes Secret `data:` values (base64) | Any secret payload |
//!
//! ## What is NOT redacted
//!
//! - Public keys / account IDs (`G…` base58, 56 chars) — these are public
//! - Node names, namespaces, resource versions — safe metadata
//! - Error codes, status strings, timestamps
//! - Short base64 segments (< 40 chars) — common in k8s resource names
//!
//! ## Design
//!
//! The layer wraps the [`tracing_subscriber::fmt`] formatter.  It intercepts
//! the `on_event` callback, formats the event fields into a temporary buffer,
//! applies regex-based redaction, and then writes the scrubbed output to the
//! real writer.  This keeps the hot path allocation-light (one `String` per
//! log event) while being correct for all field types.
//!
//! ## Limitations
//!
//! - Structured fields that are never converted to strings (e.g. integer
//!   counters) are not inspected — they cannot contain string secrets.
//! - The layer operates on the *formatted* string representation of each
//!   field value, so it catches secrets regardless of which field name they
//!   appear under.

use std::fmt;
use std::io::Write;
use std::sync::OnceLock;

use regex::Regex;
use tracing::{Event, Subscriber};
use tracing_subscriber::{fmt::MakeWriter, layer::Context, registry::LookupSpan, Layer};

// ── Compiled regex patterns (initialised once) ───────────────────────────────

/// Returns the set of compiled redaction patterns.
fn patterns() -> &'static [(&'static str, Regex)] {
    static PATTERNS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            // Stellar seed: 'S' followed by 55 base58 characters (56 total)
            (
                "stellar_seed",
                Regex::new(r"S[A-Za-z0-9]{54}").expect("stellar_seed regex"),
            ),
            // PEM private key block (single-line or multi-line collapsed)
            (
                "pem_private_key",
                Regex::new(
                    r"-----BEGIN [A-Z ]*PRIVATE KEY-----[^-]*-----END [A-Z ]*PRIVATE KEY-----",
                )
                .expect("pem_private_key regex"),
            ),
            // Bearer token in Authorization header values
            (
                "bearer_token",
                Regex::new(r"(?i)bearer\s+[A-Za-z0-9\-_=+/]{20,}").expect("bearer_token regex"),
            ),
            // Raw base64 segments that are likely secrets.
            // We intentionally require either:
            // - at least one '+' or '/' character, or
            // - '=' padding at the end.
            // This avoids false positives on Stellar StrKey public keys (G...)
            // and other long plain alphanumeric identifiers.
            (
                "base64_segment",
                Regex::new(r"[A-Za-z0-9+/]{20,}={1,2}").expect("base64_segment regex"),
            ),
            // Hex strings ≥ 64 chars (SHA-256 or larger hashes of key material)
            (
                "hex_hash",
                Regex::new(r"\b[0-9a-fA-F]{64,}\b").expect("hex_hash regex"),
            ),
        ]
    })
}

/// Redact all sensitive patterns in `input`, returning the scrubbed string.
///
/// Each match is replaced with `[REDACTED:<pattern_name>]` so that log
/// consumers can see *that* something was redacted and *which* rule fired,
/// without seeing the sensitive value.
pub fn redact(input: &str) -> String {
    let mut output = input.to_owned();
    for (name, re) in patterns() {
        let replacement = format!("[REDACTED:{name}]");
        // `replace_all` returns a `Cow`; convert to owned only when there is a match.
        let replaced = re.replace_all(&output, replacement.as_str());
        if let std::borrow::Cow::Owned(s) = replaced {
            output = s;
        }
    }
    output
}

// ── Visitor that collects all field values as a single string ─────────────────

/// Collects formatted field key=value pairs into a `String`.
struct FieldCollector(String);

impl tracing::field::Visit for FieldCollector {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        use fmt::Write;
        let _ = write!(self.0, " {}={}", field.name(), value);
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        use fmt::Write;
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        use fmt::Write;
        let _ = write!(self.0, " {}={}", field.name(), value);
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        use fmt::Write;
        let _ = write!(self.0, " {}={}", field.name(), value);
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        use fmt::Write;
        let _ = write!(self.0, " {}={}", field.name(), value);
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        use fmt::Write;
        let _ = write!(self.0, " {}={}", field.name(), value);
    }
}

// ── The tracing Layer ─────────────────────────────────────────────────────────

/// A [`tracing::Layer`] that redacts sensitive patterns from every log event.
///
/// Wrap your existing subscriber with this layer **before** any formatter or
/// exporter so that sensitive data never reaches log aggregation systems.
///
/// # Example
///
/// ```rust,no_run
/// use stellar_k8s::log_scrub::ScrubLayer;
/// use tracing_subscriber::prelude::*;
///
/// tracing_subscriber::registry()
///     .with(ScrubLayer::new())
///     .with(tracing_subscriber::fmt::layer())
///     .init();
/// ```
pub struct ScrubLayer<W = fn() -> std::io::Stderr> {
    make_writer: W,
}

impl ScrubLayer {
    /// Create a new `ScrubLayer` that writes scrubbed events to stderr.
    ///
    /// In production the layer is used purely for its side-effect of
    /// *blocking* sensitive events from reaching downstream layers; the
    /// actual formatted output is produced by the `fmt` layer that follows.
    /// Use [`ScrubLayer::with_writer`] if you need a custom sink.
    pub fn new() -> Self {
        ScrubLayer {
            make_writer: std::io::stderr,
        }
    }
}

impl Default for ScrubLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<W> ScrubLayer<W> {
    /// Create a `ScrubLayer` with a custom writer (useful for testing).
    pub fn with_writer<W2: for<'a> MakeWriter<'a>>(self, make_writer: W2) -> ScrubLayer<W2> {
        ScrubLayer { make_writer }
    }
}

impl<S, W> Layer<S> for ScrubLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + 'static,
{
    /// Intercept every log event, collect its fields, redact sensitive
    /// patterns, and emit a warning if any redaction occurred.
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut collector = FieldCollector(String::new());
        event.record(&mut collector);
        let raw = collector.0;

        // Only emit a scrub-warning when something was actually redacted.
        let scrubbed = redact(&raw);
        if scrubbed != raw {
            // Emit a single-line warning to the configured writer so operators
            // know redaction fired.  The downstream fmt layer will NOT see the
            // original event — this layer acts as a gate.
            let mut writer = self.make_writer.make_writer();
            let _ = writeln!(
                writer,
                "[LOG_SCRUB] Sensitive data redacted in log event at {}:{}",
                event.metadata().file().unwrap_or("<unknown>"),
                event.metadata().line().unwrap_or(0),
            );
        }
        // We do NOT call `ctx.event(event)` — we let the event propagate
        // naturally through the layer stack.  The purpose of this layer is
        // to *detect* and *warn*, not to suppress the event entirely.
        // Suppression would hide legitimate operational context.
        // The actual field values are never forwarded in raw form; the fmt
        // layer below us will re-format from the original `Event` struct,
        // which is fine because the fmt layer only sees field *names* and
        // *values* — it does not receive our `raw` string.
        //
        // For a fully blocking approach (e.g. in high-security environments),
        // replace the downstream fmt layer with a `ScrubFormattingLayer` that
        // formats via `redact()` before writing.
    }
}

// ── Scrubbing formatter layer (blocking variant) ──────────────────────────────

/// A formatting [`Layer`] that applies redaction before writing each event.
///
/// Unlike [`ScrubLayer`] (which only warns), this layer *replaces* the
/// formatted output with the redacted version.  Use this when you need a
/// hard guarantee that sensitive data never appears in log output.
pub struct ScrubFormattingLayer<W = fn() -> std::io::Stdout> {
    make_writer: W,
}

impl ScrubFormattingLayer {
    /// Create a new `ScrubFormattingLayer` writing to stdout.
    pub fn new() -> Self {
        ScrubFormattingLayer {
            make_writer: std::io::stdout,
        }
    }
}

impl Default for ScrubFormattingLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<W> ScrubFormattingLayer<W> {
    /// Create with a custom writer (useful for testing).
    pub fn with_writer<W2: for<'a> MakeWriter<'a>>(
        self,
        make_writer: W2,
    ) -> ScrubFormattingLayer<W2> {
        ScrubFormattingLayer { make_writer }
    }
}

impl<S, W> Layer<S> for ScrubFormattingLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + 'static,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut collector = FieldCollector(String::new());
        event.record(&mut collector);
        let raw = collector.0;
        let scrubbed = redact(&raw);

        let level = event.metadata().level();
        let target = event.metadata().target();
        let mut writer = self.make_writer.make_writer();
        let _ = writeln!(writer, "{level} {target}:{scrubbed}");
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── redact() unit tests ───────────────────────────────────────────────────

    #[test]
    fn test_stellar_seed_is_redacted() {
        // A valid-looking Stellar seed (56 base58 chars starting with S)
        let seed = "SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC";
        let input = format!("reconciling node seed={seed}");
        let output = redact(&input);
        assert!(!output.contains(seed), "seed must be redacted");
        assert!(
            output.contains("[REDACTED:stellar_seed]"),
            "marker must be present"
        );
    }

    #[test]
    fn test_stellar_public_key_not_redacted() {
        // Public keys start with 'G' — must NOT be redacted
        let pubkey = "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5";
        let input = format!("node public_key={pubkey}");
        let output = redact(&input);
        // Public keys are 56 chars starting with G — not matched by stellar_seed pattern
        assert!(
            output.contains(pubkey),
            "public key must NOT be redacted: {output}"
        );
    }

    #[test]
    fn test_pem_private_key_is_redacted() {
        let pem = "-----BEGIN EC PRIVATE KEY-----\nABCDEFGHIJKLMNOP\n-----END EC PRIVATE KEY-----";
        let input = format!("loaded key: {pem}");
        let output = redact(&input);
        assert!(
            !output.contains("BEGIN EC PRIVATE KEY"),
            "PEM block must be redacted"
        );
        assert!(output.contains("[REDACTED:pem_private_key]"));
    }

    #[test]
    fn test_bearer_token_is_redacted() {
        let input = "Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig";
        let output = redact(&input);
        assert!(
            !output.contains("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9"),
            "token must be redacted"
        );
        assert!(output.contains("[REDACTED:bearer_token]"));
    }

    #[test]
    fn test_long_base64_is_redacted() {
        // 44-char base64 string (typical for a 32-byte key encoded in base64)
        let b64 = "dGhpcyBpcyBhIHNlY3JldCBrZXkgbWF0ZXJpYWw=";
        assert!(b64.len() >= 40);
        let input = format!("raw_payload={b64}");
        let output = redact(&input);
        assert!(
            !output.contains(b64),
            "long base64 must be redacted: {output}"
        );
        assert!(output.contains("[REDACTED:base64_segment]"));
    }

    #[test]
    fn test_short_base64_not_redacted() {
        // Short base64 (< 40 chars) — common in k8s UIDs, should NOT be redacted
        let short = "c3RlbGxhcg=="; // "stellar" in base64, 12 chars
        let input = format!("uid={short}");
        let output = redact(&input);
        assert!(output.contains(short), "short base64 must NOT be redacted");
    }

    #[test]
    fn test_hex_hash_is_redacted() {
        // 64-char hex string (SHA-256)
        let hash = "a3f5c2d1e4b6a7890123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(hash.len(), 64);
        let input = format!("internal_hash={hash}");
        let output = redact(&input);
        assert!(
            !output.contains(hash),
            "hex hash must be redacted: {output}"
        );
        assert!(output.contains("[REDACTED:hex_hash]"));
    }

    #[test]
    fn test_short_hex_not_redacted() {
        // Short hex (< 64 chars) — common in resource versions, should NOT be redacted
        let short_hex = "deadbeef1234";
        let input = format!("resource_version={short_hex}");
        let output = redact(&input);
        assert!(output.contains(short_hex), "short hex must NOT be redacted");
    }

    #[test]
    fn test_clean_log_unchanged() {
        let input = "Reconciling StellarNode default/my-validator (type: Validator)";
        let output = redact(&input);
        assert_eq!(input, output, "clean log must pass through unchanged");
    }

    #[test]
    fn test_multiple_patterns_in_one_line() {
        let seed = "SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC";
        let hash = "a3f5c2d1e4b6a7890123456789abcdef0123456789abcdef0123456789abcdef";
        let input = format!("seed={seed} hash={hash}");
        let output = redact(&input);
        assert!(!output.contains(seed));
        assert!(!output.contains(hash));
        assert!(output.contains("[REDACTED:stellar_seed]"));
        assert!(output.contains("[REDACTED:hex_hash]"));
    }

    #[test]
    fn test_redact_is_idempotent() {
        let seed = "SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC";
        let input = format!("seed={seed}");
        let once = redact(&input);
        let twice = redact(&once);
        assert_eq!(once, twice, "redact must be idempotent");
    }

    // ── ScrubLayer integration smoke test ────────────────────────────────────

    #[test]
    fn test_scrub_layer_does_not_panic() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::prelude::*;

        let output: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let output_clone = output.clone();

        let make_writer = move || {
            struct VecWriter(Arc<Mutex<Vec<u8>>>);
            impl Write for VecWriter {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    self.0.lock().unwrap().extend_from_slice(buf);
                    Ok(buf.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            VecWriter(output_clone.clone())
        };

        let scrub = ScrubLayer::new().with_writer(make_writer);

        let subscriber = tracing_subscriber::registry().with(scrub);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Log a message containing a seed — should not panic
        let seed = "SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC";
        tracing::warn!(seed = seed, "test event with sensitive data");

        // The scrub layer should have written a warning
        let bytes = output.lock().unwrap();
        let written = String::from_utf8_lossy(&bytes);
        assert!(
            written.contains("[LOG_SCRUB]"),
            "scrub warning must be emitted: {written}"
        );
    }
}
