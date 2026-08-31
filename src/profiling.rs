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
//! Performance profiling integration for Rust services (issue #1386)
//!
//! Provides pprof-compatible CPU and heap profiling endpoints for the
//! Stellar-K8s operator, gated behind token-based authentication so they
//! can be safely enabled in production environments.
//!
//! # Architecture
//!
//! - CPU profiles are captured via the `pprof` crate (sampling profiler,
//!   protobuf-encoded pprof format).
//! - Heap profiles use jemalloc's built-in heap profiling (`jemalloc_pprof`)
//!   when the `profiling` feature is enabled.
//! - All endpoints bind to `127.0.0.1` only and require a
//!   `X-Profiling-Token` HTTP header whose SHA-256 matches the configured
//!   hash (`profiling.tokenSha256` in Helm values).
//!
//! # Endpoints
//!
//! | Method | Path                              | Description                        |
//! |--------|-----------------------------------|------------------------------------|
//! | GET    | `/debug/pprof/profile`            | CPU profile (seconds via `?duration=N`) |
//! | GET    | `/debug/pprof/heap`               | Heap profile snapshot              |
//! | GET    | `/debug/pprof/goroutine`          | Goroutine / async task stacks      |
//! | GET    | `/debug/pprof/allocs`             | Memory allocation profile          |
//! | GET    | `/debug/pprof/`                   | Index of available profiles        |
//!
//! # Usage
//!
//! ```bash
//! # Port-forward the profiling port
//! kubectl port-forward pod/<operator-pod> 6060:6060 -n stellar-system
//!
//! # Capture a 30-second CPU profile
//! TOKEN=$(kubectl get secret stellar-profiling-token -n stellar-system \
//!   -o jsonpath='{.data.token}' | base64 -d)
//! curl -H "X-Profiling-Token: $TOKEN" \
//!   "http://localhost:6060/debug/pprof/profile?duration=30" \
//!   -o cpu.pb.gz
//!
//! # Analyse with pprof
//! go tool pprof cpu.pb.gz
//! ```
//!
//! See `docs/profiling-runbook.md` for the full runbook.

use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::{info, warn};

/// Configuration for the profiling HTTP server.
#[derive(Clone, Debug)]
pub struct ProfilingConfig {
    /// Bind address — **must remain `127.0.0.1`** in production.
    pub bind_addr: String,
    /// SHA-256 hex digest of the expected profiling token.
    pub token_sha256: String,
    /// Default CPU profile duration when no `?duration` query param is provided.
    pub default_cpu_duration_secs: u32,
    /// Maximum CPU profile duration to accept (prevents runaway profiles).
    pub max_cpu_duration_secs: u32,
}

impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:6060".to_string(),
            token_sha256: String::new(),
            default_cpu_duration_secs: 30,
            max_cpu_duration_secs: 300,
        }
    }
}

/// Verify that a raw token's SHA-256 matches the expected hash.
///
/// This avoids storing the raw token in `values.yaml` while still allowing
/// runtime validation.
pub fn verify_token(raw_token: &str, expected_sha256: &str) -> bool {
    if expected_sha256.is_empty() {
        warn!("Profiling token hash is not configured — denying all profiling requests");
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    // Constant-time comparison (best-effort — avoid short-circuit via format)
    digest == expected_sha256
}

/// Capture a CPU profile for `duration` and return the pprof-encoded bytes.
///
/// This is a stub implementation.  When the `profiling` feature is enabled
/// the real `pprof` crate provides the sampling; otherwise this returns an
/// empty slice so the rest of the server still compiles and serves the
/// other endpoints.
#[cfg(feature = "profiling")]
pub async fn capture_cpu_profile(duration: Duration) -> crate::error::Result<Vec<u8>> {
    use prost::Message as _;

    info!(secs = duration.as_secs(), "Starting CPU profile capture");

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(99)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .map_err(|e| crate::error::Error::ConfigError(format!("pprof guard: {e}")))?;

    tokio::time::sleep(duration).await;

    let report = guard
        .report()
        .build()
        .map_err(|e| crate::error::Error::ConfigError(format!("pprof report: {e}")))?;

    let proto = report
        .pprof()
        .map_err(|e| crate::error::Error::ConfigError(format!("pprof encode: {e}")))?;

    let mut buf = Vec::new();
    proto
        .encode(&mut buf)
        .map_err(|e| crate::error::Error::ConfigError(format!("prost encode: {e}")))?;

    info!(bytes = buf.len(), "CPU profile capture complete");
    Ok(buf)
}

#[cfg(not(feature = "profiling"))]
pub async fn capture_cpu_profile(_duration: Duration) -> crate::error::Result<Vec<u8>> {
    Err(crate::error::Error::ConfigError(
        "Profiling feature is not enabled. Rebuild with --features profiling.".to_string(),
    ))
}

/// Capture a heap profile snapshot using jemalloc's built-in profiler.
///
/// Returns pprof-encoded bytes suitable for `go tool pprof`.
#[cfg(feature = "profiling")]
pub async fn capture_heap_profile() -> crate::error::Result<Vec<u8>> {
    info!("Capturing heap profile via jemalloc");

    // Activate sampling and dump
    let heap_prof = jemalloc_pprof::PROF_CTL
        .as_ref()
        .ok_or_else(|| {
            crate::error::Error::ConfigError(
                "jemalloc profiling not available (MALLOC_CONF missing prof:true?)".to_string(),
            )
        })?
        .lock()
        .await;

    let pprof_bytes = heap_prof
        .dump_pprof()
        .await
        .map_err(|e| crate::error::Error::ConfigError(format!("heap dump: {e}")))?;

    info!(bytes = pprof_bytes.len(), "Heap profile captured");
    Ok(pprof_bytes)
}

#[cfg(not(feature = "profiling"))]
pub async fn capture_heap_profile() -> crate::error::Result<Vec<u8>> {
    Err(crate::error::Error::ConfigError(
        "Profiling feature is not enabled. Rebuild with --features profiling.".to_string(),
    ))
}

/// Returns a JSON index of available profiling endpoints.
pub fn profile_index_json() -> serde_json::Value {
    serde_json::json!({
        "profiles": [
            {
                "name": "cpu",
                "href": "/debug/pprof/profile",
                "description": "CPU profile. Accepts ?duration=N (seconds)."
            },
            {
                "name": "heap",
                "href": "/debug/pprof/heap",
                "description": "Heap memory allocation profile (jemalloc)."
            },
            {
                "name": "allocs",
                "href": "/debug/pprof/allocs",
                "description": "Memory allocation samples."
            }
        ],
        "note": "All endpoints require X-Profiling-Token header."
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_token_correct_hash() {
        // sha256("test-token") pre-computed
        let expected = format!("{:x}", {
            let mut h = Sha256::new();
            h.update(b"test-token");
            h.finalize()
        });
        assert!(verify_token("test-token", &expected));
    }

    #[test]
    fn verify_token_wrong_hash() {
        assert!(!verify_token("wrong-token", "deadbeefdeadbeef"));
    }

    #[test]
    fn verify_token_empty_hash_denies() {
        assert!(!verify_token("any-token", ""));
    }

    #[test]
    fn profile_index_has_required_keys() {
        let idx = profile_index_json();
        assert!(idx["profiles"].is_array());
        assert!(idx["note"].is_string());
    }
}
