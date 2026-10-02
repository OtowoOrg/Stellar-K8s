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
//! Userspace half of the SCP eBPF sniffer.
//!
//! # Crate layout
//!
//! ```
//! security/ebpf-sniffer/
//! ├── src/
//! │   ├── bpf/
//! │   │   └── scp_trace.c      ← BPF C program (kernel side)
//! │   └── user/
//! │       ├── mod.rs           ← this file
//! │       ├── loader.rs        ← BPF object loader + perf reader + HTTP exporter
//! │       ├── metrics.rs       ← Prometheus metrics definitions
//! │       └── types.rs         ← Shared types mirroring kernel structs
//! └── Makefile                 ← Builds scp_trace.o with clang/BPF target
//! ```
//!
//! # Integration with the operator
//!
//! The operator does not import this crate directly at compile time.  Instead,
//! it communicates with the sniffer process via:
//!
//! 1. **Prometheus scraping** — the operator's `monitor_ebpf_metrics` task
//!    scrapes the sniffer's `/metrics` endpoint on port 9436.
//!
//! 2. **Shared in-memory state** (when running in-process) — `src/security/
//!    ebpf_sniffer.rs` creates a [`loader::ScpSnifferLoader`] and passes its
//!    [`loader::SnifferState`] handle into the axum router state, so handlers
//!    in `src/rest_api/ebpf_sniffer_handlers.rs` can serve the data without
//!    an extra HTTP round-trip.
//!
//! # Feature flags
//!
//! | Feature          | Effect                                          |
//! |------------------|-------------------------------------------------|
//! | `ebpf-runtime`   | Enables libbpf-sys FFI and real kprobe loading  |
//! | `simulation`     | Forces simulation mode regardless of runtime    |

pub mod loader;
pub mod metrics;
pub mod types;

pub use loader::{LoaderConfig, ScpSnifferLoader, SnifferState};
pub use metrics::ScpSnifferMetrics;
pub use types::{
    EventType, PeerDropEntry, RawScpEvent, ScpEvent, ScpNetworkHealth, XdrScpMessageType,
};
