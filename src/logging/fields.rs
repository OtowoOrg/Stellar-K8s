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
//! Standardised logging field names for CI and runtime diagnostics (Issue #1115).
//!
//! All tracing call-sites **MUST** use the constants defined here instead of
//! bare string literals.  This prevents field-name drift between CI log
//! aggregation pipelines (which rely on consistent key names for alerting and
//! dashboards) and runtime structured logs.
//!
//! # Usage
//!
//! ```rust
//! use stellar_k8s::logging::fields as F;
//!
//! # let node_name = "node-1".to_string();
//! # let namespace = "default".to_string();
//! # let reconcile_id = 42u64;
//! tracing::info!(
//!     { F::NODE }      = %node_name,
//!     { F::NAMESPACE } = %namespace,
//!     { F::RECONCILE_ID } = reconcile_id,
//!     "Reconciliation started",
//! );
//! ```
//!
//! # Field inventory
//!
//! | Constant | Wire name | Description |
//! |----------|-----------|-------------|
//! | `NODE` | `node` | StellarNode resource name |
//! | `NAMESPACE` | `namespace` | Kubernetes namespace |
//! | `NODE_TYPE` | `node_type` | Validator / Horizon / SorobanRpc |
//! | `RECONCILE_ID` | `reconcile_id` | Monotonic reconcile counter |
//! | `ERROR` | `error` | Error description (Display) |
//! | `DURATION_MS` | `duration_ms` | Operation duration in milliseconds |
//! | `COMPONENT` | `component` | Sub-system emitting the log |
//! | `PHASE` | `phase` | Lifecycle phase (e.g. "init", "reconcile", "cleanup") |
//! | `VERSION` | `version` | Image / software version |
//! | `LEDGER` | `ledger` | Stellar ledger sequence number |
//! | `REGION` | `region` | Cloud / geographic region |
//! | `CLUSTER` | `cluster` | Kubernetes cluster name or ARN |
//! | `JOB_ID` | `job_id` | Background job identifier |
//! | `AUDIT_ACTION` | `audit_action` | Audit trail action string |
//! | `SCRUB_PATTERN` | `scrub_pattern` | Regex pattern name that triggered scrubbing |
//! | `TRACE_ID` | `trace_id` | W3C trace ID (injected by OtelTraceIdLayer) |
//! | `SPAN_ID` | `span_id` | W3C span ID (injected by OtelTraceIdLayer) |
//! | `K8S_NODE` | `k8s_node` | Kubernetes node (host) name |

// ── Kubernetes / operator identity ────────────────────────────────────────────

/// StellarNode resource name (`node`).
pub const NODE: &str = "node";

/// Kubernetes namespace (`namespace`).
pub const NAMESPACE: &str = "namespace";

/// Stellar node type: Validator, Horizon, or SorobanRpc (`node_type`).
pub const NODE_TYPE: &str = "node_type";

/// Kubernetes cluster name or ARN (`cluster`).
pub const CLUSTER: &str = "cluster";

/// Kubernetes node (host) name (`k8s_node`).
pub const K8S_NODE: &str = "k8s_node";

// ── Reconciliation ─────────────────────────────────────────────────────────────

/// Monotonically-increasing reconcile counter (`reconcile_id`).
pub const RECONCILE_ID: &str = "reconcile_id";

/// Lifecycle phase, e.g. `"init"`, `"reconcile"`, `"cleanup"` (`phase`).
pub const PHASE: &str = "phase";

// ── Errors and diagnostics ─────────────────────────────────────────────────────

/// Error description (`error`). Use `%err` (Display) not `?err` (Debug) at
/// call-sites so the value is human-readable in log aggregators.
pub const ERROR: &str = "error";

/// Operation duration in milliseconds (`duration_ms`).
pub const DURATION_MS: &str = "duration_ms";

/// Sub-system or module emitting the log event (`component`).
pub const COMPONENT: &str = "component";

// ── Stellar domain ─────────────────────────────────────────────────────────────

/// Stellar ledger sequence number (`ledger`).
pub const LEDGER: &str = "ledger";

/// Image or software version (`version`).
pub const VERSION: &str = "version";

/// Cloud or geographic region (`region`).
pub const REGION: &str = "region";

// ── Background jobs ────────────────────────────────────────────────────────────

/// Background job identifier (`job_id`).
pub const JOB_ID: &str = "job_id";

// ── Audit ──────────────────────────────────────────────────────────────────────

/// Audit trail action string, e.g. `"created-deployment"` (`audit_action`).
pub const AUDIT_ACTION: &str = "audit_action";

// ── Security / scrubbing ───────────────────────────────────────────────────────

/// Name of the redaction pattern that matched, e.g. `"stellar_seed"` (`scrub_pattern`).
pub const SCRUB_PATTERN: &str = "scrub_pattern";

// ── Distributed tracing ────────────────────────────────────────────────────────

/// W3C trace ID, injected by `OtelTraceIdLayer` (`trace_id`).
pub const TRACE_ID: &str = "trace_id";

/// W3C span ID, injected by `OtelTraceIdLayer` (`span_id`).
pub const SPAN_ID: &str = "span_id";

// ── Correlation ──────────────────────────────────────────────────────────────────

/// Request correlation ID propagated across service boundaries (`correlation_id`).
pub const CORRELATION_ID: &str = "correlation_id";

// ── CI-specific ────────────────────────────────────────────────────────────────

/// CI pipeline step or job name (`ci_step`).
pub const CI_STEP: &str = "ci_step";

/// Git commit SHA (`git_sha`).
pub const GIT_SHA: &str = "git_sha";

/// Cargo feature flags active during the build (`features`).
pub const FEATURES: &str = "features";

// ── Validation helpers ─────────────────────────────────────────────────────────

/// All field name constants, used by the field-name audit test.
pub const ALL_FIELDS: &[&str] = &[
    NODE,
    NAMESPACE,
    NODE_TYPE,
    CLUSTER,
    K8S_NODE,
    RECONCILE_ID,
    PHASE,
    ERROR,
    DURATION_MS,
    COMPONENT,
    LEDGER,
    VERSION,
    REGION,
    JOB_ID,
    AUDIT_ACTION,
    SCRUB_PATTERN,
    TRACE_ID,
    SPAN_ID,
    CORRELATION_ID,
    CI_STEP,
    GIT_SHA,
    FEATURES,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_field_names_are_non_empty() {
        for field in ALL_FIELDS {
            assert!(
                !field.is_empty(),
                "field name must not be empty: {:?}",
                field
            );
        }
    }

    #[test]
    fn all_field_names_are_lowercase_snake_case() {
        for field in ALL_FIELDS {
            assert!(
                field
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "field name '{}' must be lowercase_snake_case",
                field
            );
        }
    }

    #[test]
    fn all_field_names_are_unique() {
        let set: HashSet<&&str> = ALL_FIELDS.iter().collect();
        assert_eq!(
            set.len(),
            ALL_FIELDS.len(),
            "duplicate field name constants detected"
        );
    }

    #[test]
    fn known_fields_have_expected_wire_names() {
        assert_eq!(NODE, "node");
        assert_eq!(NAMESPACE, "namespace");
        assert_eq!(ERROR, "error");
        assert_eq!(RECONCILE_ID, "reconcile_id");
        assert_eq!(TRACE_ID, "trace_id");
        assert_eq!(SPAN_ID, "span_id");
        assert_eq!(DURATION_MS, "duration_ms");
    }
}
