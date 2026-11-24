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
//! check-pipeline-log-redaction — Enforce secret redaction on pipeline logs (#1153)
//!
//! Verifies that sensitive patterns which may appear in stdout/stderr from
//! pipeline commands (make targets, cargo test, kubectl dumps, CI scripts) are
//! scrubbed by [`stellar_k8s::log_scrub::redact`] before they can leak into CI
//! artifacts or aggregated logs.
//!
//! # Modes
//!
//! ```text
//! # Audit built-in fixtures + optional --fixture files (default)
//! cargo run --locked --bin check-pipeline-log-redaction
//!
//! # Report-only (always exit 0)
//! cargo run --locked --bin check-pipeline-log-redaction -- --report
//!
//! # Scrub a captured pipeline log file in place to stdout
//! cargo run --locked --bin check-pipeline-log-redaction -- --scrub path/to/job.log
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use stellar_k8s::log_scrub::redact;

/// Built-in dirty pipeline log snippets that MUST redact completely.
const DIRTY_FIXTURES: &[(&str, &str)] = &[
    (
        "make-test-seed",
        "→ Running tests...\nseed=SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC ok",
    ),
    (
        "kubectl-secret-dump",
        "kubectl get secret stellar-seed -o yaml\ndata:\n  seed: dGhpcyBpcyBhIHNlY3JldCBrZXkgbWF0ZXJpYWw=",
    ),
    (
        "ci-bearer-echo",
        "curl -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9abcdefghij' https://example.test",
    ),
    (
        "pem-in-preflight",
        "preflight tls check\n-----BEGIN EC PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgTESTKEYMATERIAL\n-----END EC PRIVATE KEY-----",
    ),
    (
        "hex-hash-artifact",
        "artifact sha256=a3f5c2d1e4b6a7890123456789abcdef0123456789abcdef0123456789abcdef",
    ),
];

/// Clean lines that must pass through unchanged.
const CLEAN_FIXTURES: &[(&str, &str)] = &[
    (
        "reconcile-meta",
        "Reconciling StellarNode default/my-validator (type: Validator)",
    ),
    (
        "public-key",
        "account=GDQNY3PBOJAIHYADDIUNISYSKEU7AKDVKER47JQWZB3U2AM6G5JRMBSC",
    ),
];

#[derive(Parser, Debug)]
#[command(
    name = "check-pipeline-log-redaction",
    about = "Enforce secret redaction checks in logs produced by pipeline commands (#1153)"
)]
struct Cli {
    /// Report findings but always exit 0.
    #[arg(long)]
    report: bool,

    /// Additional fixture files to audit (treated as dirty: secrets must not survive).
    #[arg(long = "fixture", value_name = "PATH")]
    fixtures: Vec<PathBuf>,

    /// Read a captured pipeline log, print the scrubbed form to stdout, and exit.
    #[arg(long, value_name = "PATH")]
    scrub: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Some(path) = cli.scrub.as_ref() {
        return scrub_file(path);
    }

    let mut failures = 0usize;
    println!("==> Pipeline log redaction check (issue #1153)");

    for (name, raw) in DIRTY_FIXTURES {
        match assert_dirty_redacted(name, raw) {
            Ok(msg) => println!("  ✓ {msg}"),
            Err(msg) => {
                println!("  ✗ {msg}");
                failures += 1;
            }
        }
    }

    for (name, raw) in CLEAN_FIXTURES {
        match assert_clean_passthrough(name, raw) {
            Ok(msg) => println!("  ✓ {msg}"),
            Err(msg) => {
                println!("  ✗ {msg}");
                failures += 1;
            }
        }
    }

    for path in &cli.fixtures {
        match audit_fixture_file(path) {
            Ok(msg) => println!("  ✓ {msg}"),
            Err(msg) => {
                println!("  ✗ {msg}");
                failures += 1;
            }
        }
    }

    if failures == 0 {
        println!("✓ All pipeline log redaction checks passed");
        ExitCode::SUCCESS
    } else if cli.report {
        println!("⚠ {failures} finding(s) (report-only)");
        ExitCode::SUCCESS
    } else {
        println!("✗ {failures} finding(s) — pipeline logs must not leak secrets");
        ExitCode::from(1)
    }
}

fn scrub_file(path: &Path) -> ExitCode {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            print!("{}", redact(&raw));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to read {}: {e}", path.display());
            ExitCode::from(2)
        }
    }
}

fn assert_dirty_redacted(name: &str, raw: &str) -> Result<String, String> {
    let scrubbed = redact(raw);
    if scrubbed == *raw {
        return Err(format!(
            "fixture '{name}': redact() left dirty pipeline log unchanged"
        ));
    }
    if !scrubbed.contains("[REDACTED:") {
        return Err(format!(
            "fixture '{name}': expected [REDACTED:…] marker after scrub"
        ));
    }
    // Residual leak heuristics mirroring log_scrub patterns.
    if residual_secret(&scrubbed) {
        return Err(format!(
            "fixture '{name}': scrubbed output still looks secretful:\n{scrubbed}"
        ));
    }
    Ok(format!("dirty fixture '{name}' fully redacted"))
}

fn assert_clean_passthrough(name: &str, raw: &str) -> Result<String, String> {
    let scrubbed = redact(raw);
    if scrubbed != *raw {
        return Err(format!(
            "fixture '{name}': clean pipeline log was altered:\n  before={raw}\n  after={scrubbed}"
        ));
    }
    Ok(format!("clean fixture '{name}' unchanged"))
}

fn audit_fixture_file(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("fixture {}: read error: {e}", path.display()))?;
    let scrubbed = redact(&raw);
    if residual_secret(&scrubbed) {
        return Err(format!(
            "fixture {}: secrets remain after redact()",
            path.display()
        ));
    }
    Ok(format!("file fixture {} scrubbed clean", path.display()))
}

fn residual_secret(s: &str) -> bool {
    // Mirror log_scrub stellar_seed: 'S' + 54 alphanumerics = 55 chars total
    // (see src/log_scrub.rs). Seeds may appear bare or as key=value tokens.
    if s.split_whitespace().any(|tok| {
        if tok.contains("[REDACTED") {
            return false;
        }
        let candidate = tok.rsplit('=').next().unwrap_or(tok);
        candidate.len() == 55
            && candidate.starts_with('S')
            && candidate.chars().all(|c| c.is_ascii_alphanumeric())
    }) {
        return true;
    }
    if s.to_ascii_lowercase().contains("bearer ")
        && !s.contains("[REDACTED:bearer_token]")
        && s.to_ascii_lowercase()
            .split("bearer ")
            .nth(1)
            .is_some_and(|rest| {
                rest.split_whitespace()
                    .next()
                    .is_some_and(|tok| tok.len() >= 20)
            })
    {
        return true;
    }
    if s.contains("BEGIN ")
        && s.contains("PRIVATE KEY")
        && !s.contains("[REDACTED:pem_private_key]")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_fixtures_all_redact() {
        for (name, raw) in DIRTY_FIXTURES {
            assert_dirty_redacted(name, raw).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn clean_fixtures_passthrough() {
        for (name, raw) in CLEAN_FIXTURES {
            assert_clean_passthrough(name, raw).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn pipeline_command_capture_roundtrip() {
        // Simulate a make/ci log line that accidentally echoed a seed.
        let captured = "make test\n[test] using seed=SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC\nok";
        let scrubbed = redact(captured);
        assert!(scrubbed.contains("[REDACTED:stellar_seed]"));
        assert!(!scrubbed.contains("SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC"));
        assert!(!residual_secret(&scrubbed));
    }

    #[test]
    fn residual_secret_detects_unredacted_seed() {
        assert!(residual_secret(
            "seed=SCZANGBA5RLMQ4DQTARF4VIRYOIMTUPN4MXQHZIX3BGOANFZFZQAVSC"
        ));
        assert!(!residual_secret("seed=[REDACTED:stellar_seed]"));
    }
}
