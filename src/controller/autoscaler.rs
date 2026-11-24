//! Pending-queue-depth autoscaling for Soroban RPC nodes (issue #18).
//!
//! Standard HPA over CPU/memory is too slow for Soroban RPC's bursty load: by
//! the time utilization metrics accumulate, user transactions are already
//! dropping. This module drives scaling decisions directly from the node's
//! **pending request queue depth**, which reacts within one poll cycle.
//!
//! # Design
//!
//! - [`PendingQueueCollector`] polls each managed node's Prometheus metrics
//!   endpoint, parses the pending-queue gauge, and (a) publishes it as the
//!   `stellar_node_pending_rpc_queue` metric for observability and (b) feeds it
//!   to the autoscaler state.
//! - [`desired_replicas`] computes the target replica count with **pure integer
//!   arithmetic** (`ceil(pending / target)`), so there is no floating-point
//!   drift to reconcile across runs.
//! - [`QueueAutoscaler::evaluate`] issues an immediate **scale-up** on load
//!   bursts (fast reaction) and gates **scale-down** behind a stabilization
//!   window so a transient dip never thrashes the pod count.
//! - [`QueueAutoscaler::patch_deployment`] patches the Soroban RPC
//!   [`Deployment`] `spec.replicas` directly, so the operator owns the scaling
//!   decision end-to-end. If the user also configured a Kubernetes HPA, the
//!   HPA still owns CPU/utilization scaling and its `minReplicas` is bumped to
//!   the operator's desired count so the two never fight (see [`build_hpa`](crate::controller::resources)).
//!
//! # Invariants
//!
//! 1. Desired replicas are always in `[min_replicas, max_replicas]`.
//! 2. Scale-up fires on the first sample that exceeds capacity, subject only to
//!    the (default-zero) scale-up cooldown.
//! 3. Scale-down only fires after the desired count has stayed below the
//!    current count for the whole `stabilization_window_seconds` window.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Api, Patch, PatchParams};
use kube::ResourceExt;
use tracing::{debug, error, info};

use crate::controller::gas_autoscaling::parse_duration;
use crate::crd::QueueAutoscalingConfig;
use crate::crd::{NodeType, StellarNode};

// ============================================================================
// Core data types
// ============================================================================

/// A single pending-queue observation.
#[derive(Debug, Clone)]
pub struct PendingQueueSample {
    /// Pending requests observed in the node's RPC queue.
    pub pending: u64,
    /// When the sample was captured.
    pub sampled_at: Instant,
}

/// Shared mutable state for the queue autoscaling pipeline.
#[derive(Debug, Default)]
pub struct QueueAutoscalingState {
    pub current_replicas: i32,
    pub last_scale_up_at: Option<Instant>,
    pub last_scale_down_at: Option<Instant>,
    /// Rolling `(timestamp, desired_replicas)` history used to enforce the
    /// scale-down stabilization window.
    pub desired_history: VecDeque<(Instant, u32)>,
}

/// Reference to a managed `StellarNode`.
#[derive(Debug, Clone)]
pub struct StellarNodeRef {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

// ============================================================================
// Scaling decision types
// ============================================================================

/// Direction of a scaling event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleDirection {
    Up,
    Down,
}

/// Reason the autoscaler held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    CooldownActive { direction: ScaleDirection },
    AtBoundary,
    StabilizationActive,
    WithinTarget,
}

/// Outcome of one autoscaler evaluation cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalingDecision {
    ScaleUp { from: i32, to: i32, pending: u64 },
    ScaleDown { from: i32, to: i32, pending: u64 },
    Hold { reason: HoldReason },
}

// ============================================================================
// Pure scaling math (integer-only, no floating-point drift)
// ============================================================================

/// Compute the desired replica count for a pending queue depth using integer
/// arithmetic only.
///
/// `desired = ceil(pending / target)` clamped into `[min_replicas, max_replicas]`.
/// A zero target falls back to `min_replicas` rather than dividing by zero, and
/// an empty queue still yields at least `min_replicas` replicas.
pub fn desired_replicas(
    pending: u64,
    target_per_replica: u64,
    min_replicas: u32,
    max_replicas: u32,
) -> u32 {
    let min = min_replicas.max(1);
    let max = max_replicas.max(min);
    if target_per_replica == 0 || pending == 0 {
        return min;
    }
    // ceil(pending / target) without floating point.
    let desired = pending.div_ceil(target_per_replica).max(1) as u32;
    desired.clamp(min, max)
}

// ============================================================================
// Autoscaler
// ============================================================================

/// Drives the Soroban RPC Deployment replica count from pending queue depth.
pub struct QueueAutoscaler {
    pub config: QueueAutoscalingConfig,
    pub state: Arc<Mutex<QueueAutoscalingState>>,
    /// `None` in tests; `patch_deployment` becomes a no-op in that case.
    pub k8s_client: Option<kube::Client>,
    pub node_ref: StellarNodeRef,
}

impl QueueAutoscaler {
    pub fn new(config: QueueAutoscalingConfig, node_ref: StellarNodeRef) -> Self {
        let current_replicas = config.min_replicas as i32;
        Self {
            config,
            state: Arc::new(Mutex::new(QueueAutoscalingState {
                current_replicas,
                ..Default::default()
            })),
            k8s_client: None,
            node_ref,
        }
    }

    /// Current desired replica count for a given pending queue depth.
    pub fn desired_replicas(&self, pending: u64) -> u32 {
        desired_replicas(
            pending,
            self.config.target_pending_per_replica,
            self.config.min_replicas,
            self.config.max_replicas,
        )
    }

    /// Evaluate the latest pending-queue sample and produce a scaling decision.
    ///
    /// `now` is injectable so tests can simulate a clock deterministically.
    ///
    /// Rules:
    /// - **Scale-up** fires as soon as `desired > current` (subject to the
    ///   scale-up cooldown), reacting to a load burst within one poll cycle.
    /// - **Scale-down** only fires after `desired < current` has persisted for
    ///   the entire `stabilization_window_seconds` window; within the window it
    ///   uses the most conservative (max) desired count so a transient dip never
    ///   thrashes the pod count.
    pub fn evaluate(&self, pending: u64, now: Instant) -> ScalingDecision {
        let desired = self.desired_replicas(pending);
        let mut state = self.state.lock().unwrap();

        // Record this observation for the stabilization window, keeping the
        // history bounded to the window plus a small margin.
        let window = Duration::from_secs(self.config.stabilization_window_seconds as u64);
        state.desired_history.push_back((now, desired));
        while state
            .desired_history
            .front()
            .map(|(ts, _)| now.duration_since(*ts) > window + window)
            .unwrap_or(false)
        {
            state.desired_history.pop_front();
        }

        let current = state.current_replicas;

        // --- Scale up (fast path) ---
        if desired > current as u32 {
            if current >= self.config.max_replicas as i32 {
                return ScalingDecision::Hold {
                    reason: HoldReason::AtBoundary,
                };
            }
            if let Some(last_up) = state.last_scale_up_at {
                if let Ok(cooldown) = parse_duration(&self.config.scale_up_cooldown) {
                    if now.duration_since(last_up) < cooldown {
                        return ScalingDecision::Hold {
                            reason: HoldReason::CooldownActive {
                                direction: ScaleDirection::Up,
                            },
                        };
                    }
                }
            }
            let to = desired as i32;
            state.current_replicas = to;
            state.last_scale_up_at = Some(now);
            return ScalingDecision::ScaleUp {
                from: current,
                to,
                pending,
            };
        }

        // --- Scale down (stabilized) ---
        if desired < current as u32 {
            if current <= self.config.min_replicas as i32 {
                return ScalingDecision::Hold {
                    reason: HoldReason::AtBoundary,
                };
            }
            // Only scale down once the window has fully elapsed and the desired
            // count has stayed at or below `current` the entire time.
            if let Some(stabilized_desired) = stabilized_desired_in_window(
                &state.desired_history,
                now,
                window,
                current as u32,
            ) {
                if let Ok(cooldown) = parse_duration(&self.config.scale_down_cooldown) {
                    if let Some(last_down) = state.last_scale_down_at {
                        if now.duration_since(last_down) < cooldown {
                            return ScalingDecision::Hold {
                                reason: HoldReason::CooldownActive {
                                    direction: ScaleDirection::Down,
                                },
                            };
                        }
                    }
                }
                let to = stabilized_desired.max(self.config.min_replicas) as i32;
                state.current_replicas = to;
                state.last_scale_down_at = Some(now);
                return ScalingDecision::ScaleDown {
                    from: current,
                    to,
                    pending,
                };
            }
            return ScalingDecision::Hold {
                reason: HoldReason::StabilizationActive,
            };
        }

        ScalingDecision::Hold {
            reason: HoldReason::WithinTarget,
        }
    }

    /// Patch the Soroban RPC Deployment `spec.replicas` to the decision target.
    ///
    /// A no-op (returns `Ok`) when no `kube::Client` is available (tests).
    pub async fn patch_deployment(
        &self,
        decision: &ScalingDecision,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (from, to, reason) = match decision {
            ScalingDecision::ScaleUp { from, to, .. } => {
                (*from, *to, "pending queue above target")
            }
            ScalingDecision::ScaleDown { from, to, .. } => {
                (*from, *to, "pending queue below target (stabilized)")
            }
            ScalingDecision::Hold { reason } => {
                debug!("Holding queue autoscaling: {:?}", reason);
                return Ok(());
            }
        };

        let Some(client) = &self.k8s_client else {
            debug!(
                "No kube client (test mode): would scale {}/{} from {} to {}",
                self.node_ref.namespace, self.node_ref.name, from, to
            );
            return Ok(());
        };

        let deployments: Api<Deployment> = Api::namespaced(client.clone(), &self.node_ref.namespace);
        let patch = serde_json::json!({ "spec": { "replicas": to } });
        deployments
            .patch(
                &self.node_ref.name,
                &PatchParams::apply("stellar-operator-queue-autoscaler").force(),
                &Patch::Merge(&patch),
            )
            .await?;

        info!(
            "Scaled Soroban RPC Deployment {}/{} from {} to {} replicas ({})",
            self.node_ref.namespace, self.node_ref.name, from, to, reason
        );
        Ok(())
    }

    /// Run the autoscaling loop, sampling `pending_fn` every poll interval and
    /// applying decisions until `shutdown` fires.
    pub async fn run(
        &self,
        mut pending_fn: Box<dyn FnMut() -> u64 + Send>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let interval = Duration::from_secs(self.config.poll_interval_seconds as u64);
        let mut ticker = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let pending = pending_fn();
                    let decision = self.evaluate(pending, Instant::now());
                    if let Err(e) = self.patch_deployment(&decision).await {
                        error!("Failed to scale deployment for {}: {}", self.node_ref.name, e);
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Shutting down queue autoscaler for {}", self.node_ref.name);
                        break;
                    }
                }
            }
        }
    }
}

/// The stabilized scale-down target within the window, if the window has fully
/// elapsed and the desired count has stayed at or below `current` throughout.
///
/// Returns `None` while the window has not yet filled or if any observation
/// within the window exceeded `current` (a recent burst that should hold the
/// current capacity).
fn stabilized_desired_in_window(
    history: &VecDeque<(Instant, u32)>,
    now: Instant,
    window: Duration,
    current: u32,
) -> Option<u32> {
    let window_start = now.checked_sub(window)?;
    let within: Vec<&(Instant, u32)> = history.iter().filter(|(ts, _)| *ts >= window_start).collect();
    if within.is_empty() {
        return None;
    }
    // Require the window to have filled: the oldest retained observation must
    // predate the window start, otherwise we simply have not waited long enough.
    let oldest = history.front()?;
    if oldest.0 > window_start {
        return None;
    }
    let max_desired = within.iter().map(|(_, d)| *d).max()?;
    if max_desired < current {
        Some(max_desired)
    } else {
        None
    }
}

// ============================================================================
// Collector
// ============================================================================

/// Errors while collecting the pending queue metric from a node.
#[derive(Debug)]
pub enum QueueCollectionError {
    Network(String),
    HttpError { status: u16, body: String },
    ParseError(String),
}

impl std::fmt::Display for QueueCollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueCollectionError::Network(msg) => write!(f, "network error: {msg}"),
            QueueCollectionError::HttpError { status, body } => write!(f, "HTTP {status}: {body}"),
            QueueCollectionError::ParseError(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

/// Polls the Soroban RPC node's Prometheus metrics endpoint for the pending
/// queue gauge.
pub struct PendingQueueCollector {
    pub metric_url: String,
    pub metric_name: String,
    pub max_retries: u32,
}

impl PendingQueueCollector {
    pub async fn poll_once(&self) -> Result<u64, QueueCollectionError> {
        let client = reqwest::Client::new();
        let mut last_network_err: Option<String> = None;

        for attempt in 0..=self.max_retries {
            match client.get(&self.metric_url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| QueueCollectionError::Network(e.to_string()))?;
                    if !status.is_success() {
                        return Err(QueueCollectionError::HttpError {
                            status: status.as_u16(),
                            body,
                        });
                    }
                    return parse_prometheus_gauge(&body, &self.metric_name);
                }
                Err(e) => {
                    last_network_err = Some(e.to_string());
                    if attempt < self.max_retries {
                        tokio::time::sleep(Duration::from_millis(100 * (1u64 << attempt))).await;
                    }
                }
            }
        }

        Err(QueueCollectionError::Network(
            last_network_err.unwrap_or_else(|| "unknown network error".to_string()),
        ))
    }
}

/// Publish the pending-queue depth as the `stellar_node_pending_rpc_queue`
/// gauge. Available only when the `metrics` feature is enabled.
#[cfg(feature = "metrics")]
pub fn publish_pending_queue_metric(
    namespace: &str,
    name: &str,
    node_type: &str,
    network: &str,
    pending: u64,
) {
    let labels = crate::controller::metrics::NodeLabels {
        namespace: namespace.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        network: network.to_string(),
        hardware_generation: "unknown".to_string(),
    };
    crate::controller::metrics::PENDING_RPC_QUEUE
        .get_or_create(&labels)
        .set(pending as i64);
}

/// Parse a Prometheus text-format gauge line:
/// `metric_name{labels...} value` or `metric_name value`.
///
/// Returns the numeric value of the first matching line (labels are ignored —
/// the collector only queries a single node). A prefix match is used so that
/// `soroban_rpc_pending_requests` also matches lines emitted with a trailing
/// `{...}` label set, but sub-metrics like `soroban_rpc_pending_requests_total`
/// are excluded by checking the next byte is `{` or whitespace.
pub fn parse_prometheus_gauge(body: &str, metric_name: &str) -> Result<u64, QueueCollectionError> {
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(metric_name) {
            let after = rest.chars().next().unwrap_or(' ');
            if after != '{' && !after.is_whitespace() {
                continue;
            }
            // Value is the last whitespace-delimited token (after any label set).
            let value_token = line.split_whitespace().last().unwrap_or("");
            let value: f64 = value_token
                .parse()
                .map_err(|_| QueueCollectionError::ParseError(format!(
                    "cannot parse gauge value from line: {line}"
                )))?;
            if value < 0.0 {
                return Ok(0);
            }
            return Ok(value.round() as u64);
        }
    }
    Err(QueueCollectionError::ParseError(format!(
        "metric '{metric_name}' not found in response"
    )))
}

// ============================================================================
// Runner registry
// ============================================================================

static QUEUE_SCALERS: OnceLock<Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>> =
    OnceLock::new();

/// Ensure the queue autoscaler background loop is running (or stopped) for a node.
pub fn ensure_queue_autoscaler_running(client: kube::Client, node: &StellarNode) {
    let Some(autoscaling) = &node.spec.autoscaling else {
        return;
    };
    let Some(config) = &autoscaling.queue_autoscaling else {
        return;
    };
    let config = config.clone();

    let key = format!(
        "{}/{}",
        node.namespace().unwrap_or_default(),
        node.name_any()
    );
    let mut scalers = QUEUE_SCALERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();

    if !config.enabled || node.spec.node_type != NodeType::SorobanRpc {
        if let Some(tx) = scalers.remove(&key) {
            info!("Stopping queue autoscaler for {}", key);
            let _ = tx.send(true);
        }
        return;
    }

    if scalers.contains_key(&key) {
        return;
    }

    info!("Starting queue autoscaler for {}", key);
    let (tx, rx) = tokio::sync::watch::channel(false);
    scalers.insert(key.clone(), tx);

    let metric_url = config.metric_url.clone().unwrap_or_else(|| {
        format!(
            "http://{}.{}.svc.cluster.local:8000/metrics",
            node.name_any(),
            node.namespace().unwrap_or_else(|| "default".to_string())
        )
    });

    let node_type = node.spec.node_type.to_string();
    let network = node.spec.network_passphrase().to_string();
    let ns = node.namespace().unwrap_or_default();
    let name = node.name_any();
    // node_type/network are used for the Prometheus gauge (metrics feature).
    let _ = (&node_type, &network);

    let state = Arc::new(Mutex::new(QueueAutoscalingState {
        current_replicas: config.min_replicas as i32,
        ..Default::default()
    }));

    let autoscaler = QueueAutoscaler {
        config: config.clone(),
        state: state.clone(),
        k8s_client: Some(client),
        node_ref: StellarNodeRef {
            namespace: ns.clone(),
            name: name.clone(),
            uid: node.metadata.uid.clone().unwrap_or_default(),
        },
    };

    let collector = PendingQueueCollector {
        metric_url,
        metric_name: config.metric_name.clone(),
        max_retries: 3,
    };

    let mut rx_col = rx.clone();
    let col_autoscaler = autoscaler.clone_handle();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(
            config.poll_interval_seconds as u64,
        ));
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match collector.poll_once().await {
                        Ok(pending) => {
                            #[cfg(feature = "metrics")]
                            publish_pending_queue_metric(&ns, &name, &node_type, &network, pending);
                            let decision = col_autoscaler.evaluate(pending, Instant::now());
                            if let Err(e) = col_autoscaler.patch_deployment(&decision).await {
                                error!("Failed to scale deployment for {}: {}", key, e);
                            }
                        }
                        Err(e) => debug!("Queue metric poll failed for {}: {}", key, e),
                    }
                }
                _ = rx_col.changed() => {
                    if *rx_col.borrow() { break; }
                }
            }
        }
    });
    let _ = rx; // receiver ownership retained
}

// Helper so the spawned loop shares the same autoscaler handle.
impl QueueAutoscaler {
    fn clone_handle(&self) -> Self {
        Self {
            config: self.config.clone(),
            state: self.state.clone(),
            k8s_client: self.k8s_client.clone(),
            node_ref: self.node_ref.clone(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn config(min: u32, max: u32, target: u64, stabilization: u32) -> QueueAutoscalingConfig {
        QueueAutoscalingConfig {
            enabled: true,
            min_replicas: min,
            max_replicas: max,
            target_pending_per_replica: target,
            metric_name: "soroban_rpc_pending_requests".to_string(),
            metric_url: None,
            scale_up_cooldown: "0s".to_string(),
            scale_down_cooldown: "0s".to_string(),
            stabilization_window_seconds: stabilization,
            poll_interval_seconds: 1,
        }
    }

    fn autoscaler(cfg: QueueAutoscalingConfig) -> QueueAutoscaler {
        let mut a = QueueAutoscaler::new(
            cfg.clone(),
            StellarNodeRef {
                namespace: "test".to_string(),
                name: "soroban-rpc".to_string(),
                uid: "uid".to_string(),
            },
        );
        a.config = cfg;
        a
    }

    // --- Integer-math desired_replicas ---

    #[test]
    fn desired_replicas_uses_integer_math_without_float_drift() {
        // target=100: 0 pending -> 1 (min), 1..=100 -> 1, 101 -> 2, 600 -> 6
        assert_eq!(desired_replicas(0, 100, 1, 10), 1);
        assert_eq!(desired_replicas(1, 100, 1, 10), 1);
        assert_eq!(desired_replicas(100, 100, 1, 10), 1);
        assert_eq!(desired_replicas(101, 100, 1, 10), 2);
        assert_eq!(desired_replicas(600, 100, 1, 10), 6);
        // Exact boundary never overshoots (no ceil of float artifacts).
        assert_eq!(desired_replicas(10_000, 100, 1, 10), 10);
        assert_eq!(desired_replicas(10_001, 100, 1, 10), 10); // clamped to max
    }

    #[test]
    fn desired_replicas_clamps_to_min_and_max() {
        assert_eq!(desired_replicas(1, 100, 3, 10), 3); // below min
        assert_eq!(desired_replicas(50_000, 100, 1, 8), 8); // above max
        assert_eq!(desired_replicas(0, 100, 1, 1), 1); // min == max
        assert_eq!(desired_replicas(500, 0, 2, 9), 2); // zero target -> min
        assert_eq!(desired_replicas(0, 0, 0, 0), 1); // degenerate -> at least 1
    }

    // --- 500% traffic spike simulation ---

    #[tokio::test]
    async fn scales_up_within_three_seconds_on_500_percent_spike() {
        let cfg = config(1, 10, 100, 300);
        let scaler = autoscaler(cfg);
        let start = Instant::now();
        let mut now = start;

        // Baseline: 1 replica comfortably absorbs the steady-state queue.
        assert!(matches!(
            scaler.evaluate(80, now),
            ScalingDecision::Hold { .. }
        ));

        // 500% traffic spike: pending jumps from ~100 baseline to 600.
        let spike_pending = 600u64;
        let mut scaled_up = false;
        let mut simulated_elapsed = Duration::ZERO;
        for _ in 0..6 {
            // Poll every 500ms.
            now = now.checked_add(Duration::from_millis(500)).unwrap();
            simulated_elapsed += Duration::from_millis(500);
            match scaler.evaluate(spike_pending, now) {
                ScalingDecision::ScaleUp { from, to, .. } => {
                    assert_eq!(from, 1);
                    assert_eq!(to, 6);
                    scaled_up = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(scaled_up, "must scale up during the spike");
        assert!(
            simulated_elapsed <= Duration::from_secs(3),
            "scale-up took {simulated_elapsed:?}, expected <= 3s"
        );

        // The deployment would now be patched to 6 replicas.
        let decision = ScalingDecision::ScaleUp {
            from: 1,
            to: 6,
            pending: spike_pending,
        };
        let patched = scaler.patch_deployment(&decision).await;
        assert!(patched.is_ok(), "patch (test mode no-op) must succeed");
    }

    // --- Scale-down stabilization ---

    #[test]
    fn scale_down_is_blocked_during_stabilization_window() {
        let cfg = config(1, 10, 100, 300);
        let scaler = autoscaler(cfg);
        let mut now = Instant::now();

        // Scale up under load first.
        assert!(matches!(
            scaler.evaluate(600, now),
            ScalingDecision::ScaleUp { to: 6, .. }
        ));

        // Load drops back to baseline; desired returns to 1.
        now = now.checked_add(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            scaler.evaluate(80, now),
            ScalingDecision::Hold {
                reason: HoldReason::StabilizationActive
            }
        ));

        // Even 60 seconds later the window has not elapsed (300s), so hold.
        now = now.checked_add(Duration::from_secs(60)).unwrap();
        assert!(matches!(
            scaler.evaluate(80, now),
            ScalingDecision::Hold {
                reason: HoldReason::StabilizationActive
            }
        ));
    }

    #[test]
    fn scale_down_fires_after_stabilization_window_elapses() {
        let cfg = config(1, 10, 100, 300);
        let scaler = autoscaler(cfg);
        let mut now = Instant::now();

        assert!(matches!(
            scaler.evaluate(600, now),
            ScalingDecision::ScaleUp { to: 6, .. }
        ));

        // Low load persists. The scale-down may only fire once the stabilization
        // window (300s) has fully elapsed with the desired count below current.
        let mut scaled_down = false;
        for _ in 0..=70 {
            now = now.checked_add(Duration::from_secs(5)).unwrap();
            match scaler.evaluate(80, now) {
                ScalingDecision::ScaleDown { from, to, .. } => {
                    assert_eq!(from, 6);
                    assert_eq!(to, 1);
                    scaled_down = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(scaled_down, "must scale down once the window elapses");
    }

    #[test]
    fn transient_dip_inside_window_does_not_scale_down() {
        let cfg = config(1, 10, 100, 300);
        let scaler = autoscaler(cfg);
        let mut now = Instant::now();
        assert!(matches!(
            scaler.evaluate(600, now),
            ScalingDecision::ScaleUp { to: 6, .. }
        ));

        // A single low sample is not enough: still holding.
        now = now.checked_add(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            scaler.evaluate(80, now),
            ScalingDecision::Hold { .. }
        ));

        // Load surges again inside the window: desired stays high, so even after
        // 300s+ the max-in-window desired is not below current -> still hold.
        for step in 1..=61 {
            now = now.checked_add(Duration::from_secs(5)).unwrap();
            if step == 30 {
                let _ = scaler.evaluate(700, now); // burst inside the window
            } else {
                let _ = scaler.evaluate(80, now);
            }
        }
        now = now.checked_add(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            scaler.evaluate(80, now),
            ScalingDecision::Hold {
                reason: HoldReason::StabilizationActive
            }
        ));
    }

    #[test]
    fn never_scales_below_min_or_above_max() {
        let cfg = config(2, 5, 100, 60);
        let scaler = autoscaler(cfg);
        let mut now = Instant::now();

        // Max clamp.
        assert!(matches!(
            scaler.evaluate(100_000, now),
            ScalingDecision::ScaleUp { to: 5, .. }
        ));
        // At max, further load never scales past the boundary.
        assert!(matches!(
            scaler.evaluate(100_000, now),
            ScalingDecision::Hold { .. }
        ));

        // Over time, load drains and the stabilized scale-down stops at min (2).
        let mut scaled_to_min = false;
        for _ in 0..=70 {
            now = now.checked_add(Duration::from_secs(5)).unwrap();
            match scaler.evaluate(0, now) {
                ScalingDecision::ScaleDown { to, .. } => {
                    assert_eq!(to, 2);
                    scaled_to_min = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(scaled_to_min, "must scale down to min");
        // At min, no further scale-down.
        assert!(matches!(
            scaler.evaluate(0, now),
            ScalingDecision::Hold { .. }
        ));
    }

    // --- Prometheus gauge parsing ---

    #[test]
    fn parses_prometheus_gauge_line_with_labels() {
        let body = "# HELP soroban_rpc_pending_requests pending queue\n# TYPE soroban_rpc_pending_requests gauge\nsoroban_rpc_pending_requests{instance=\"soroban-rpc\"} 523\n";
        assert_eq!(parse_prometheus_gauge(body, "soroban_rpc_pending_requests").unwrap(), 523);
    }

    #[test]
    fn parses_bare_gauge_line() {
        assert_eq!(
            parse_prometheus_gauge("soroban_rpc_pending_requests 77\n", "soroban_rpc_pending_requests")
                .unwrap(),
            77
        );
    }

    #[test]
    fn does_not_match_submetrics_or_missing_metric() {
        let body = "soroban_rpc_pending_requests_total 5\nsoroban_rpc_pending_requests_seconds 2\n";
        assert!(parse_prometheus_gauge(body, "soroban_rpc_pending_requests").is_err());
    }
}
