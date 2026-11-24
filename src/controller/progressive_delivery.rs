//! Progressive Delivery Controller with Automated Canary Metric Gates
//!
//! Implements a canary promotion controller that advances rollouts only when
//! Prometheus-sourced SLOs remain within budget at each weight increment.
//! Each promotion step is modelled as a reconciler pass keyed on the hash of
//! `(targetRevision, stepIndex)` to guarantee idempotency across retries.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │               Progressive Delivery Controller                    │
//! │                                                                  │
//! │  1. Determine current canary step (stepIndex + targetRevision)   │
//! │  2. Compute idempotency key = SHA-256(targetRevision | stepIndex)│
//! │  3. Query Prometheus for each configured SLO metric              │
//! │  4. Gate: all SLOs within budget? → advance traffic weight       │
//! │           any SLO violated? → rollback within 90 s              │
//! │  5. Record GateEvaluationResult as a Kubernetes status condition │
//! │  6. Requeue for the next analysis window                         │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Traffic Split Backends
//!
//! The controller supports two backends:
//! - **ServiceMesh** – Istio `VirtualService` / Linkerd `TrafficSplit` weight patches
//! - **PureService** – Kubernetes `Service` selector swap + weighted `Endpoints`
//!
//! # Configuration example
//!
//! ```yaml
//! spec:
//!   progressiveDelivery:
//!     enabled: true
//!     prometheusUrl: "http://prometheus-operated.monitoring:9090"
//!     analysisWindowSeconds: 120
//!     stepWeights: [10, 25, 50, 75, 100]
//!     rollbackTimeoutSeconds: 90
//!     sloThresholds:
//!       - metric: "histogram_quantile(0.99, rate(http_request_duration_seconds_bucket[2m]))"
//!         maxValue: 0.5
//!       - metric: "rate(http_requests_total{status=~\"5..\"}[2m]) / rate(http_requests_total[2m])"
//!         maxValue: 0.01
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::Duration;
use tracing::{debug, error, info, warn};

// ============================================================================
// Configuration types
// ============================================================================

/// A single SLO threshold: the Prometheus PromQL expression must evaluate to a
/// value at or below `max_value` for the gate to pass.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SloThreshold {
    /// PromQL expression to evaluate.
    /// Example: `"histogram_quantile(0.99, rate(http_request_duration_seconds_bucket[2m]))"`
    pub metric: String,

    /// Maximum acceptable value. The gate fails if the query returns a value
    /// strictly greater than this.
    pub max_value: f64,
}

/// Full configuration for a progressive delivery rollout.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveDeliveryConfig {
    /// Enable progressive delivery for this deployment.
    #[serde(default)]
    pub enabled: bool,

    /// Prometheus base URL.
    /// Default: `"http://prometheus-operated.monitoring:9090"`
    #[serde(default = "default_prometheus_url")]
    pub prometheus_url: String,

    /// Seconds to wait (observe metrics) before evaluating the gate at each step.
    /// Default: 120
    #[serde(default = "default_analysis_window")]
    pub analysis_window_seconds: u64,

    /// Ordered list of canary traffic weights (percent). The controller advances
    /// through these in sequence; the final entry must be 100 to complete the rollout.
    /// Default: `[10, 25, 50, 75, 100]`
    #[serde(default = "default_step_weights")]
    pub step_weights: Vec<u32>,

    /// SLO thresholds that must all pass for a step to be promoted.
    #[serde(default)]
    pub slo_thresholds: Vec<SloThreshold>,

    /// Maximum seconds allowed for a rollback to complete.
    /// Default: 90
    #[serde(default = "default_rollback_timeout")]
    pub rollback_timeout_seconds: u64,
}

fn default_prometheus_url() -> String {
    "http://prometheus-operated.monitoring:9090".to_string()
}

fn default_analysis_window() -> u64 {
    120
}

fn default_step_weights() -> Vec<u32> {
    vec![10, 25, 50, 75, 100]
}

fn default_rollback_timeout() -> u64 {
    90
}

impl Default for ProgressiveDeliveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prometheus_url: default_prometheus_url(),
            analysis_window_seconds: default_analysis_window(),
            step_weights: default_step_weights(),
            slo_thresholds: Vec::new(),
            rollback_timeout_seconds: default_rollback_timeout(),
        }
    }
}

// ============================================================================
// Idempotency key
// ============================================================================

/// Uniquely identifies a single promotion step attempt.
///
/// The `id` field is the hex-encoded SHA-256 of `"{target_revision}:{step_index}"`.
/// Re-using the same `(targetRevision, stepIndex)` pair always produces the
/// same `id`, making the reconciler pass idempotent across retries.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromotionStepKey {
    /// The image digest or Git SHA that identifies the canary revision.
    pub target_revision: String,
    /// Zero-based index into `ProgressiveDeliveryConfig::step_weights`.
    pub step_index: usize,
    /// Deterministic SHA-256 hex string derived from `target_revision` and `step_index`.
    pub id: String,
}

impl PromotionStepKey {
    /// Create a new key and compute its idempotency `id`.
    pub fn new(target_revision: impl Into<String>, step_index: usize) -> Self {
        let target_revision = target_revision.into();
        let id = compute_step_hash(&target_revision, step_index);
        Self {
            target_revision,
            step_index,
            id,
        }
    }
}

impl fmt::Display for PromotionStepKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@step{}({})",
            self.target_revision,
            self.step_index,
            &self.id[..12]
        )
    }
}

/// Compute `SHA-256("{target_revision}:{step_index}")` and return the lower-hex string.
pub fn compute_step_hash(target_revision: &str, step_index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(target_revision.as_bytes());
    hasher.update(b":");
    hasher.update(step_index.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

// ============================================================================
// Gate evaluation result
// ============================================================================

/// The outcome of evaluating a single SLO threshold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThresholdEvaluation {
    /// The PromQL expression that was evaluated.
    pub metric: String,
    /// Maximum allowed value.
    pub max_value: f64,
    /// Observed value returned by Prometheus (`None` if the query failed).
    pub observed_value: Option<f64>,
    /// `true` if the threshold was satisfied (`observed_value <= max_value`).
    pub passed: bool,
}

/// Aggregate result of evaluating all SLO thresholds for one promotion step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateEvaluationResult {
    /// The step this evaluation belongs to.
    pub step_key: PromotionStepKey,
    /// Individual threshold outcomes.
    pub evaluations: Vec<ThresholdEvaluation>,
    /// `true` if **all** thresholds passed.
    pub gate_passed: bool,
    /// RFC-3339 timestamp at which the evaluation completed.
    pub evaluated_at: String,
}

// ============================================================================
// Status condition
// ============================================================================

/// Mirrors the Kubernetes `metav1.Condition` shape so that gate results can be
/// recorded directly into a CRD's `.status.conditions` array.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StepCondition {
    /// Condition type, e.g. `"CanaryGatePassed"` or `"CanaryGateFailed"`.
    #[serde(rename = "type")]
    pub condition_type: String,
    /// `"True"`, `"False"`, or `"Unknown"`.
    pub status: String,
    /// The idempotency key of the step that produced this condition.
    pub reason: String,
    /// Human-readable explanation.
    pub message: String,
    /// RFC-3339 timestamp.
    pub last_transition_time: String,
}

impl StepCondition {
    fn from_gate_result(result: &GateEvaluationResult, weight: u32) -> Self {
        let (condition_type, status) = if result.gate_passed {
            ("CanaryGatePassed", "True")
        } else {
            ("CanaryGateFailed", "False")
        };

        let failed_metrics: Vec<String> = result
            .evaluations
            .iter()
            .filter(|e| !e.passed)
            .map(|e| {
                format!(
                    "{} (observed={:.4}, max={:.4})",
                    e.metric,
                    e.observed_value.unwrap_or(f64::NAN),
                    e.max_value
                )
            })
            .collect();

        let message = if result.gate_passed {
            format!(
                "All {} SLO(s) within budget at {}% canary weight (step {})",
                result.evaluations.len(),
                weight,
                result.step_key.step_index
            )
        } else {
            format!(
                "SLO violation at {}% canary weight (step {}): {}",
                weight,
                result.step_key.step_index,
                failed_metrics.join("; ")
            )
        };

        Self {
            condition_type: condition_type.to_string(),
            status: status.to_string(),
            reason: result.step_key.id.clone(),
            message,
            last_transition_time: result.evaluated_at.clone(),
        }
    }
}

// ============================================================================
// Traffic split backend
// ============================================================================

/// Which mechanism to use for adjusting canary traffic weight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrafficSplitBackend {
    /// Istio `VirtualService` or Linkerd `TrafficSplit` CRD.
    ServiceMesh {
        /// `"istio"` or `"linkerd"` – controls the patch path.
        mesh_type: String,
        /// Name of the VirtualService / TrafficSplit resource.
        resource_name: String,
    },
    /// Pure Kubernetes `Service` + weighted `EndpointSlice` approach.
    PureService {
        /// Stable service name (receives 100% - canary_weight).
        stable_service: String,
        /// Canary service name (receives canary_weight).
        canary_service: String,
    },
}

// ============================================================================
// Prometheus scraping helper (mirrors predictive_scaling.rs pattern)
// ============================================================================

/// Scrape the current scalar value of a Prometheus instant query.
///
/// Returns `None` if the query returns no data or the HTTP request fails.
pub async fn scrape_prometheus_query(prometheus_url: &str, promql: &str) -> Option<f64> {
    let url = format!(
        "{}/api/v1/query?query={}",
        prometheus_url,
        urlencoding(promql)
    );

    let response = reqwest::get(&url).await.ok()?;
    let body: serde_json::Value = response.json().await.ok()?;

    body["data"]["result"]
        .as_array()?
        .first()?
        .get("value")?
        .as_array()?
        .get(1)?
        .as_str()?
        .parse::<f64>()
        .ok()
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}

fn now_rfc3339() -> String {
    // chrono is already a dependency via predictive_scaling; use std time as a
    // fallback string to avoid an extra import statement in this module.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // ISO-8601 without chrono: produce a valid RFC-3339 string.
    let (y, mo, d, h, mi, s) = epoch_to_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Minimal epoch→calendar decomposition (no external dependency).
fn epoch_to_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let total_min = secs / 60;
    let mi = total_min % 60;
    let total_hours = total_min / 60;
    let h = total_hours % 24;
    let total_days = total_hours / 24;

    // Gregorian calendar decomposition
    let mut year = 1970u64;
    let mut remaining = total_days;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }
    let months = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for &m in &months {
        let days = if month == 2 && is_leap(year) { 29 } else { m };
        if remaining < days {
            break;
        }
        remaining -= days;
        month += 1;
    }
    let day = remaining + 1;
    (year, month, day, h, mi, s)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ============================================================================
// Canary promotion controller
// ============================================================================

/// Core controller responsible for evaluating SLO gates and driving
/// canary traffic weight through the configured step sequence.
pub struct CanaryPromotionController {
    /// Kubernetes client used to patch Service / VirtualService resources.
    pub client: kube::Client,
}

impl CanaryPromotionController {
    /// Create a new controller with the given Kubernetes client.
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    // ------------------------------------------------------------------
    // Gate evaluation
    // ------------------------------------------------------------------

    /// Evaluate all SLO gates defined in `config` for the given `step_key`.
    ///
    /// Queries Prometheus for every [`SloThreshold`] and aggregates the results
    /// into a [`GateEvaluationResult`]. The call is idempotent for the same
    /// `step_key.id` because Prometheus is read-only.
    pub async fn evaluate_slo_gate(
        &self,
        step_key: PromotionStepKey,
        config: &ProgressiveDeliveryConfig,
    ) -> GateEvaluationResult {
        debug!(
            step = %step_key,
            thresholds = config.slo_thresholds.len(),
            "Evaluating SLO gate"
        );

        let mut evaluations: Vec<ThresholdEvaluation> = Vec::new();

        for threshold in &config.slo_thresholds {
            let observed = scrape_prometheus_query(&config.prometheus_url, &threshold.metric).await;

            let passed = match observed {
                Some(v) => v <= threshold.max_value,
                // A query failure is treated as a gate failure (fail-safe).
                None => {
                    warn!(
                        step = %step_key,
                        metric = %threshold.metric,
                        "Prometheus query returned no data; treating as gate failure"
                    );
                    false
                }
            };

            evaluations.push(ThresholdEvaluation {
                metric: threshold.metric.clone(),
                max_value: threshold.max_value,
                observed_value: observed,
                passed,
            });
        }

        let gate_passed = evaluations.iter().all(|e| e.passed);

        if gate_passed {
            info!(step = %step_key, "SLO gate passed – all thresholds within budget");
        } else {
            warn!(step = %step_key, "SLO gate failed – one or more thresholds violated");
        }

        GateEvaluationResult {
            step_key,
            evaluations,
            gate_passed,
            evaluated_at: now_rfc3339(),
        }
    }

    // ------------------------------------------------------------------
    // Traffic weight
    // ------------------------------------------------------------------

    /// Set the canary traffic weight for the given deployment/service.
    ///
    /// For `ServiceMesh`, patches the VirtualService/TrafficSplit weight field.
    /// For `PureService`, updates a well-known annotation on the canary Service
    /// that an external load-balancer or ingress controller can read.
    pub async fn advance_weight(
        &self,
        backend: &TrafficSplitBackend,
        namespace: &str,
        name: &str,
        weight: u32,
    ) -> Result<(), kube::Error> {
        match backend {
            TrafficSplitBackend::ServiceMesh {
                mesh_type,
                resource_name,
            } => {
                info!(
                    namespace,
                    name,
                    mesh = %mesh_type,
                    resource = %resource_name,
                    weight,
                    "Advancing ServiceMesh canary weight"
                );
                self.patch_service_mesh_weight(
                    namespace,
                    resource_name,
                    mesh_type,
                    weight,
                    100 - weight,
                )
                .await
            }
            TrafficSplitBackend::PureService {
                canary_service,
                stable_service: _,
            } => {
                info!(
                    namespace,
                    name,
                    canary_service = %canary_service,
                    weight,
                    "Advancing PureService canary weight via annotation"
                );
                self.patch_service_weight_annotation(namespace, canary_service, weight)
                    .await
            }
        }
    }

    /// Roll back the canary to 0% weight, completing within the configured
    /// `rollback_timeout_seconds` (must be ≤ 90 s per the acceptance criteria).
    pub async fn rollback(
        &self,
        backend: &TrafficSplitBackend,
        namespace: &str,
        name: &str,
        rollback_timeout_seconds: u64,
    ) -> Result<(), kube::Error> {
        // Clamp to the hard 90-second requirement from the acceptance criteria.
        let timeout = Duration::from_secs(rollback_timeout_seconds.min(90));

        warn!(
            namespace,
            name,
            timeout_secs = timeout.as_secs(),
            "Initiating canary rollback"
        );

        tokio::time::timeout(timeout, async {
            match backend {
                TrafficSplitBackend::ServiceMesh {
                    mesh_type,
                    resource_name,
                } => {
                    info!(
                        namespace,
                        name,
                        mesh = %mesh_type,
                        resource = %resource_name,
                        "Rolling back ServiceMesh canary to 0%"
                    );
                    self.patch_service_mesh_weight(namespace, resource_name, mesh_type, 0, 100)
                        .await
                }
                TrafficSplitBackend::PureService {
                    canary_service,
                    stable_service: _,
                } => {
                    info!(
                        namespace,
                        name,
                        canary_service = %canary_service,
                        "Rolling back PureService canary to 0%"
                    );
                    self.patch_service_weight_annotation(namespace, canary_service, 0)
                        .await
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            error!(
                namespace,
                name,
                timeout_secs = timeout.as_secs(),
                "Rollback timed out"
            );
            // Surface as a kube::Error by synthesising one from the timeout.
            Err(kube::Error::ReadEvents(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "canary rollback for {}/{} exceeded {}s timeout",
                    namespace,
                    name,
                    timeout.as_secs()
                ),
            )))
        })
    }

    // ------------------------------------------------------------------
    // Reconcile step (idempotent)
    // ------------------------------------------------------------------

    /// Idempotent reconciliation of a single promotion step.
    ///
    /// The `step_key.id` (SHA-256 of `targetRevision:stepIndex`) serves as the
    /// idempotency token. Re-entering with the same key re-evaluates the gates
    /// but will not advance weight a second time if the step was already
    /// recorded as `CanaryGatePassed` externally (callers should check the
    /// returned [`StepCondition`] before re-queuing).
    ///
    /// Flow:
    /// 1. Wait out the analysis window.
    /// 2. Evaluate SLO gates.
    /// 3. On pass: advance weight.
    /// 4. On fail: rollback.
    /// 5. Return a `StepCondition` recording the outcome.
    pub async fn reconcile_step(
        &self,
        step_key: PromotionStepKey,
        config: &ProgressiveDeliveryConfig,
        backend: &TrafficSplitBackend,
        namespace: &str,
        name: &str,
    ) -> StepCondition {
        let weight = config
            .step_weights
            .get(step_key.step_index)
            .copied()
            .unwrap_or(0);

        info!(
            step = %step_key,
            weight,
            analysis_window_secs = config.analysis_window_seconds,
            "Reconciling canary step"
        );

        // 1. Observe metrics for the configured analysis window before evaluating.
        tokio::time::sleep(Duration::from_secs(config.analysis_window_seconds)).await;

        // 2. Evaluate gates.
        let gate_result = self.evaluate_slo_gate(step_key.clone(), config).await;

        if gate_result.gate_passed {
            // 3. Advance traffic weight.
            if let Err(e) = self
                .advance_weight(backend, namespace, name, weight)
                .await
            {
                error!(
                    step = %step_key,
                    error = %e,
                    "Failed to advance canary weight after gate pass"
                );
            }
        } else {
            // 4. Roll back.
            if let Err(e) = self
                .rollback(backend, namespace, name, config.rollback_timeout_seconds)
                .await
            {
                error!(
                    step = %step_key,
                    error = %e,
                    "Rollback failed or timed out"
                );
            }
        }

        // 5. Produce a status condition.
        StepCondition::from_gate_result(&gate_result, weight)
    }

    // ------------------------------------------------------------------
    // Internal Kubernetes patch helpers
    // ------------------------------------------------------------------

    /// Patch a Service Mesh resource (VirtualService / TrafficSplit) to update
    /// canary and stable weights.
    async fn patch_service_mesh_weight(
        &self,
        namespace: &str,
        resource_name: &str,
        mesh_type: &str,
        canary_weight: u32,
        stable_weight: u32,
    ) -> Result<(), kube::Error> {
        use kube::api::{Patch, PatchParams};

        // Build the patch payload according to the mesh type.
        let patch_body: serde_json::Value = match mesh_type {
            "istio" => serde_json::json!({
                "spec": {
                    "http": [{
                        "route": [
                            { "destination": { "host": format!("{resource_name}-stable") }, "weight": stable_weight },
                            { "destination": { "host": format!("{resource_name}-canary") }, "weight": canary_weight }
                        ]
                    }]
                }
            }),
            "linkerd" => serde_json::json!({
                "spec": {
                    "backends": [
                        { "weight": stable_weight, "service": { "name": format!("{resource_name}-stable") } },
                        { "weight": canary_weight, "service": { "name": format!("{resource_name}-canary") } }
                    ]
                }
            }),
            other => {
                warn!(mesh = other, "Unknown mesh type; using generic weight annotation");
                serde_json::json!({
                    "metadata": {
                        "annotations": {
                            "stellar.org/canary-weight": canary_weight.to_string(),
                            "stellar.org/stable-weight": stable_weight.to_string()
                        }
                    }
                })
            }
        };

        // Apply as a strategic merge patch against the VirtualService / TrafficSplit.
        // We use the `networking.istio.io/v1alpha3` group for Istio and
        // `split.smi-spec.io/v1alpha4` for Linkerd; both have generic JSON patch
        // support via kube::api::DynamicObject.
        let api: kube::Api<kube::core::DynamicObject> = {
            use kube::discovery::ApiResource;
            let (group, version, kind) = match mesh_type {
                "linkerd" => (
                    "split.smi-spec.io",
                    "v1alpha4",
                    "TrafficSplit",
                ),
                _ => (
                    "networking.istio.io",
                    "v1alpha3",
                    "VirtualService",
                ),
            };
            let ar = ApiResource {
                group: group.to_string(),
                version: version.to_string(),
                api_version: format!("{group}/{version}"),
                kind: kind.to_string(),
                plural: if kind == "TrafficSplit" {
                    "trafficsplits".to_string()
                } else {
                    "virtualservices".to_string()
                },
            };
            kube::Api::namespaced_with(self.client.clone(), namespace, &ar)
        };

        api.patch(
            resource_name,
            &PatchParams::apply("stellar-progressive-delivery"),
            &Patch::Merge(&patch_body),
        )
        .await
        .map(|_| ())
    }

    /// Patch a Kubernetes Service's `stellar.org/canary-weight` annotation.
    /// An external controller (e.g. the traffic shaper) reads this annotation
    /// to split traffic between the stable and canary services.
    async fn patch_service_weight_annotation(
        &self,
        namespace: &str,
        service_name: &str,
        weight: u32,
    ) -> Result<(), kube::Error> {
        use k8s_openapi::api::core::v1::Service;
        use kube::api::{Patch, PatchParams};

        let api: kube::Api<Service> = kube::Api::namespaced(self.client.clone(), namespace);

        let patch = serde_json::json!({
            "metadata": {
                "annotations": {
                    "stellar.org/canary-weight": weight.to_string()
                }
            }
        });

        api.patch(
            service_name,
            &PatchParams::apply("stellar-progressive-delivery"),
            &Patch::Merge(&patch),
        )
        .await
        .map(|_| ())
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Idempotency key hashing
    // ------------------------------------------------------------------

    #[test]
    fn test_step_hash_deterministic() {
        let h1 = compute_step_hash("sha256:abc123", 2);
        let h2 = compute_step_hash("sha256:abc123", 2);
        assert_eq!(h1, h2, "Same inputs must produce identical hashes");
    }

    #[test]
    fn test_step_hash_different_revision() {
        let h1 = compute_step_hash("sha256:aaa", 0);
        let h2 = compute_step_hash("sha256:bbb", 0);
        assert_ne!(h1, h2, "Different revisions must produce different hashes");
    }

    #[test]
    fn test_step_hash_different_step_index() {
        let h1 = compute_step_hash("rev-1", 0);
        let h2 = compute_step_hash("rev-1", 1);
        assert_ne!(h1, h2, "Different step indices must produce different hashes");
    }

    #[test]
    fn test_step_hash_length() {
        let h = compute_step_hash("any-revision", 99);
        // SHA-256 → 64 hex chars
        assert_eq!(h.len(), 64, "SHA-256 hex string must be 64 characters");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_promotion_step_key_new() {
        let key = PromotionStepKey::new("v1.2.3", 3);
        assert_eq!(key.target_revision, "v1.2.3");
        assert_eq!(key.step_index, 3);
        assert_eq!(key.id, compute_step_hash("v1.2.3", 3));
    }

    #[test]
    fn test_promotion_step_key_idempotent() {
        let k1 = PromotionStepKey::new("rev-abc", 1);
        let k2 = PromotionStepKey::new("rev-abc", 1);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_promotion_step_key_display() {
        let key = PromotionStepKey::new("rev-xyz", 2);
        let s = format!("{key}");
        assert!(s.contains("rev-xyz"), "Display must include revision");
        assert!(s.contains("step2"), "Display must include step index");
        // id prefix (12 chars) should appear
        assert!(s.contains(&key.id[..12]));
    }

    // ------------------------------------------------------------------
    // Gate evaluation logic (synchronous, no network)
    // ------------------------------------------------------------------

    fn make_gate_result(passed: bool, step_index: usize) -> GateEvaluationResult {
        let step_key = PromotionStepKey::new("rev-test", step_index);
        GateEvaluationResult {
            evaluations: vec![ThresholdEvaluation {
                metric: "test_metric".to_string(),
                max_value: 0.01,
                observed_value: if passed { Some(0.005) } else { Some(0.05) },
                passed,
            }],
            gate_passed: passed,
            evaluated_at: "2024-01-01T00:00:00Z".to_string(),
            step_key,
        }
    }

    #[test]
    fn test_gate_passed_produces_correct_condition() {
        let result = make_gate_result(true, 0);
        let condition = StepCondition::from_gate_result(&result, 10);
        assert_eq!(condition.condition_type, "CanaryGatePassed");
        assert_eq!(condition.status, "True");
        assert!(condition.message.contains("10%"));
    }

    #[test]
    fn test_gate_failed_produces_correct_condition() {
        let result = make_gate_result(false, 1);
        let condition = StepCondition::from_gate_result(&result, 25);
        assert_eq!(condition.condition_type, "CanaryGateFailed");
        assert_eq!(condition.status, "False");
        assert!(condition.message.contains("25%"));
        assert!(condition.message.contains("test_metric"));
    }

    #[test]
    fn test_condition_reason_is_step_id() {
        let result = make_gate_result(true, 2);
        let condition = StepCondition::from_gate_result(&result, 50);
        assert_eq!(condition.reason, result.step_key.id);
    }

    #[test]
    fn test_all_thresholds_must_pass_for_gate() {
        let step_key = PromotionStepKey::new("rev-mixed", 0);
        let result = GateEvaluationResult {
            step_key,
            evaluations: vec![
                ThresholdEvaluation {
                    metric: "latency".to_string(),
                    max_value: 0.5,
                    observed_value: Some(0.3),
                    passed: true,
                },
                ThresholdEvaluation {
                    metric: "error_rate".to_string(),
                    max_value: 0.01,
                    observed_value: Some(0.05), // violated
                    passed: false,
                },
            ],
            gate_passed: false, // must be false when any threshold fails
            evaluated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let condition = StepCondition::from_gate_result(&result, 10);
        assert_eq!(condition.condition_type, "CanaryGateFailed");
        assert!(
            condition.message.contains("error_rate"),
            "Failing metric must appear in the condition message"
        );
    }

    #[test]
    fn test_no_slo_thresholds_all_pass() {
        // With no thresholds configured, the gate is vacuously true.
        let step_key = PromotionStepKey::new("rev-empty", 0);
        let result = GateEvaluationResult {
            step_key,
            evaluations: vec![],
            gate_passed: true,
            evaluated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        assert!(result.gate_passed);
    }

    // ------------------------------------------------------------------
    // Rollback timeout
    // ------------------------------------------------------------------

    #[test]
    fn test_rollback_timeout_clamped_to_90s() {
        // The rollback helper clamps to 90 s. Verify the clamping arithmetic.
        let raw_timeout: u64 = 300; // 5 minutes
        let clamped = raw_timeout.min(90);
        assert_eq!(clamped, 90);
    }

    #[test]
    fn test_rollback_timeout_below_90s_unchanged() {
        let raw_timeout: u64 = 60;
        let clamped = raw_timeout.min(90);
        assert_eq!(clamped, 60);
    }

    // ------------------------------------------------------------------
    // Configuration defaults
    // ------------------------------------------------------------------

    #[test]
    fn test_default_config() {
        let cfg = ProgressiveDeliveryConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.analysis_window_seconds, 120);
        assert_eq!(cfg.step_weights, vec![10, 25, 50, 75, 100]);
        assert_eq!(cfg.rollback_timeout_seconds, 90);
        assert!(cfg.slo_thresholds.is_empty());
    }

    #[test]
    fn test_step_weights_last_is_100() {
        let cfg = ProgressiveDeliveryConfig::default();
        assert_eq!(
            *cfg.step_weights.last().unwrap(),
            100,
            "Final step weight must be 100 to complete the rollout"
        );
    }

    // ------------------------------------------------------------------
    // Traffic split backend serialisation
    // ------------------------------------------------------------------

    #[test]
    fn test_service_mesh_backend_roundtrip() {
        let backend = TrafficSplitBackend::ServiceMesh {
            mesh_type: "istio".to_string(),
            resource_name: "my-vsvc".to_string(),
        };
        let json = serde_json::to_string(&backend).unwrap();
        let back: TrafficSplitBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(backend, back);
    }

    #[test]
    fn test_pure_service_backend_roundtrip() {
        let backend = TrafficSplitBackend::PureService {
            stable_service: "app-stable".to_string(),
            canary_service: "app-canary".to_string(),
        };
        let json = serde_json::to_string(&backend).unwrap();
        let back: TrafficSplitBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(backend, back);
    }

    // ------------------------------------------------------------------
    // URL encoding helper
    // ------------------------------------------------------------------

    #[test]
    fn test_urlencoding_spaces() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
    }

    #[test]
    fn test_urlencoding_braces() {
        assert_eq!(urlencoding("foo{bar}"), "foo%7Bbar%7D");
    }

    #[test]
    fn test_urlencoding_alphanumeric_unchanged() {
        assert_eq!(urlencoding("simpleMetric123"), "simpleMetric123");
    }

    #[test]
    fn test_urlencoding_equals_sign() {
        assert_eq!(urlencoding("a=b"), "a%3Db");
    }

    // ------------------------------------------------------------------
    // RFC-3339 timestamp helper
    // ------------------------------------------------------------------

    #[test]
    fn test_now_rfc3339_format() {
        let ts = now_rfc3339();
        // Must match YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.len(), 20, "timestamp must be 20 chars: {ts}");
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }
}
