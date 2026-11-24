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

//! P2P Gossip Network Threat Detection Firewall
//!
//! Monitors and filters malicious peer connections attempting to flood the
//! Stellar SCP gossip network on port 11625. Detects malformed XDR payloads,
//! spam floods, and connections failing cryptographic handshakes, then
//! automatically blacklists offending IPs via Kubernetes NetworkPolicy and
//! iptables.
//!
//! # Architecture
//!
//! ```text
//!  ┌─────────────────────────────────────────────────┐
//!  │              FirewallEngine                      │
//!  │  ┌──────────────┐   ┌────────────────────────┐  │
//!  │  │eBPF Intercept│──▶│    XdrAnalyzer          │  │
//!  │  │(port 11625)  │   │  (heuristic detection)  │  │
//!  │  └──────────────┘   └──────────┬─────────────┘  │
//!  │                                │ ThreatLevel     │
//!  │                    ┌───────────▼─────────────┐  │
//!  │                    │   BlacklistManager        │  │
//!  │                    │  (K8s NetworkPolicy +     │  │
//!  │                    │   iptables enforcement)   │  │
//!  │                    └───────────┬─────────────┘  │
//!  │                                │                 │
//!  │                    ┌───────────▼─────────────┐  │
//!  │                    │  FirewallMetrics          │  │
//!  │                    │  (Prometheus /metrics)    │  │
//!  │                    └─────────────────────────┘  │
//!  └─────────────────────────────────────────────────┘
//! ```

pub mod analyzer;
pub mod blacklist;
pub mod ebpf;
pub mod firewall;
pub mod metrics;

pub use analyzer::{AnalysisResult, PacketInfo, ThreatLevel, XdrAnalyzer};
pub use blacklist::{BlacklistEntry, BlacklistManager, BlacklistReason};
pub use firewall::{FirewallConfig, FirewallEngine};
pub use metrics::FirewallMetrics;
