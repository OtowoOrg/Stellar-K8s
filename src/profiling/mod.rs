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
//! Performance Profiling Integration for Rust Services (issue #1416)
//!
//! Provides CPU and memory profiling endpoints gated behind authentication
//! for production use. Profiles can be captured and exported in pprof-compatible
//! format for analysis with standard tooling (pprof, flamegraph, etc.).
//!
//! # Architecture
//!
//! ```text
//! HTTP request (authenticated)
//!   → /debug/pprof/profile     — CPU profile (duration configurable)
//!   → /debug/pprof/heap        — Heap/memory allocation profile
//!   → /debug/pprof/goroutine   — Active async task trace
//!   → /debug/pprof/cmdline     — Process command line
//!   → /debug/pprof/symbol      — Symbol lookup
//! ```
//!
//! All endpoints require a valid `X-Profiling-Token` header matching the
//! configured secret. In production this token should be a high-entropy
//! random value managed via Kubernetes Secrets.

pub mod collector;
pub mod endpoints;
pub mod exporter;
pub mod metrics;
pub mod reporter;

pub use collector::{AllocationSample, CpuSample, ProfileCollector};
pub use endpoints::{ProfilingAuth, ProfilingConfig, ProfilingEndpoints};
pub use exporter::{ProfileExporter, ProfileFormat};
pub use metrics::ProfilingMetrics;
pub use reporter::{BottleneckReport, ProfileReporter};
