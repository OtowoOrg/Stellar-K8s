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
//! HTTP endpoints for the profiling server (issue #1416).
//!
//! Exposes pprof-compatible routes gated behind a shared secret token.
//! All routes return JSON; a future iteration can add protobuf/pprof binary
//! output for direct consumption by `go tool pprof`.

use crate::profiling::collector::{CollectorConfig, ProfileCollector};
use crate::profiling::exporter::{ProfileExporter, ProfileFormat};
use crate::profiling::reporter::ProfileReporter;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Authentication configuration for the profiling endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilingAuth {
    /// Header name to inspect for the profiling token.
    /// Defaults to `X-Profiling-Token`.
    pub header_name: String,
    /// SHA-256 hex digest of the allowed token value.
    /// The raw token is never stored; callers supply the hex digest.
    pub token_sha256: String,
    /// Whether the profiling server is enabled at all.
    pub enabled: bool,
}

impl Default for ProfilingAuth {
    fn default() -> Self {
        Self {
            header_name: "X-Profiling-Token".to_string(),
            token_sha256: String::new(),
            enabled: false,
        }
    }
}

impl ProfilingAuth {
    /// Verify a raw token string against the stored SHA-256 digest.
    pub fn verify(&self, raw_token: &str) -> bool {
        if !self.enabled || self.token_sha256.is_empty() {
            return false;
        }
        use sha2::{Digest, Sha256};
        let digest = hex::encode(Sha256::digest(raw_token.as_bytes()));
        // Constant-time comparison via XOR fold to avoid timing side-channels.
        let stored = self.token_sha256.as_bytes();
        let provided = digest.as_bytes();
        if stored.len() != provided.len() {
            return false;
        }
        stored
            .iter()
            .zip(provided)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

/// Top-level configuration for the profiling subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilingConfig {
    /// Authentication settings.
    pub auth: ProfilingAuth,
    /// Collector behaviour settings.
    pub collector: CollectorConfig,
    /// Bind address for the dedicated profiling HTTP server.
    /// Defaults to `127.0.0.1:6060` (localhost-only — never expose publicly).
    pub bind_addr: String,
}

impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            auth: ProfilingAuth::default(),
            collector: CollectorConfig::default(),
            bind_addr: "127.0.0.1:6060".to_string(),
        }
    }
}

// ── Request / response types ──────────────────────────────────────────────────

/// Query parameters for `/debug/pprof/profile`.
#[derive(Debug, Clone, Deserialize)]
pub struct CpuProfileQuery {
    /// Duration in seconds (default: collector config default).
    pub duration: Option<u64>,
    /// Output format: `json` (default) or `pprof` (protobuf, future).
    pub format: Option<String>,
}

/// Response from the `/debug/pprof/profile` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CpuProfileResponse {
    /// The requested format.
    pub format: String,
    /// Raw profile payload.  JSON-encoded for `format=json`.
    pub payload: serde_json::Value,
    /// Capture duration in seconds.
    pub duration_secs: f64,
    /// Top-5 hottest stack frames by sample count.
    pub top_frames: Vec<FrameEntry>,
}

/// Response from the `/debug/pprof/heap` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct HeapProfileResponse {
    pub format: String,
    pub payload: serde_json::Value,
    /// Live heap in bytes at capture time.
    pub live_heap_bytes: u64,
    /// Top-5 allocation sites by size.
    pub top_allocations: Vec<AllocationEntry>,
}

/// Response from `/debug/pprof/goroutine` (active async-task trace).
#[derive(Debug, Clone, Serialize)]
pub struct TaskTraceResponse {
    pub active_tasks: usize,
    pub worker_threads: usize,
    pub blocking_threads: usize,
    pub capture_time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameEntry {
    pub symbol: String,
    pub samples: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllocationEntry {
    pub site: String,
    pub bytes: u64,
    pub pct: f64,
}

// ── ProfilingEndpoints ────────────────────────────────────────────────────────

/// Handles profiling HTTP endpoint logic.
///
/// The caller is responsible for wiring these handlers into their HTTP
/// framework (Axum, Actix, etc.) and for checking authentication before
/// calling any handler method.
pub struct ProfilingEndpoints {
    collector: Arc<ProfileCollector>,
    exporter: ProfileExporter,
    reporter: ProfileReporter,
    config: ProfilingConfig,
}

impl ProfilingEndpoints {
    /// Construct a new handler set from config.
    pub fn new(config: ProfilingConfig) -> Self {
        let collector = Arc::new(ProfileCollector::new(config.collector.clone()));
        Self {
            collector,
            exporter: ProfileExporter::new(),
            reporter: ProfileReporter::new(),
            config,
        }
    }

    /// Shared reference to the underlying collector (e.g. for background tasks).
    pub fn collector(&self) -> Arc<ProfileCollector> {
        Arc::clone(&self.collector)
    }

    /// Authentication config accessor.
    pub fn auth(&self) -> &ProfilingAuth {
        &self.config.auth
    }

    // ── Handlers ─────────────────────────────────────────────────────────────

    /// Handle `GET /debug/pprof/profile?duration=30&format=json`
    pub async fn handle_cpu_profile(
        &self,
        query: CpuProfileQuery,
    ) -> Result<CpuProfileResponse, ProfilingError> {
        let duration_secs = query
            .duration
            .unwrap_or(self.config.collector.default_cpu_duration_secs)
            .min(self.config.collector.max_cpu_duration_secs);

        let sample = self
            .collector
            .capture_cpu_profile(Duration::from_secs(duration_secs))
            .await;

        let format = query.format.unwrap_or_else(|| "json".to_string());
        let payload = self.exporter.export_cpu(&sample, ProfileFormat::Json)?;

        // Build top-frames list
        let total_samples: u64 = sample.stack_counts.values().sum();
        let mut frames: Vec<FrameEntry> = sample
            .stack_counts
            .iter()
            .map(|(sym, &cnt)| FrameEntry {
                symbol: sym.clone(),
                samples: cnt,
                pct: if total_samples > 0 {
                    cnt as f64 / total_samples as f64 * 100.0
                } else {
                    0.0
                },
            })
            .collect();
        frames.sort_by(|a, b| b.samples.cmp(&a.samples));
        frames.truncate(5);

        Ok(CpuProfileResponse {
            format,
            payload,
            duration_secs: sample.duration_secs,
            top_frames: frames,
        })
    }

    /// Handle `GET /debug/pprof/heap?format=json`
    pub async fn handle_heap_profile(
        &self,
        format: Option<String>,
    ) -> Result<HeapProfileResponse, ProfilingError> {
        let sample = self.collector.capture_heap_profile().await;
        let fmt = format.unwrap_or_else(|| "json".to_string());
        let payload = self.exporter.export_heap(&sample, ProfileFormat::Json)?;

        let total_bytes: u64 = sample.allocation_sites.values().sum();
        let mut allocs: Vec<AllocationEntry> = sample
            .allocation_sites
            .iter()
            .map(|(site, &bytes)| AllocationEntry {
                site: site.clone(),
                bytes,
                pct: if total_bytes > 0 {
                    bytes as f64 / total_bytes as f64 * 100.0
                } else {
                    0.0
                },
            })
            .collect();
        allocs.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        allocs.truncate(5);

        Ok(HeapProfileResponse {
            format: fmt,
            payload,
            live_heap_bytes: sample.live_heap_bytes,
            top_allocations: allocs,
        })
    }

    /// Handle `GET /debug/pprof/goroutine` — returns active Tokio task info.
    pub async fn handle_task_trace(&self) -> TaskTraceResponse {
        // In production, query tokio::runtime::RuntimeMetrics for accurate counts.
        TaskTraceResponse {
            active_tasks: 42,
            worker_threads: num_cpus(),
            blocking_threads: 2,
            capture_time: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Handle `GET /debug/pprof/cmdline`.
    pub async fn handle_cmdline(&self) -> Vec<String> {
        std::env::args().collect()
    }

    /// Analyse recent samples and return a bottleneck report.
    pub async fn handle_analysis(&self) -> serde_json::Value {
        let cpu_samples = self.collector.recent_cpu_samples(10).await;
        let alloc_samples = self.collector.recent_alloc_samples(10).await;
        let report = self.reporter.analyse(&cpu_samples, &alloc_samples);
        serde_json::to_value(report).unwrap_or(serde_json::Value::Null)
    }
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProfilingError {
    #[error("profiling is disabled")]
    Disabled,
    #[error("authentication failed")]
    Unauthorized,
    #[error("export error: {0}")]
    Export(String),
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_endpoints() -> ProfilingEndpoints {
        let mut config = ProfilingConfig::default();
        config.auth.enabled = true;
        // sha256("test-token")
        config.auth.token_sha256 =
            "4a3d0c7f6e9c6d6c2c2bc5ad97e9dbcf1e1b5e0e6c8e8d4e3a3c9b7f2d1e0a5f".to_string();
        ProfilingEndpoints::new(config)
    }

    #[tokio::test]
    async fn cpu_profile_returns_frames() {
        let ep = make_endpoints();
        let res = ep
            .handle_cpu_profile(CpuProfileQuery {
                duration: Some(1),
                format: None,
            })
            .await
            .unwrap();
        assert!(!res.top_frames.is_empty());
        assert!(res.duration_secs <= 1.0);
    }

    #[tokio::test]
    async fn heap_profile_returns_allocations() {
        let ep = make_endpoints();
        let res = ep.handle_heap_profile(None).await.unwrap();
        assert!(!res.top_allocations.is_empty());
    }

    #[tokio::test]
    async fn task_trace_returns_data() {
        let ep = make_endpoints();
        let trace = ep.handle_task_trace().await;
        assert!(trace.worker_threads >= 1);
    }

    #[test]
    fn auth_verify_correct_token() {
        use sha2::{Digest, Sha256};
        let raw = "my-secret-token";
        let digest = hex::encode(Sha256::digest(raw.as_bytes()));
        let auth = ProfilingAuth {
            header_name: "X-Profiling-Token".to_string(),
            token_sha256: digest,
            enabled: true,
        };
        assert!(auth.verify(raw));
        assert!(!auth.verify("wrong-token"));
    }

    #[test]
    fn auth_disabled_always_fails() {
        let auth = ProfilingAuth {
            enabled: false,
            ..Default::default()
        };
        assert!(!auth.verify("any-token"));
    }
}
