//! Byzantine Monitoring — Multi-Vantage-Point Consensus Observer
//!
//! # Problem
//!
//! A Stellar node might believe it is in consensus while actually being network-partitioned.
//! Monitoring from a single vantage point (the cluster itself) cannot distinguish between
//! "the network is fine" and "we are isolated from the network".
//!
//! # Solution
//!
//! Deploy lightweight `stellar-watcher` sidecars in multiple geographically dispersed
//! cloud regions. Each watcher independently polls the Stellar Core HTTP API and reports
//! the latest externalized ledger hash. A central Prometheus instance aggregates all
//! watcher observations and fires an alert when more than 20% of watchers disagree on
//! the current ledger hash — a strong signal of a Byzantine partition.
//!
//! # Architecture
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────────────┐
//!  │                    Stellar Network                              │
//!  │                                                                 │
//!  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
//!  │  │  Validator A │  │  Validator B │  │  Validator C │         │
//!  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘         │
//!  └─────────┼─────────────────┼─────────────────┼─────────────────┘
//!            │                 │                 │
//!  ┌─────────▼─────────────────▼─────────────────▼─────────────────┐
//!  │                  Watcher Sidecars (per region)                 │
//!  │                                                                 │
//!  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
//!  │  │ Watcher      │  │ Watcher      │  │ Watcher      │         │
//!  │  │ us-east-1    │  │ eu-west-1    │  │ ap-south-1   │         │
//!  │  │ /metrics     │  │ /metrics     │  │ /metrics     │         │
//!  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘         │
//!  └─────────┼─────────────────┼─────────────────┼─────────────────┘
//!            │                 │                 │
//!  ┌─────────▼─────────────────▼─────────────────▼─────────────────┐
//!  │              Central Prometheus + AlertManager                  │
//!  │                                                                 │
//!  │  stellar_watcher_ledger_hash_divergence_ratio > 0.20           │
//!  │  → PagerDuty / Slack alert: "Byzantine partition detected"     │
//!  └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Metrics Exported by Each Watcher
//!
//! - `stellar_watcher_ledger_sequence` (gauge): Latest externalized ledger sequence seen
//!   from this vantage point.
//! - `stellar_watcher_ledger_hash` (gauge, label: `hash`): Always 1; the `hash` label
//!   carries the hex-encoded ledger close hash. Prometheus label cardinality is bounded
//!   because only the *latest* hash is kept.
//! - `stellar_watcher_consensus_view` (gauge): 1 if the watcher sees the node as
//!   externalized/synced, 0 otherwise.
//! - `stellar_watcher_poll_errors_total` (counter): Number of failed polls.
//! - `stellar_watcher_last_poll_timestamp_seconds` (gauge): Unix timestamp of the last
//!   successful poll.
//! - `stellar_watcher_region` (gauge, label: `region`, `cloud`, `node_endpoint`): Always 1;
//!   carries watcher identity metadata.
//!
//! # Aggregation Rule (PrometheusRule)
//!
//! The central Prometheus evaluates:
//!
//! ```promql
//! # Fraction of watchers that see a different hash than the majority
//! (
//!   count by (network) (
//!     stellar_watcher_ledger_hash != on(network) group_left()
//!     (topk by (network) (1, count by (network, hash) (stellar_watcher_ledger_hash)))
//!   )
//! )
//! /
//! count by (network) (stellar_watcher_ledger_hash)
//! > 0.20
//! ```
//!
//! See `monitoring/byzantine-alerts.yaml` for the full PrometheusRule manifest.

pub mod aggregator;
pub mod types;
pub mod watcher;
