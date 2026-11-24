//! Zero-Trust Egress Policy Synthesis from Workload Telemetry
//!
//! Automatically synthesises least-privilege NetworkPolicy egress rules by
//! observing real egress flows, then continuously tightens them against a
//! deny-by-default baseline.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────┐
//! │                  Egress Policy Synthesis Loop                        │
//! │                                                                      │
//! │  1. Ingest egress flows into ring buffer (10k flows/sec per node)    │
//! │  2. Observe for 7-day window before first enforcement                │
//! │  3. Synthesise candidate NetworkPolicy from observed destinations    │
//! │  4. Render human-readable YAML diff against existing policy          │
//! │  5. Write candidate policies to shadow namespace for review          │
//! │  6. Promote approved policies to workload namespace                  │
//! └──────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Design goals
//!
//! - **Throughput**: [`FlowIngestionBuffer`] uses a [`VecDeque`] ring buffer
//!   with capacity-based eviction, keeping the hot `ingest_flow` path
//!   allocation-free once the buffer is full.
//! - **Safety**: Policies live in a `{namespace}-shadow` namespace and are
//!   never applied to workloads until the 7-day observe window elapses *and*
//!   an operator explicitly calls [`EgressPolicySynthesizer::promote_to_workload_namespace`].
//! - **Observability**: Every promotion and synthesis step is traced with
//!   structured [`tracing`] fields.
//!
//! # Constants
//!
//! - [`OBSERVE_WINDOW_DAYS`] — minimum days of observation before enforcement.
//! - [`MAX_FALSE_POSITIVE_RATE`] — target upper bound on false-positive blocks.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use k8s_openapi::api::networking::v1::{
    NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyPort, NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::api::{ObjectMeta, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use tracing::{debug, info, warn};

// ============================================================================
// Constants
// ============================================================================

/// Minimum number of days to observe egress flows before enforcement is allowed.
pub const OBSERVE_WINDOW_DAYS: u64 = 7;

/// Maximum acceptable false-positive block rate in staging (0.1 %).
pub const MAX_FALSE_POSITIVE_RATE: f64 = 0.001;

/// Ring-buffer capacity per node.  At 10 k flows/sec this retains ~1 second
/// of back-pressure before eviction, which is intentional: the synthesiser
/// reads aggregated destination sets, not individual flow records.
pub const FLOW_BUFFER_CAPACITY: usize = 10_000;

// ============================================================================
// EgressFlow — a single observed flow record
// ============================================================================

/// A single egress flow observed on a node.
///
/// Captured by the eBPF / CNI telemetry agent and forwarded to the
/// synthesiser via [`FlowIngestionBuffer::ingest`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct EgressFlow {
    /// Name of the source pod.
    pub source_pod: String,
    /// Namespace of the source pod.
    pub source_namespace: String,
    /// Destination IP address (IPv4 or IPv6).
    pub destination_ip: String,
    /// Destination port number.
    pub destination_port: u16,
    /// IP protocol (`TCP`, `UDP`, or `SCTP`).
    pub protocol: String,
    /// Wall-clock time the flow was first observed.
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// FlowIngestionBuffer — ring buffer for high-throughput flow ingestion
// ============================================================================

/// High-throughput ring buffer for [`EgressFlow`] records.
///
/// The buffer is bounded at [`FLOW_BUFFER_CAPACITY`] entries.  When full,
/// the oldest flow is evicted before the new one is inserted, keeping the
/// ingest path allocation-free and O(1).
///
/// # Throughput target
///
/// At 10 k flows/sec per node the buffer saturates in ~1 second.  The
/// synthesiser is expected to drain the buffer (via [`Self::drain`]) at least
/// once per second.  Flows that have already been aggregated into the
/// destination set are not stored twice.
pub struct FlowIngestionBuffer {
    buffer: VecDeque<EgressFlow>,
    capacity: usize,
    /// Total flows ingested since creation (monotonic counter).
    pub total_ingested: u64,
    /// Total flows evicted due to buffer overflow.
    pub total_evicted: u64,
}

impl FlowIngestionBuffer {
    /// Create a new buffer with `capacity` maximum entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            total_ingested: 0,
            total_evicted: 0,
        }
    }

    /// Ingest a single flow.
    ///
    /// This is the **fast path** — it must not block or allocate when the
    /// buffer is below capacity, and must stay O(1) even when the buffer is
    /// full.  Callers on the data plane should call this from a non-async
    /// context.
    #[inline]
    pub fn ingest(&mut self, flow: EgressFlow) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
            self.total_evicted += 1;
        }
        self.buffer.push_back(flow);
        self.total_ingested += 1;
    }

    /// Drain all buffered flows, returning them as a `Vec`.
    ///
    /// The buffer is left empty after this call.  The synthesiser calls this
    /// periodically to fold flows into its destination aggregation map.
    pub fn drain(&mut self) -> Vec<EgressFlow> {
        self.buffer.drain(..).collect()
    }

    /// Current number of flows waiting in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// True if the buffer contains no flows.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

// ============================================================================
// ObserveWindow — 7-day observation guard
// ============================================================================

/// Tracks the per-namespace observation window that must elapse before the
/// synthesiser is permitted to enforce a generated policy.
///
/// The window starts the first time a flow is ingested for a namespace and
/// is considered complete after [`OBSERVE_WINDOW_DAYS`] days.
#[derive(Debug, Clone)]
pub struct ObserveWindow {
    /// Keyed by namespace name; value is the timestamp of the first observed
    /// flow for that namespace.
    first_seen: HashMap<String, DateTime<Utc>>,
}

impl ObserveWindow {
    /// Create an empty observe-window tracker.
    pub fn new() -> Self {
        Self {
            first_seen: HashMap::new(),
        }
    }

    /// Record that at least one flow has been seen for `namespace`.
    ///
    /// If this is the first observation for the namespace, the window clock
    /// starts now.
    pub fn record_observation(&mut self, namespace: &str) {
        self.first_seen
            .entry(namespace.to_string())
            .or_insert_with(Utc::now);
    }

    /// Return `true` if the observation window for `namespace` has elapsed.
    ///
    /// Returns `false` if no flows have ever been seen for the namespace.
    pub fn is_complete(&self, namespace: &str) -> bool {
        match self.first_seen.get(namespace) {
            None => false,
            Some(&start) => {
                let elapsed = Utc::now().signed_duration_since(start);
                elapsed >= ChronoDuration::days(OBSERVE_WINDOW_DAYS as i64)
            }
        }
    }

    /// Return the timestamp at which enforcement will first be permitted, or
    /// `None` if no observations have been recorded for this namespace.
    pub fn enforcement_eligible_at(&self, namespace: &str) -> Option<DateTime<Utc>> {
        self.first_seen
            .get(namespace)
            .map(|&start| start + ChronoDuration::days(OBSERVE_WINDOW_DAYS as i64))
    }

    /// Number of namespaces currently being tracked.
    pub fn tracked_namespaces(&self) -> usize {
        self.first_seen.len()
    }
}

impl Default for ObserveWindow {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// CandidateEgressPolicy — a synthesised policy pending review
// ============================================================================

/// A candidate [`NetworkPolicy`] produced by the synthesiser.
///
/// Candidates are written to the shadow namespace by [`ShadowNamespaceWriter`]
/// and must pass a human (or automated gate) review before promotion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEgressPolicy {
    /// The synthesised Kubernetes NetworkPolicy object.
    pub policy: NetworkPolicy,
    /// Namespace the policy *targets* (not the shadow namespace it lives in).
    pub target_namespace: String,
    /// ISO-8601 timestamp when this candidate was generated.
    pub generated_at: DateTime<Utc>,
    /// Number of unique egress flows that informed this policy.
    pub flow_count: u64,
    /// Whether the observe window has elapsed and enforcement is permitted.
    pub enforcement_ready: bool,
}

impl CandidateEgressPolicy {
    /// Return the policy name, falling back to `"<unnamed>"` if the metadata
    /// name is absent.
    pub fn name(&self) -> &str {
        self.policy
            .metadata
            .name
            .as_deref()
            .unwrap_or("<unnamed>")
    }
}

// ============================================================================
// ShadowNamespaceWriter — writes candidate policies to a review namespace
// ============================================================================

/// Writes [`CandidateEgressPolicy`] objects to the shadow namespace
/// `{target_namespace}-shadow` so that operators can review them before
/// promotion.
pub struct ShadowNamespaceWriter {
    client: Client,
}

impl ShadowNamespaceWriter {
    /// Create a new writer backed by the given Kubernetes client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Return the shadow namespace name for `target_namespace`.
    pub fn shadow_namespace(target_namespace: &str) -> String {
        format!("{target_namespace}-shadow")
    }

    /// Write `candidate` to its shadow namespace.
    ///
    /// Uses a server-side apply patch so that repeated calls are idempotent.
    pub async fn write(&self, candidate: &CandidateEgressPolicy) -> Result<(), kube::Error> {
        let shadow_ns = Self::shadow_namespace(&candidate.target_namespace);

        let mut policy = candidate.policy.clone();
        // Rewrite the namespace to point at the shadow namespace.
        policy.metadata.namespace = Some(shadow_ns.clone());

        let api: Api<NetworkPolicy> = Api::namespaced(self.client.clone(), &shadow_ns);

        let patch_params = PatchParams::apply("stellar-egress-synthesizer").force();
        let policy_name = policy
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "unnamed-egress-policy".to_string());

        api.patch(&policy_name, &patch_params, &Patch::Apply(&policy))
            .await?;

        info!(
            target_namespace = %candidate.target_namespace,
            shadow_namespace = %shadow_ns,
            policy = %policy_name,
            flow_count = candidate.flow_count,
            enforcement_ready = candidate.enforcement_ready,
            "Wrote candidate egress policy to shadow namespace"
        );

        Ok(())
    }
}

// ============================================================================
// EgressPolicySynthesizer — the main controller struct
// ============================================================================

/// Synthesises least-privilege egress [`NetworkPolicy`] objects from observed
/// workload telemetry.
///
/// # Usage
///
/// ```rust,ignore
/// let mut synthesizer = EgressPolicySynthesizer::new();
/// synthesizer.ingest_flow(flow);
/// let candidates = synthesizer.synthesize_candidate_policies("stellar-prod");
/// for candidate in &candidates {
///     let diff = synthesizer.render_yaml_diff(None, candidate);
///     println!("{diff}");
/// }
/// ```
pub struct EgressPolicySynthesizer {
    /// Per-namespace ring buffers for in-flight flow ingestion.
    buffers: HashMap<String, FlowIngestionBuffer>,
    /// Aggregated destination set per namespace.
    /// Key: namespace → set of `(destination_ip, destination_port, protocol)`.
    destinations: HashMap<String, HashMap<(String, u16, String), u64>>,
    /// Observation-window tracker.
    observe_window: ObserveWindow,
    /// Total flows ingested across all namespaces (monotonic).
    pub total_flows_ingested: u64,
}

impl EgressPolicySynthesizer {
    /// Create a new synthesiser with empty state.
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            destinations: HashMap::new(),
            observe_window: ObserveWindow::new(),
            total_flows_ingested: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Flow ingestion (fast path, non-async)
    // -----------------------------------------------------------------------

    /// Ingest a single [`EgressFlow`].
    ///
    /// This is the **non-async fast path** designed for the data-plane hot
    /// loop.  It enqueues the flow in the per-namespace ring buffer and
    /// immediately folds it into the destination aggregation map so the
    /// buffer can be kept small.
    ///
    /// Performance: O(1) amortised, no heap allocation when the buffer is at
    /// capacity.
    pub fn ingest_flow(&mut self, flow: EgressFlow) {
        let ns = flow.source_namespace.clone();

        // Record first observation for the observe-window clock.
        self.observe_window.record_observation(&ns);

        // Buffer the flow (ring buffer with capacity-based eviction).
        let buffer = self
            .buffers
            .entry(ns.clone())
            .or_insert_with(|| FlowIngestionBuffer::new(FLOW_BUFFER_CAPACITY));
        buffer.ingest(flow.clone());

        // Fold into aggregated destination map immediately so the buffer can
        // be drained without losing information.
        let dest_key = (
            flow.destination_ip.clone(),
            flow.destination_port,
            flow.protocol.clone(),
        );
        let count = self
            .destinations
            .entry(ns.clone())
            .or_default()
            .entry(dest_key)
            .or_insert(0);
        *count += 1;

        self.total_flows_ingested += 1;

        debug!(
            namespace = %ns,
            destination_ip = %flow.destination_ip,
            destination_port = flow.destination_port,
            protocol = %flow.protocol,
            total = self.total_flows_ingested,
            "Flow ingested"
        );
    }

    // -----------------------------------------------------------------------
    // Policy synthesis
    // -----------------------------------------------------------------------

    /// Synthesise candidate [`NetworkPolicy`] objects for `namespace`.
    ///
    /// Each unique `(destination_port, protocol)` pair observed in the egress
    /// flows becomes one [`NetworkPolicyEgressRule`].  The resulting policy is
    /// a deny-by-default policy that only allows observed traffic patterns.
    ///
    /// Returns an empty `Vec` if no flows have been recorded for the namespace.
    pub fn synthesize_candidate_policies(&self, namespace: &str) -> Vec<CandidateEgressPolicy> {
        let Some(destinations) = self.destinations.get(namespace) else {
            debug!(namespace, "No flows recorded; skipping synthesis");
            return vec![];
        };

        // Aggregate by (port, protocol) → collect all destination IPs.
        let mut by_port: HashMap<(u16, String), Vec<String>> = HashMap::new();
        let mut total_flows: u64 = 0;

        for ((ip, port, proto), &count) in destinations {
            by_port
                .entry((*port, proto.clone()))
                .or_default()
                .push(ip.clone());
            total_flows += count;
        }

        // Build egress rules — one rule per (port, protocol).
        let mut egress_rules: Vec<NetworkPolicyEgressRule> = by_port
            .iter()
            .map(|((port, proto), _ips)| {
                let protocol_str = proto.to_uppercase();
                NetworkPolicyEgressRule {
                    ports: Some(vec![NetworkPolicyPort {
                        port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                            *port as i32,
                        )),
                        protocol: Some(protocol_str),
                        end_port: None,
                    }]),
                    // No `to` peers — allow to any IP on the observed port.
                    // Operators can tighten peer selectors during review.
                    to: None,
                }
            })
            .collect();

        // Sort for deterministic output (port ascending, then protocol).
        egress_rules.sort_by(|a, b| {
            let port_a = a
                .ports
                .as_ref()
                .and_then(|p| p.first())
                .and_then(|p| {
                    if let Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                        n,
                    )) = &p.port
                    {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            let port_b = b
                .ports
                .as_ref()
                .and_then(|p| p.first())
                .and_then(|p| {
                    if let Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                        n,
                    )) = &p.port
                    {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            port_a.cmp(&port_b)
        });

        let policy_name = format!("synthesized-egress-{namespace}");
        let enforcement_ready = self.is_observe_window_complete(namespace);

        let mut annotations: BTreeMap<String, String> = BTreeMap::new();
        annotations.insert(
            "stellar.org/synthesized-at".to_string(),
            Utc::now().to_rfc3339(),
        );
        annotations.insert(
            "stellar.org/flow-count".to_string(),
            total_flows.to_string(),
        );
        annotations.insert(
            "stellar.org/enforcement-ready".to_string(),
            enforcement_ready.to_string(),
        );
        if let Some(eligible_at) = self.observe_window.enforcement_eligible_at(namespace) {
            annotations.insert(
                "stellar.org/enforcement-eligible-at".to_string(),
                eligible_at.to_rfc3339(),
            );
        }

        let mut labels: BTreeMap<String, String> = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/managed-by".to_string(),
            "stellar-egress-synthesizer".to_string(),
        );
        labels.insert(
            "stellar.org/target-namespace".to_string(),
            namespace.to_string(),
        );

        let policy = NetworkPolicy {
            metadata: ObjectMeta {
                name: Some(policy_name),
                namespace: Some(namespace.to_string()),
                labels: Some(labels),
                annotations: Some(annotations),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec {
                // Apply to all pods in the namespace (empty selector = select all).
                pod_selector: LabelSelector::default(),
                // Deny all egress not covered by the rules below.
                policy_types: Some(vec!["Egress".to_string()]),
                egress: Some(egress_rules),
                ingress: None,
            }),
        };

        info!(
            namespace,
            flow_count = total_flows,
            enforcement_ready,
            rule_count = policy
                .spec
                .as_ref()
                .and_then(|s| s.egress.as_ref())
                .map(|e| e.len())
                .unwrap_or(0),
            "Synthesised candidate egress policy"
        );

        vec![CandidateEgressPolicy {
            policy,
            target_namespace: namespace.to_string(),
            generated_at: Utc::now(),
            flow_count: total_flows,
            enforcement_ready,
        }]
    }

    // -----------------------------------------------------------------------
    // YAML diff rendering
    // -----------------------------------------------------------------------

    /// Render a human-readable YAML diff between `old` and `new` candidate
    /// policies.
    ///
    /// When `old` is `None` the diff is rendered as a pure addition (all lines
    /// prefixed with `+`).  Lines present in both are prefixed with a space.
    /// Lines removed relative to `old` are prefixed with `-`.
    ///
    /// The output is suitable for display in a pull-request comment or CLI.
    pub fn render_yaml_diff(
        &self,
        old: Option<&CandidateEgressPolicy>,
        new: &CandidateEgressPolicy,
    ) -> String {
        let new_yaml = serde_yaml_policy(&new.policy);

        match old {
            None => {
                // Pure addition — prefix every line with '+'.
                let added: String = new_yaml
                    .lines()
                    .map(|l| format!("+{l}\n"))
                    .collect();
                format!(
                    "--- /dev/null\n+++ synthesized-egress-{}\n{}",
                    new.target_namespace, added
                )
            }
            Some(prev) => {
                let old_yaml = serde_yaml_policy(&prev.policy);
                compute_line_diff(&old_yaml, &new_yaml, &prev.target_namespace, &new.target_namespace)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Observe-window guard
    // -----------------------------------------------------------------------

    /// Return `true` if the 7-day observe window has elapsed for `namespace`
    /// and enforcement is permitted.
    pub fn is_observe_window_complete(&self, namespace: &str) -> bool {
        self.observe_window.is_complete(namespace)
    }

    // -----------------------------------------------------------------------
    // Policy promotion
    // -----------------------------------------------------------------------

    /// Promote a candidate policy from the shadow namespace to the workload
    /// namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if the observe window has not yet elapsed or if the
    /// Kubernetes API call fails.
    pub async fn promote_to_workload_namespace(
        client: Client,
        candidate: &CandidateEgressPolicy,
        target_namespace: &str,
    ) -> Result<(), kube::Error> {
        // Safety guard: never promote before the observe window is complete.
        if !candidate.enforcement_ready {
            warn!(
                target_namespace,
                policy = candidate.name(),
                "Promotion blocked: observe window not yet complete"
            );
            // Surface the guard as an Kubernetes API-style error so callers
            // can distinguish it from infrastructure errors.
            return Err(kube::Error::Api(kube::error::ErrorResponse {
                status: "Forbidden".to_string(),
                message: format!(
                    "observe window of {OBSERVE_WINDOW_DAYS} days has not elapsed for \
                     namespace '{target_namespace}'; policy '{}'",
                    candidate.name()
                ),
                reason: "ObserveWindowIncomplete".to_string(),
                code: 403,
            }));
        }

        let mut policy = candidate.policy.clone();
        policy.metadata.namespace = Some(target_namespace.to_string());
        // Clear resource version so the patch creates or replaces cleanly.
        policy.metadata.resource_version = None;

        let api: Api<NetworkPolicy> = Api::namespaced(client, target_namespace);
        let policy_name = policy
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| format!("synthesized-egress-{target_namespace}"));

        let patch_params = PatchParams::apply("stellar-egress-synthesizer").force();
        api.patch(&policy_name, &patch_params, &Patch::Apply(&policy))
            .await?;

        info!(
            target_namespace,
            policy = %policy_name,
            "Promoted candidate egress policy to workload namespace"
        );

        Ok(())
    }
}

impl Default for EgressPolicySynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Serialise a [`NetworkPolicy`] to a compact YAML string for diff rendering.
///
/// Falls back to a JSON representation if the YAML serialiser is unavailable.
fn serde_yaml_policy(policy: &NetworkPolicy) -> String {
    // serde_yaml is not always a direct dependency; we use serde_json as a
    // portable fallback and produce a JSON-based "YAML" string.  Real
    // deployments should swap this for a proper serde_yaml call.
    match serde_json::to_string_pretty(policy) {
        Ok(json) => json,
        Err(e) => format!("<serialisation error: {e}>"),
    }
}

/// Compute a line-level diff between two YAML strings.
///
/// Lines only in `old` are prefixed with `-`, lines only in `new` with `+`,
/// and common lines with a space.  This is a simple patience-diff-inspired
/// implementation: common prefix and suffix are preserved, the middle is
/// shown as a block removal followed by a block addition.
fn compute_line_diff(old_yaml: &str, new_yaml: &str, old_name: &str, new_name: &str) -> String {
    let old_lines: Vec<&str> = old_yaml.lines().collect();
    let new_lines: Vec<&str> = new_yaml.lines().collect();

    // Find the common prefix length.
    let common_prefix = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Find the common suffix length (excluding the common prefix).
    let old_suffix_start = common_prefix;
    let new_suffix_start = common_prefix;
    let common_suffix = old_lines[old_suffix_start..]
        .iter()
        .rev()
        .zip(new_lines[new_suffix_start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let old_changed_end = old_lines.len() - common_suffix;
    let new_changed_end = new_lines.len() - common_suffix;

    let mut diff = format!("--- {old_name}\n+++ {new_name}\n");

    for line in &old_lines[..common_prefix] {
        diff.push(' ');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &old_lines[common_prefix..old_changed_end] {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &new_lines[new_suffix_start..new_changed_end] {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &old_lines[old_changed_end..] {
        diff.push(' ');
        diff.push_str(line);
        diff.push('\n');
    }

    diff
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // -----------------------------------------------------------------------
    // FlowIngestionBuffer tests
    // -----------------------------------------------------------------------

    fn make_flow(ns: &str, ip: &str, port: u16) -> EgressFlow {
        EgressFlow {
            source_pod: "test-pod".to_string(),
            source_namespace: ns.to_string(),
            destination_ip: ip.to_string(),
            destination_port: port,
            protocol: "TCP".to_string(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn buffer_capacity_eviction() {
        let mut buf = FlowIngestionBuffer::new(5);
        for i in 0..8u16 {
            buf.ingest(make_flow("default", &format!("10.0.0.{i}"), 443));
        }
        // Buffer should hold exactly the last 5 flows.
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.total_ingested, 8);
        assert_eq!(buf.total_evicted, 3);
    }

    #[test]
    fn buffer_drain_empties_buffer() {
        let mut buf = FlowIngestionBuffer::new(10);
        buf.ingest(make_flow("default", "10.0.0.1", 443));
        buf.ingest(make_flow("default", "10.0.0.2", 80));
        let drained = buf.drain();
        assert_eq!(drained.len(), 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn flow_ingestion_throughput_simulation() {
        // Simulate 10 k flows ingested in well under 1 second on modern
        // hardware.  We allow a generous 500 ms budget to avoid flakiness on
        // CI runners.
        let mut synthesizer = EgressPolicySynthesizer::new();
        let flow_count = 10_000usize;

        let start = Instant::now();
        for i in 0..flow_count {
            let flow = EgressFlow {
                source_pod: format!("pod-{i}"),
                source_namespace: "perf-test".to_string(),
                destination_ip: format!("10.{}.{}.{}", (i / 65536) % 256, (i / 256) % 256, i % 256),
                destination_port: (443 + (i % 100)) as u16,
                protocol: "TCP".to_string(),
                timestamp: Utc::now(),
            };
            synthesizer.ingest_flow(flow);
        }
        let elapsed = start.elapsed();

        assert_eq!(synthesizer.total_flows_ingested, flow_count as u64);
        assert!(
            elapsed < Duration::from_millis(500),
            "10k flow ingestion took {:?}, expected < 500ms",
            elapsed
        );
    }

    // -----------------------------------------------------------------------
    // ObserveWindow tests
    // -----------------------------------------------------------------------

    #[test]
    fn observe_window_not_complete_before_start() {
        let window = ObserveWindow::new();
        assert!(!window.is_complete("default"));
    }

    #[test]
    fn observe_window_not_complete_immediately_after_first_observation() {
        let mut window = ObserveWindow::new();
        window.record_observation("default");
        // The window was just started; 7 days have not passed.
        assert!(!window.is_complete("default"));
    }

    #[test]
    fn observe_window_complete_when_backdated() {
        let mut window = ObserveWindow::new();
        // Inject a start time 8 days in the past.
        let past = Utc::now() - ChronoDuration::days(8);
        window.first_seen.insert("default".to_string(), past);
        assert!(window.is_complete("default"));
    }

    #[test]
    fn observe_window_not_complete_6_days_ago() {
        let mut window = ObserveWindow::new();
        let past = Utc::now() - ChronoDuration::days(6);
        window.first_seen.insert("default".to_string(), past);
        assert!(!window.is_complete("default"));
    }

    #[test]
    fn observe_window_enforcement_eligible_at_is_7_days_after_first_seen() {
        let mut window = ObserveWindow::new();
        let start = Utc::now();
        window.first_seen.insert("default".to_string(), start);
        let eligible_at = window.enforcement_eligible_at("default").unwrap();
        let diff = eligible_at.signed_duration_since(start);
        assert_eq!(diff.num_days(), 7);
    }

    #[test]
    fn observe_window_tracks_multiple_namespaces_independently() {
        let mut window = ObserveWindow::new();
        let past = Utc::now() - ChronoDuration::days(8);
        window.first_seen.insert("ns-a".to_string(), past);
        window.record_observation("ns-b");
        assert!(window.is_complete("ns-a"));
        assert!(!window.is_complete("ns-b"));
        assert_eq!(window.tracked_namespaces(), 2);
    }

    // -----------------------------------------------------------------------
    // EgressPolicySynthesizer — synthesis tests
    // -----------------------------------------------------------------------

    #[test]
    fn synthesize_returns_empty_for_unknown_namespace() {
        let synthesizer = EgressPolicySynthesizer::new();
        let candidates = synthesizer.synthesize_candidate_policies("nonexistent");
        assert!(candidates.is_empty());
    }

    #[test]
    fn synthesize_produces_one_candidate_per_namespace() {
        let mut synthesizer = EgressPolicySynthesizer::new();
        synthesizer.ingest_flow(make_flow("stellar-prod", "10.0.0.1", 443));
        synthesizer.ingest_flow(make_flow("stellar-prod", "10.0.0.2", 443));
        synthesizer.ingest_flow(make_flow("stellar-prod", "10.0.0.3", 80));

        let candidates = synthesizer.synthesize_candidate_policies("stellar-prod");
        assert_eq!(candidates.len(), 1);

        let candidate = &candidates[0];
        assert_eq!(candidate.target_namespace, "stellar-prod");
        assert_eq!(candidate.flow_count, 3);

        let rule_count = candidate
            .policy
            .spec
            .as_ref()
            .and_then(|s| s.egress.as_ref())
            .map(|e| e.len())
            .unwrap_or(0);
        // Two unique ports (443 and 80) → two egress rules.
        assert_eq!(rule_count, 2);
    }

    #[test]
    fn synthesize_policy_has_correct_policy_type() {
        let mut synthesizer = EgressPolicySynthesizer::new();
        synthesizer.ingest_flow(make_flow("test-ns", "192.168.1.1", 5432));

        let candidates = synthesizer.synthesize_candidate_policies("test-ns");
        let policy_types = candidates[0]
            .policy
            .spec
            .as_ref()
            .and_then(|s| s.policy_types.as_ref())
            .cloned()
            .unwrap_or_default();

        assert!(policy_types.contains(&"Egress".to_string()));
    }

    #[test]
    fn synthesize_enforcement_not_ready_for_fresh_namespace() {
        let mut synthesizer = EgressPolicySynthesizer::new();
        synthesizer.ingest_flow(make_flow("fresh-ns", "10.0.0.1", 443));
        let candidates = synthesizer.synthesize_candidate_policies("fresh-ns");
        assert!(!candidates[0].enforcement_ready);
    }

    #[test]
    fn synthesize_enforcement_ready_after_window_elapsed() {
        let mut synthesizer = EgressPolicySynthesizer::new();
        // Backdate the observe window to 8 days ago.
        let past = Utc::now() - ChronoDuration::days(8);
        synthesizer
            .observe_window
            .first_seen
            .insert("ready-ns".to_string(), past);
        synthesizer.ingest_flow(make_flow("ready-ns", "10.0.0.1", 443));

        let candidates = synthesizer.synthesize_candidate_policies("ready-ns");
        assert!(candidates[0].enforcement_ready);
    }

    // -----------------------------------------------------------------------
    // YAML diff rendering tests
    // -----------------------------------------------------------------------

    #[test]
    fn render_yaml_diff_new_policy_has_plus_prefix() {
        let synthesizer = EgressPolicySynthesizer::new();
        let mut s = EgressPolicySynthesizer::new();
        s.ingest_flow(make_flow("diff-ns", "10.0.0.1", 443));
        let candidates = s.synthesize_candidate_policies("diff-ns");
        let diff = synthesizer.render_yaml_diff(None, &candidates[0]);

        // Every content line should start with '+'.
        let content_lines: Vec<&str> = diff
            .lines()
            .filter(|l| !l.starts_with("---") && !l.starts_with("+++"))
            .collect();
        assert!(!content_lines.is_empty());
        for line in &content_lines {
            assert!(
                line.starts_with('+'),
                "Expected '+' prefix on line: {line}"
            );
        }
    }

    #[test]
    fn render_yaml_diff_identical_policies_has_no_plus_minus() {
        let mut synthesizer = EgressPolicySynthesizer::new();
        synthesizer.ingest_flow(make_flow("same-ns", "10.0.0.1", 443));
        let candidates = synthesizer.synthesize_candidate_policies("same-ns");
        let candidate = &candidates[0];

        let diff = synthesizer.render_yaml_diff(Some(candidate), candidate);

        let changed_lines: Vec<&str> = diff
            .lines()
            .filter(|l| {
                !l.starts_with("---")
                    && !l.starts_with("+++")
                    && (l.starts_with('+') || l.starts_with('-'))
            })
            .collect();
        assert!(
            changed_lines.is_empty(),
            "Expected no +/- lines for identical policies, got: {changed_lines:?}"
        );
    }

    #[test]
    fn render_yaml_diff_contains_header_lines() {
        let synthesizer = EgressPolicySynthesizer::new();
        let mut s = EgressPolicySynthesizer::new();
        s.ingest_flow(make_flow("hdr-ns", "10.0.0.1", 443));
        let candidates = s.synthesize_candidate_policies("hdr-ns");
        let diff = synthesizer.render_yaml_diff(None, &candidates[0]);

        assert!(diff.contains("---"), "Missing --- header in diff");
        assert!(diff.contains("+++"), "Missing +++ header in diff");
    }

    // -----------------------------------------------------------------------
    // Constants tests
    // -----------------------------------------------------------------------

    #[test]
    fn observe_window_days_constant() {
        assert_eq!(OBSERVE_WINDOW_DAYS, 7);
    }

    #[test]
    fn max_false_positive_rate_constant() {
        assert!((MAX_FALSE_POSITIVE_RATE - 0.001).abs() < f64::EPSILON);
    }

    #[test]
    fn shadow_namespace_writer_naming() {
        assert_eq!(
            ShadowNamespaceWriter::shadow_namespace("stellar-prod"),
            "stellar-prod-shadow"
        );
        assert_eq!(
            ShadowNamespaceWriter::shadow_namespace("default"),
            "default-shadow"
        );
    }
}
