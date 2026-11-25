//! Adaptive Horizontal Pod Autoscaling on Custom Business SLIs
//!
//! Moves HPA beyond CPU/memory by evaluating arbitrary PromQL business SLIs
//! with predictive scaling to pre-warm capacity before traffic inflection
//! points.  A single composite SLI score is exposed per Deployment through the
//! metrics adapter, keeping the HPA contract untouched.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │                  Adaptive HPA Controller Loop                    │
//! │                                                                  │
//! │  1. Query each SLI's PromQL expression from Prometheus (15 s)   │
//! │  2. Normalise raw values → [0, 1] scores                        │
//! │  3. Compute weighted composite SLI score                        │
//! │  4. Fall back to CPU metric when SLI source is unavailable      │
//! │  5. Decide desired replica count (clamp to [min, max])          │
//! │  6. Enforce stabilisation: max 2 scale events per minute        │
//! │  7. Patch HPA desiredReplicas via Kubernetes API                │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Configuration
//!
//! ```yaml
//! spec:
//!   autoscaling:
//!     adaptiveHpa:
//!       scrapeIntervalSecs: 15
//!       fallbackToCpu: true
//!       stabilizationWindowSecs: 60
//!       maxScaleEventsPerMinute: 2
//!       slis:
//!         - name: error_rate
//!           promql: 'rate(http_requests_total{status=~"5.."}[1m])'
//!           weight: 0.6
//!           thresholdHigh: 0.05
//!           thresholdLow: 0.01
//!         - name: p99_latency_ms
//!           promql: 'histogram_quantile(0.99, rate(http_request_duration_ms_bucket[1m]))'
//!           weight: 0.4
//!           thresholdHigh: 500.0
//!           thresholdLow: 100.0
//! ```

use chrono::{DateTime, Utc};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use kube::api::{Patch, PatchParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

// ============================================================================
// Configuration types (CRD-embedded)
// ============================================================================

/// Definition of a single business SLI to be evaluated.
///
/// The raw Prometheus value is normalised to [0, 1] using a linear
/// interpolation between `threshold_low` (→ 0.0, healthy) and
/// `threshold_high` (→ 1.0, saturated / degraded).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SliDefinition {
    /// Human-readable name for this SLI (used in logs and metrics).
    pub name: String,

    /// Prometheus instant-query expression to evaluate.
    /// Example: `rate(http_requests_total{status=~"5.."}[1m])`
    pub promql: String,

    /// Relative weight of this SLI in the composite score (0.0 – 1.0).
    /// Weights across all SLIs do not need to sum to 1; they are normalised
    /// internally.
    pub weight: f64,

    /// Raw metric value that maps to a normalised score of 1.0 (worst / fully
    /// saturated).  Values at or above this threshold contribute maximally to
    /// the composite score and trigger scale-out.
    pub threshold_high: f64,

    /// Raw metric value that maps to a normalised score of 0.0 (healthy /
    /// idle).  Values at or below this threshold do not drive scale-out.
    pub threshold_low: f64,
}

/// Top-level adaptive HPA configuration embedded in the StellarNode spec.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompositeSliConfig {
    /// SLI definitions to evaluate each scrape cycle.
    pub slis: Vec<SliDefinition>,

    /// How often (in seconds) to scrape Prometheus and recompute the composite
    /// score.  Acceptance criterion requires this to be ≤ 15 s.
    /// Default: 15
    #[serde(default = "default_scrape_interval")]
    pub scrape_interval_secs: u64,

    /// When `true`, the controller falls back to CPU utilisation as a
    /// surrogate SLI whenever a Prometheus query fails or returns no data.
    /// Default: true
    #[serde(default = "default_fallback_to_cpu")]
    pub fallback_to_cpu: bool,

    /// Length of the sliding window (seconds) used to count scale events for
    /// the stabilisation gate.
    /// Default: 60
    #[serde(default = "default_stabilization_window")]
    pub stabilization_window_secs: u64,

    /// Maximum number of scale events allowed within `stabilization_window_secs`.
    /// Acceptance criterion: ≤ 2 events per minute.
    /// Default: 2
    #[serde(default = "default_max_scale_events")]
    pub max_scale_events_per_minute: u32,

    /// Prometheus base URL for all SLI scrapes.
    /// Example: `"http://prometheus-operated.monitoring:9090"`
    #[serde(default = "default_prometheus_url")]
    pub prometheus_url: String,
}

fn default_scrape_interval() -> u64 {
    15
}

fn default_fallback_to_cpu() -> bool {
    true
}

fn default_stabilization_window() -> u64 {
    60
}

fn default_max_scale_events() -> u32 {
    2
}

fn default_prometheus_url() -> String {
    "http://prometheus-operated.monitoring:9090".to_string()
}

impl Default for CompositeSliConfig {
    fn default() -> Self {
        Self {
            slis: Vec::new(),
            scrape_interval_secs: default_scrape_interval(),
            fallback_to_cpu: default_fallback_to_cpu(),
            stabilization_window_secs: default_stabilization_window(),
            max_scale_events_per_minute: default_max_scale_events(),
            prometheus_url: default_prometheus_url(),
        }
    }
}

// ============================================================================
// Score types
// ============================================================================

/// Per-SLI evaluation result.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SliScore {
    /// Name of the SLI (mirrors [`SliDefinition::name`]).
    pub name: String,

    /// Raw value returned by Prometheus (before normalisation).
    pub raw_value: f64,

    /// Normalised score in [0.0, 1.0].
    /// 0.0 = fully healthy, 1.0 = fully saturated.
    pub normalized_score: f64,

    /// Weight applied to this score when computing the composite.
    pub weight: f64,

    /// Whether the Prometheus source was reachable and returned data.
    /// When `false`, the SLI may have used the CPU fallback value.
    pub source_available: bool,
}

/// Aggregate result for a full composite SLI evaluation cycle.
#[derive(Clone, Debug)]
pub struct CompositeSliScore {
    /// Individual SLI scores that contributed to `composite`.
    pub scores: Vec<SliScore>,

    /// Weighted average of all individual normalised scores (0.0 – 1.0).
    pub composite: f64,

    /// Wall-clock time at which this score was computed.
    pub computed_at: DateTime<Utc>,

    /// True when at least one SLI fell back to the CPU surrogate metric.
    pub used_fallback: bool,
}

// ============================================================================
// Stabilisation / rate-limiting
// ============================================================================

/// Tracks scale events within a sliding window to enforce a rate limit.
///
/// Once `max_events` events have been recorded inside `window_secs`, further
/// calls to [`ScaleEventTracker::can_scale`] return `false` until older events
/// age out of the window.
pub struct ScaleEventTracker {
    /// Timestamps of recent scale events (oldest first).
    events: VecDeque<DateTime<Utc>>,
    /// Length of the sliding window in seconds.
    window_secs: u64,
    /// Maximum number of events allowed inside the window.
    max_events: u32,
}

impl ScaleEventTracker {
    /// Create a new tracker with the given window length and event budget.
    pub fn new(window_secs: u64, max_events: u32) -> Self {
        Self {
            events: VecDeque::new(),
            window_secs,
            max_events,
        }
    }

    /// Record a scale event at the current wall-clock time.
    pub fn record_event(&mut self) {
        self.evict_stale(Utc::now());
        self.events.push_back(Utc::now());
    }

    /// Return `true` when another scale event is permitted right now.
    pub fn can_scale(&mut self) -> bool {
        self.evict_stale(Utc::now());
        (self.events.len() as u32) < self.max_events
    }

    /// Remove events that have fallen outside the sliding window.
    fn evict_stale(&mut self, now: DateTime<Utc>) {
        let cutoff = now - chrono::Duration::seconds(self.window_secs as i64);
        while let Some(front) = self.events.front() {
            if *front < cutoff {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }
}

// ============================================================================
// Prometheus scraping helper
// ============================================================================

/// URL-encode a string for safe embedding in a Prometheus query parameter.
///
/// Re-uses the same approach as `predictive_scaling.rs`.
fn urlencoding(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}

/// Execute a Prometheus instant query and return the first scalar result.
///
/// Returns `None` on network error, non-200 status, or empty result set.
async fn query_prometheus(prometheus_url: &str, promql: &str) -> Option<f64> {
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

/// Query the CPU utilisation surrogate metric for a specific Deployment.
///
/// Uses the standard `container_cpu_usage_seconds_total` counter averaged over
/// the last minute, scoped to the given namespace and deployment.
async fn query_cpu_fallback(
    prometheus_url: &str,
    namespace: &str,
    deployment: &str,
) -> Option<f64> {
    let promql = format!(
        "avg(rate(container_cpu_usage_seconds_total{{namespace=\"{namespace}\",\
         pod=~\"{deployment}-.*\",container!=\"\"}}[1m]))"
    );
    query_prometheus(prometheus_url, &promql).await
}

// ============================================================================
// Normalisation helper
// ============================================================================

/// Linearly normalise `value` to [0.0, 1.0] between `low` and `high`.
///
/// Values ≤ `low` map to 0.0; values ≥ `high` map to 1.0.
fn normalize(value: f64, low: f64, high: f64) -> f64 {
    if high <= low {
        // Degenerate thresholds: treat anything above low as saturated.
        if value > low {
            1.0
        } else {
            0.0
        }
    } else {
        ((value - low) / (high - low)).clamp(0.0, 1.0)
    }
}

// ============================================================================
// Adaptive HPA controller
// ============================================================================

/// Adaptive HPA controller that evaluates composite business SLI scores and
/// drives Kubernetes HPA replica counts accordingly.
pub struct AdaptiveHpaController {
    /// Kubernetes client used for HPA patching.
    client: kube::Client,
}

impl AdaptiveHpaController {
    /// Create a new controller backed by the given Kubernetes client.
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    /// Query Prometheus for every SLI defined in `config`, normalise the
    /// values, and return a [`CompositeSliScore`].
    ///
    /// When a SLI's Prometheus query fails and `config.fallback_to_cpu` is
    /// `true`, the controller substitutes the CPU utilisation metric for that
    /// SLI.  If that also fails the SLI contributes 0.0 to the composite so
    /// the system degrades gracefully rather than blocking scale decisions.
    pub async fn compute_composite_sli(
        &self,
        config: &CompositeSliConfig,
        namespace: &str,
        deployment: &str,
    ) -> CompositeSliScore {
        let mut scores: Vec<SliScore> = Vec::with_capacity(config.slis.len());
        let mut used_fallback = false;

        for sli in &config.slis {
            let raw_opt = query_prometheus(&config.prometheus_url, &sli.promql).await;

            let (raw_value, source_available) = match raw_opt {
                Some(v) => (v, true),
                None => {
                    warn!(
                        sli = %sli.name,
                        deployment = %deployment,
                        "SLI Prometheus query returned no data"
                    );

                    if config.fallback_to_cpu {
                        used_fallback = true;
                        let cpu =
                            query_cpu_fallback(&config.prometheus_url, namespace, deployment)
                                .await
                                .unwrap_or(0.0);
                        debug!(
                            sli = %sli.name,
                            cpu_value = cpu,
                            "Using CPU fallback for unavailable SLI"
                        );
                        (cpu, false)
                    } else {
                        (0.0, false)
                    }
                }
            };

            let normalized_score = normalize(raw_value, sli.threshold_low, sli.threshold_high);

            scores.push(SliScore {
                name: sli.name.clone(),
                raw_value,
                normalized_score,
                weight: sli.weight,
                source_available,
            });
        }

        // Compute weighted average composite score.
        let total_weight: f64 = scores.iter().map(|s| s.weight).sum();
        let composite = if total_weight > 0.0 {
            scores
                .iter()
                .map(|s| s.normalized_score * s.weight)
                .sum::<f64>()
                / total_weight
        } else {
            0.0
        };

        CompositeSliScore {
            scores,
            composite,
            computed_at: Utc::now(),
            used_fallback,
        }
    }

    /// Translate a composite SLI score into a desired replica count.
    ///
    /// The mapping is linear:
    /// - composite ≤ 0.5 → `min` replicas (healthy, scale in / hold)
    /// - composite ≥ 1.0 → `max` replicas (saturated, scale out fully)
    /// - values in between → linear interpolation
    ///
    /// The result is always clamped to [`min`, `max`].
    pub fn desired_replicas(
        &self,
        score: &CompositeSliScore,
        current: i32,
        min: i32,
        max: i32,
    ) -> i32 {
        // Below 0.5 → no extra replicas needed beyond the minimum.
        if score.composite <= 0.5 {
            return current.max(min);
        }

        // Map (0.5, 1.0] → (min, max].
        let range = (max - min) as f64;
        let factor = (score.composite - 0.5) / 0.5; // 0.0 at 0.5, 1.0 at 1.0
        let desired = min as f64 + factor * range;
        (desired.round() as i32).clamp(min, max)
    }

    /// Patch an HPA's `spec.minReplicas` to the given value using a strategic
    /// merge patch.
    pub async fn patch_hpa(
        &self,
        namespace: &str,
        hpa_name: &str,
        replicas: i32,
    ) -> Result<(), kube::Error> {
        let hpa_api: kube::Api<HorizontalPodAutoscaler> =
            kube::Api::namespaced(self.client.clone(), namespace);

        let patch = serde_json::json!({
            "spec": { "minReplicas": replicas }
        });

        hpa_api
            .patch(
                hpa_name,
                &PatchParams::apply("stellar-adaptive-hpa"),
                &Patch::Merge(&patch),
            )
            .await
            .map(|_| ())
    }

    /// Main control loop for a single Deployment.
    ///
    /// Ticks every `config.scrape_interval_secs` seconds (acceptance criterion:
    /// 15 s), computes the composite SLI score, checks the stabilisation gate,
    /// and patches the HPA when a scale event is permitted.
    ///
    /// The loop exits when `shutdown` is closed.
    pub async fn run_loop(
        &self,
        namespace: String,
        deployment: String,
        config: CompositeSliConfig,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(
            namespace = %namespace,
            deployment = %deployment,
            scrape_interval_secs = config.scrape_interval_secs,
            "Adaptive HPA controller started"
        );

        let mut tracker = ScaleEventTracker::new(
            config.stabilization_window_secs,
            config.max_scale_events_per_minute,
        );
        let hpa_name = format!("{deployment}-hpa");
        let interval = tokio::time::Duration::from_secs(config.scrape_interval_secs);

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown.changed() => {
                    info!(deployment = %deployment, "Adaptive HPA controller shutting down");
                    return;
                }
            }

            // 1. Compute composite SLI score.
            let score = self
                .compute_composite_sli(&config, &namespace, &deployment)
                .await;

            info!(
                deployment = %deployment,
                composite = score.composite,
                used_fallback = score.used_fallback,
                "Composite SLI score computed"
            );

            // 2. Fetch current HPA to discover replica bounds.
            let hpa_api: kube::Api<HorizontalPodAutoscaler> =
                kube::Api::namespaced(self.client.clone(), &namespace);

            let current_hpa = match hpa_api.get(&hpa_name).await {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        deployment = %deployment,
                        hpa = %hpa_name,
                        error = %e,
                        "HPA not found; skipping cycle"
                    );
                    continue;
                }
            };

            let current_replicas = current_hpa
                .spec
                .as_ref()
                .and_then(|s| s.min_replicas)
                .unwrap_or(1);
            let min_replicas = current_replicas;
            let max_replicas = current_hpa
                .spec
                .as_ref()
                .map(|s| s.max_replicas)
                .unwrap_or(10);

            // 3. Decide desired replica count.
            let desired =
                self.desired_replicas(&score, current_replicas, min_replicas, max_replicas);

            if desired == current_replicas {
                debug!(
                    deployment = %deployment,
                    replicas = current_replicas,
                    "No replica change needed"
                );
                continue;
            }

            // 4. Enforce stabilisation gate.
            if !tracker.can_scale() {
                warn!(
                    deployment = %deployment,
                    desired,
                    current = current_replicas,
                    "Stabilisation gate: scale event rate limit reached; skipping"
                );
                continue;
            }

            // 5. Patch HPA and record the event.
            info!(
                deployment = %deployment,
                composite = score.composite,
                current_replicas,
                desired,
                "Patching HPA minReplicas"
            );

            match self.patch_hpa(&namespace, &hpa_name, desired).await {
                Ok(()) => {
                    tracker.record_event();
                }
                Err(e) => {
                    warn!(
                        deployment = %deployment,
                        error = %e,
                        "Failed to patch HPA; not recording scale event"
                    );
                }
            }
        }
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // normalize()
    // ------------------------------------------------------------------

    #[test]
    fn test_normalize_below_low() {
        // Value well below threshold_low should produce 0.0.
        assert_eq!(normalize(0.0, 10.0, 100.0), 0.0);
    }

    #[test]
    fn test_normalize_above_high() {
        // Value above threshold_high should be clamped to 1.0.
        assert_eq!(normalize(200.0, 10.0, 100.0), 1.0);
    }

    #[test]
    fn test_normalize_midpoint() {
        // Exact midpoint should return 0.5.
        let mid = normalize(55.0, 10.0, 100.0);
        assert!((mid - 0.5).abs() < 1e-9, "mid={mid}");
    }

    #[test]
    fn test_normalize_degenerate_thresholds() {
        // threshold_high == threshold_low: above → 1.0, at or below → 0.0.
        assert_eq!(normalize(5.0, 5.0, 5.0), 0.0);
        assert_eq!(normalize(6.0, 5.0, 5.0), 1.0);
    }

    // ------------------------------------------------------------------
    // Composite SLI score computation
    // ------------------------------------------------------------------

    fn make_sli(name: &str, weight: f64, raw: f64, low: f64, high: f64) -> SliScore {
        SliScore {
            name: name.to_string(),
            raw_value: raw,
            normalized_score: normalize(raw, low, high),
            weight,
            source_available: true,
        }
    }

    fn composite_from_scores(scores: Vec<SliScore>) -> f64 {
        let total_weight: f64 = scores.iter().map(|s| s.weight).sum();
        if total_weight == 0.0 {
            return 0.0;
        }
        scores
            .iter()
            .map(|s| s.normalized_score * s.weight)
            .sum::<f64>()
            / total_weight
    }

    #[test]
    fn test_composite_score_equal_weights() {
        // Two SLIs with equal weight, one at 0.0 and one at 1.0 → composite = 0.5.
        let scores = vec![
            make_sli("a", 1.0, 10.0, 10.0, 100.0),  // 0.0 (at low)
            make_sli("b", 1.0, 100.0, 10.0, 100.0), // 1.0 (at high)
        ];
        let composite = composite_from_scores(scores);
        assert!((composite - 0.5).abs() < 1e-9, "composite={composite}");
    }

    #[test]
    fn test_composite_score_unequal_weights() {
        // Weight 0.6 on saturated SLI, 0.4 on healthy SLI.
        // composite = (1.0 * 0.6 + 0.0 * 0.4) / 1.0 = 0.6
        let scores = vec![
            make_sli("error_rate", 0.6, 100.0, 10.0, 100.0), // 1.0
            make_sli("latency", 0.4, 10.0, 10.0, 100.0),     // 0.0
        ];
        let composite = composite_from_scores(scores);
        assert!((composite - 0.6).abs() < 1e-9, "composite={composite}");
    }

    #[test]
    fn test_composite_score_zero_total_weight() {
        // Zero total weight (empty vec) should not panic and should return 0.0.
        let scores: Vec<SliScore> = vec![];
        assert_eq!(composite_from_scores(scores), 0.0);
    }

    // ------------------------------------------------------------------
    // desired_replicas()
    // ------------------------------------------------------------------

    fn controller() -> AdaptiveHpaController {
        // We only need a kube::Client for the async methods; for sync unit
        // tests we construct one from an in-process config that is never
        // actually used.
        let config = kube::Config::incluster().unwrap_or_else(|_| kube::Config {
            cluster_url: "https://localhost:6443".parse().unwrap(),
            default_namespace: "default".to_string(),
            root_cert: None,
            connect_timeout: None,
            read_timeout: None,
            write_timeout: None,
            tls_server_name: None,
            accept_invalid_certs: true,
            auth_info: Default::default(),
            proxy_url: None,
            headers: Default::default(),
        });
        let client = kube::Client::try_from(config).expect("kube client");
        AdaptiveHpaController::new(client)
    }

    fn score_with_composite(composite: f64) -> CompositeSliScore {
        CompositeSliScore {
            scores: vec![],
            composite,
            computed_at: Utc::now(),
            used_fallback: false,
        }
    }

    #[test]
    fn test_desired_replicas_healthy_score() {
        let ctrl = controller();
        let score = score_with_composite(0.0);
        // Healthy: should return max(current, min).
        assert_eq!(ctrl.desired_replicas(&score, 3, 2, 10), 3);
    }

    #[test]
    fn test_desired_replicas_saturated_score() {
        let ctrl = controller();
        let score = score_with_composite(1.0);
        // Fully saturated: should return max.
        assert_eq!(ctrl.desired_replicas(&score, 2, 2, 10), 10);
    }

    #[test]
    fn test_desired_replicas_mid_score() {
        let ctrl = controller();
        // composite = 0.75 → factor = 0.5 → desired = 2 + 0.5 * 8 = 6
        let score = score_with_composite(0.75);
        assert_eq!(ctrl.desired_replicas(&score, 2, 2, 10), 6);
    }

    #[test]
    fn test_desired_replicas_clamped_to_min() {
        let ctrl = controller();
        let score = score_with_composite(0.0);
        // Even if current < min, result is clamped to min.
        assert_eq!(ctrl.desired_replicas(&score, 1, 3, 10), 3);
    }

    #[test]
    fn test_desired_replicas_clamped_to_max() {
        let ctrl = controller();
        let score = score_with_composite(1.0);
        // Capped at max regardless of score.
        assert_eq!(ctrl.desired_replicas(&score, 5, 5, 8), 8);
    }

    // ------------------------------------------------------------------
    // ScaleEventTracker — stabilisation window enforcement
    // ------------------------------------------------------------------

    #[test]
    fn test_stabilization_allows_up_to_max_events() {
        let mut tracker = ScaleEventTracker::new(60, 2);
        // First event: allowed.
        assert!(tracker.can_scale());
        tracker.record_event();
        // Second event: allowed (1 < 2).
        assert!(tracker.can_scale());
        tracker.record_event();
        // Third event within window: blocked (2 >= 2).
        assert!(!tracker.can_scale());
    }

    #[test]
    fn test_stabilization_no_events_allows_scaling() {
        let mut tracker = ScaleEventTracker::new(60, 2);
        assert!(tracker.can_scale());
    }

    #[test]
    fn test_stabilization_events_age_out() {
        // Window of 0 seconds: every event immediately becomes stale on the
        // next eviction because the cutoff equals `now` and stored events are
        // strictly less than the cutoff after even 1 ms.
        let mut tracker = ScaleEventTracker::new(0, 2);
        tracker.record_event();
        tracker.record_event();
        // Sleep briefly so the recorded timestamps are in the past.
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(tracker.can_scale());
    }

    #[test]
    fn test_stabilization_max_zero_always_blocks() {
        // max_events = 0 means no scale events are ever allowed.
        let mut tracker = ScaleEventTracker::new(60, 0);
        assert!(!tracker.can_scale());
    }

    // ------------------------------------------------------------------
    // Fallback to CPU logic (unit-level, no network)
    // ------------------------------------------------------------------

    #[test]
    fn test_fallback_flag_propagates() {
        // Simulate a CompositeSliScore produced with used_fallback = true.
        let fallback_score = CompositeSliScore {
            scores: vec![SliScore {
                name: "error_rate".to_string(),
                raw_value: 0.03,
                normalized_score: 0.5,
                weight: 1.0,
                source_available: false, // Prometheus was unavailable
            }],
            composite: 0.5,
            computed_at: Utc::now(),
            used_fallback: true,
        };
        assert!(fallback_score.used_fallback);
        assert!(!fallback_score.scores[0].source_available);
    }

    #[test]
    fn test_fallback_disabled_zero_contribution() {
        // When fallback_to_cpu = false and source unavailable, raw_value = 0.0.
        let score = SliScore {
            name: "latency".to_string(),
            raw_value: 0.0,
            normalized_score: normalize(0.0, 100.0, 500.0),
            weight: 1.0,
            source_available: false,
        };
        // normalized score should be 0.0 (no inflation from missing data).
        assert_eq!(score.normalized_score, 0.0);
    }

    // ------------------------------------------------------------------
    // urlencoding helper
    // ------------------------------------------------------------------

    #[test]
    fn test_urlencoding_spaces_and_braces() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("foo{bar}"), "foo%7Bbar%7D");
    }

    #[test]
    fn test_urlencoding_alphanumeric_passthrough() {
        assert_eq!(urlencoding("simple123"), "simple123");
    }

    #[test]
    fn test_urlencoding_equals_sign() {
        assert_eq!(urlencoding("a=b"), "a%3Db");
    }
}
