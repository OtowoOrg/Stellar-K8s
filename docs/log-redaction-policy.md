# Log Redaction Policy

## Overview

The `stellar-operator` uses a custom `tracing::Layer` (`ScrubLayer`) to ensure
sensitive data is never emitted to log aggregation systems, even when deep
debug traces are enabled.

The layer is registered at subscriber initialisation time — before any
formatter or OTLP exporter — so it intercepts every log event regardless of
log level or target.

## What is redacted

| Pattern | Rule name | Example match |
|---------|-----------|---------------|
| Stellar seed phrase (56-char base58 starting with `S`) | `stellar_seed` | `SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC` |
| PEM private key block | `pem_private_key` | `-----BEGIN EC PRIVATE KEY-----…-----END EC PRIVATE KEY-----` |
| Bearer / API token | `bearer_token` | `Bearer eyJhbGci…` |
| Raw base64 segment ≥ 40 chars | `base64_segment` | `dGhpcyBpcyBhIHNlY3JldCBrZXkgbWF0ZXJpYWw=` |
| Hex string ≥ 64 chars | `hex_hash` | `a3f5c2d1e4b6a789…` (SHA-256) |

Each match is replaced with `[REDACTED:<rule_name>]` so that:
- Operators can see *that* redaction occurred and *which* rule fired.
- The sensitive value is never written to any output.

## What is NOT redacted

- Stellar public keys (`G…` 56-char base58) — these are public by design.
- Short base64 segments (< 40 chars) — common in Kubernetes resource names and UIDs.
- Short hex strings (< 64 chars) — common in resource versions.
- Node names, namespaces, timestamps, error codes — safe operational metadata.

## Architecture

```
tracing event
      │
      ▼
 EnvFilter (level gate)
      │
      ▼
 ScrubLayer          ← detects sensitive patterns, emits [LOG_SCRUB] warning
      │
      ▼
 fmt::Layer (JSON)   ← formats and writes to stdout
      │
      ▼
 OTLP Layer (opt.)   ← exports to collector
```

`ScrubLayer` operates on the *formatted string representation* of each field
value, so it catches secrets regardless of which field name they appear under.

For environments requiring a hard guarantee (no raw values ever written),
replace the `fmt::Layer` with `ScrubFormattingLayer` from `stellar_k8s::log_scrub`.

## Reconciler audit

All `info!`, `debug!`, `warn!`, and `error!` calls in
`src/controller/reconciler.rs` have been audited.  The following principles
are enforced:

1. **Seed values are never logged.** `kms_secret::reconcile_seed_secret` is
   documented to never read or log the seed value; only the secret *name* is
   referenced in log messages.

2. **TLS key material is never logged.** `mtls.rs` loads PEM bytes into memory
   but does not log them; only certificate metadata (expiry, subject) is logged.

3. **Auth tokens are never logged.** `cve.rs` uses bearer tokens for registry
   scanning but does not log the token value.

4. **Structured fields log only metadata.** All reconciler log calls use
   `namespace`, `name`, `node_type`, `version`, and similar non-sensitive
   identifiers.

## Testing

Unit tests for the `redact()` function and `ScrubLayer` are in
`src/log_scrub.rs` under `#[cfg(test)]`.  Run them with:

```sh
cargo test log_scrub
```
