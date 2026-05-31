//! Gas-consumption-driven autoscaling for Soroban RPC nodes.
//!
//! This module implements the core types and logic for collecting per-ledger
//! gas usage from the Soroban RPC API, computing an EWMA trend score, and
//! driving Kubernetes HPA scaling decisions.
//!
//! # Architecture
//!
//! Three components share state via `Arc<Mutex<GasAutoscalingState>>`:
//! - [`GasCollector`] — polls the Soroban RPC `getTransactions` endpoint
//! - [`GasTrendCalculator`] — computes EWMA over the ring buffer
//! - [`GasAutoscaler`] — evaluates thresholds and patches the HPA

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, error, info};

use crate::crd::GasAutoscalingConfig;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use kube::api::{Api, Patch, PatchParams};
use kube::ResourceExt;
use std::collections::HashMap;
use std::sync::OnceLock;

// ============================================================================
// Core data types
// ============================================================================

/// A single gas-usage observation for one ledger.
#[derive(Debug, Clone)]
pub struct LedgerGasSample {
    pub ledger_sequence: u64,
    pub gas_used: u64,
    pub collected_at: DateTime<Utc>,
}

/// Shared mutable state for the gas autoscaling pipeline.
///
/// Protected by `Arc<Mutex<...>>` and shared between `GasCollector`,
/// `GasTrendCalculator`, and `GasAutoscaler`.
pub struct GasAutoscalingState {
    pub ring_buffer: VecDeque<LedgerGasSample>,
    pub window_size: usize,
    pub current_score: Option<f64>,
    pub current_replicas: i32,
    pub last_scale_up_at: Option<Instant>,
    pub last_scale_down_at: Option<Instant>,
}

/// Prometheus label set for gas autoscaling metrics.
#[derive(Debug, Clone)]
pub struct GasAutoscalingLabels {
    pub namespace: String,
    pub node_name: String,
}

/// Reference to a `StellarNode` Kubernetes resource.
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
#[derive(Debug, Clone, PartialEq)]
pub enum ScaleDirection {
    Up,
    Down,
}

/// Reason why the autoscaler chose not to scale.
#[derive(Debug, Clone, PartialEq)]
pub enum HoldReason {
    CooldownActive { direction: ScaleDirection },
    WithinThresholds,
    AtBoundary,
}

/// The outcome of one autoscaler evaluation cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalingDecision {
    ScaleUp { from: i32, to: i32, score: f64 },
    ScaleDown { from: i32, to: i32, score: f64 },
    Hold { reason: HoldReason },
}

// ============================================================================
// Error types
// ============================================================================

/// Errors that can occur while collecting gas data from the Soroban RPC API.
#[derive(Debug)]
pub enum GasCollectionError {
    /// Network or connection-level failure.
    Network(String),
    /// HTTP 4xx/5xx response from the RPC endpoint.
    HttpError { status: u16, body: String },
    /// JSON parse failure.
    ParseError(String),
}

impl std::fmt::Display for GasCollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GasCollectionError::Network(msg) => write!(f, "network error: {msg}"),
            GasCollectionError::HttpError { status, body } => {
                write!(f, "HTTP {status}: {body}")
            }
            GasCollectionError::ParseError(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

impl std::error::Error for GasCollectionError {}

// ============================================================================
// Soroban RPC API response types
// ============================================================================

#[derive(Deserialize)]
struct GetTransactionsResponse {
    result: GetTransactionsResult,
}

#[derive(Deserialize)]
struct GetTransactionsResult {
    transactions: Vec<TransactionEntry>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TransactionEntry {
    fee_charged: Option<u64>,
    ledger: Option<u64>,
}

// ============================================================================
// Component stubs (methods added in subsequent tasks)
// ============================================================================

/// Polls the Soroban RPC `getTransactions` endpoint and pushes gas samples
/// into the shared ring buffer.
///
/// Methods are implemented in task 2.2 and 2.3.
pub struct GasCollector {
    pub rpc_url: String,
    pub poll_interval: Duration,
    pub max_retries: u32,
    pub state: Arc<Mutex<GasAutoscalingState>>,
    pub metrics_labels: GasAutoscalingLabels,
}

impl GasCollector {
    /// Poll the Soroban RPC `getTransactions` endpoint once, update the ring
    /// buffer on success, and return the new sample (or `None` if the
    /// transaction list was empty).
    ///
    /// Retry policy: on network/connection errors, retry up to `self.max_retries`
    /// times with exponential backoff `100ms * 2^attempt`. HTTP 4xx/5xx errors
    /// are not retried. Parse errors leave the ring buffer unchanged.
    pub async fn poll_once(&self) -> Result<Option<LedgerGasSample>, GasCollectionError> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransactions",
            "params": {
                "startLedger": 0,
                "pagination": { "limit": 1 }
            }
        });

        let mut last_network_err: Option<String> = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_millis(100 * (1u64 << (attempt - 1)));
                tokio::time::sleep(backoff).await;
            }

            let response = match client.post(&self.rpc_url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    last_network_err = Some(e.to_string());
                    continue; // retry
                }
            };

            let status = response.status();
            if status.is_client_error() || status.is_server_error() {
                let status_code = status.as_u16();
                let body_text = response.text().await.unwrap_or_default();
                return Err(GasCollectionError::HttpError {
                    status: status_code,
                    body: body_text,
                });
            }

            // Successful HTTP response — parse JSON.
            let text = response
                .text()
                .await
                .map_err(|e| GasCollectionError::ParseError(e.to_string()))?;

            let parsed: GetTransactionsResponse = serde_json::from_str(&text).map_err(|e| {
                GasCollectionError::ParseError(format!("{e}: {}", &text[..text.len().min(1024)]))
            })?;

            let transactions = parsed.result.transactions;
            if transactions.is_empty() {
                return Ok(None);
            }

            let gas_used: u64 = transactions
                .iter()
                .map(|t| t.fee_charged.unwrap_or(0))
                .sum();

            let ledger_sequence = transactions.first().and_then(|t| t.ledger).unwrap_or(0);

            let sample = LedgerGasSample {
                ledger_sequence,
                gas_used,
                collected_at: chrono::Utc::now(),
            };

            // Push into ring buffer, evicting oldest entry if at capacity.
            {
                let mut state = self.state.lock().unwrap();
                if state.ring_buffer.len() >= state.window_size {
                    state.ring_buffer.pop_front();
                }
                state.ring_buffer.push_back(sample.clone());
            }

            return Ok(Some(sample));
        }

        Err(GasCollectionError::Network(
            last_network_err.unwrap_or_else(|| "unknown network error".to_string()),
        ))
    }

    /// Run the gas collection loop, ticking every `self.poll_interval`.
    ///
    /// The loop calls [`Self::poll_once`] on each tick and logs the result.
    /// It exits cleanly when `shutdown` receives `true`.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.poll_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match self.poll_once().await {
                        Ok(Some(sample)) => {
                            debug!(
                                "collected gas sample: ledger={}, gas_used={}",
                                sample.ledger_sequence, sample.gas_used
                            );
                        }
                        Ok(None) => {
                            debug!("no transactions in latest ledger, skipping");
                        }
                        Err(GasCollectionError::Network(msg)) => {
                            error!("gas collection network error: {msg}");
                        }
                        Err(GasCollectionError::HttpError { status, body: _ }) => {
                            error!("gas collection HTTP error: status={status}");
                        }
                        Err(GasCollectionError::ParseError(msg)) => {
                            error!("gas collection parse error: {msg}");
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }
}

/// Computes the EWMA gas trend score from the ring buffer.
///
/// Methods are implemented in task 3.1.
pub struct GasTrendCalculator {
    pub alpha: f64,
}

impl GasTrendCalculator {
    /// Compute the exponentially weighted moving average (EWMA) over a slice
    /// of gas samples.
    ///
    /// Returns `None` if the slice is empty.
    ///
    /// Formula:
    /// ```text
    /// score_0 = samples[0] as f64
    /// score_i = alpha * samples[i] + (1 - alpha) * score_{i-1}
    /// ```
    pub fn compute_ewma(samples: &[u64], alpha: f64) -> Option<f64> {
        if samples.is_empty() {
            return None;
        }

        let mut score = samples[0] as f64;
        for sample in samples.iter().skip(1) {
            score = alpha * *sample as f64 + (1.0 - alpha) * score;
        }

        Some(score)
    }

    /// Compute the fill level of the ring buffer: current_len / window_size.
    ///
    /// Returns a value in [0.0, 1.0].
    pub fn fill_level(current_len: usize, window_size: usize) -> f64 {
        if window_size == 0 {
            return 0.0;
        }

        let ratio = current_len as f64 / window_size as f64;
        ratio.clamp(0.0, 1.0)
    }
}

/// Evaluates the current trend score against configured thresholds and
/// patches the Kubernetes HPA resource accordingly.
///
/// Methods are implemented in task 4.1.
pub struct GasAutoscaler {
    pub config: GasAutoscalingConfig,
    pub state: Arc<Mutex<GasAutoscalingState>>,
    pub k8s_client: kube::Client,
    pub node_ref: StellarNodeRef,
}

impl GasAutoscaler {
    /// Evaluate the current gas trend score and return a scaling decision.
    ///
    /// This method:
    /// 1. Reads the current state (score, replicas, last scale timestamps)
    /// 2. Checks if score is available
    /// 3. Parses cooldown durations
    /// 4. Evaluates scale-up conditions (threshold, cooldown, max replicas)
    /// 5. Evaluates scale-down conditions (threshold, cooldown, min replicas)
    /// 6. Returns appropriate ScalingDecision
    ///
    /// This method does NOT patch the HPA — that's done in task 4.2.
    pub async fn evaluate_and_scale(
        &self,
    ) -> Result<ScalingDecision, Box<dyn std::error::Error + Send + Sync>> {
        // 1. Lock state and read current values
        let (current_score, current_replicas, last_scale_up_at, last_scale_down_at) = {
            let state = self.state.lock().unwrap();
            (
                state.current_score,
                state.current_replicas,
                state.last_scale_up_at,
                state.last_scale_down_at,
            )
        };

        // 2. If no score available, hold
        let score = match current_score {
            Some(s) => s,
            None => {
                return Ok(ScalingDecision::Hold {
                    reason: HoldReason::WithinThresholds,
                });
            }
        };

        // 3. Parse cooldown durations
        let scale_up_cooldown = parse_duration(&self.config.scale_up_cooldown)?;
        let scale_down_cooldown = parse_duration(&self.config.scale_down_cooldown)?;

        let now = Instant::now();

        // 4. Check scale-up conditions
        if score > self.config.scale_up_threshold {
            // Check if we're at max replicas
            if current_replicas >= self.config.max_replicas as i32 {
                return Ok(ScalingDecision::Hold {
                    reason: HoldReason::AtBoundary,
                });
            }

            // Check cooldown
            if let Some(last_up) = last_scale_up_at {
                if now.duration_since(last_up) < scale_up_cooldown {
                    return Ok(ScalingDecision::Hold {
                        reason: HoldReason::CooldownActive {
                            direction: ScaleDirection::Up,
                        },
                    });
                }
            }

            // Scale up!
            let new_replicas = (current_replicas + self.config.scale_up_step as i32)
                .min(self.config.max_replicas as i32);

            // Update state
            {
                let mut state = self.state.lock().unwrap();
                state.current_replicas = new_replicas;
                state.last_scale_up_at = Some(now);
            }

            return Ok(ScalingDecision::ScaleUp {
                from: current_replicas,
                to: new_replicas,
                score,
            });
        }

        // 5. Check scale-down conditions
        if score < self.config.scale_down_threshold {
            // Check if we're at min replicas
            if current_replicas <= self.config.min_replicas as i32 {
                return Ok(ScalingDecision::Hold {
                    reason: HoldReason::AtBoundary,
                });
            }

            // Check cooldown
            if let Some(last_down) = last_scale_down_at {
                if now.duration_since(last_down) < scale_down_cooldown {
                    return Ok(ScalingDecision::Hold {
                        reason: HoldReason::CooldownActive {
                            direction: ScaleDirection::Down,
                        },
                    });
                }
            }

            // Scale down!
            let new_replicas = (current_replicas - self.config.scale_down_step as i32)
                .max(self.config.min_replicas as i32);

            // Update state
            {
                let mut state = self.state.lock().unwrap();
                state.current_replicas = new_replicas;
                state.last_scale_down_at = Some(now);
            }

            return Ok(ScalingDecision::ScaleDown {
                from: current_replicas,
                to: new_replicas,
                score,
            });
        }

        // 6. Within thresholds
        Ok(ScalingDecision::Hold {
            reason: HoldReason::WithinThresholds,
        })
    }

    /// Patch the Kubernetes HPA resource to enforce the new `minReplicas`.
    pub async fn patch_hpa(
        &self,
        decision: &ScalingDecision,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (from, to, score, reason_str) = match decision {
            ScalingDecision::ScaleUp { from, to, score } => {
                (*from, *to, *score, "Gas usage above scale-up threshold")
            }
            ScalingDecision::ScaleDown { from, to, score } => {
                (*from, *to, *score, "Gas usage below scale-down threshold")
            }
            ScalingDecision::Hold { reason } => {
                debug!("Holding scaling: {:?}", reason);
                return Ok(());
            }
        };

        debug!(
            "Patching HPA {}/{} from {} to {} (score: {:.2})",
            self.node_ref.namespace, self.node_ref.name, from, to, score
        );

        let hpa_api: Api<HorizontalPodAutoscaler> =
            Api::namespaced(self.k8s_client.clone(), &self.node_ref.namespace);

        // We patch `minReplicas` to force the scaling up based on gas trend,
        // without preventing K8s from scaling it further up via CPU/Memory if needed.
        let patch = serde_json::json!({
            "spec": {
                "minReplicas": to
            }
        });

        hpa_api
            .patch(
                &self.node_ref.name,
                &PatchParams::apply("stellar-operator-gas-autoscaler").force(),
                &Patch::Merge(&patch),
            )
            .await?;

        info!(
            "Successfully patched HPA {}/{} to minReplicas: {} (was {}). Reason: {}",
            self.node_ref.namespace, self.node_ref.name, to, from, reason_str
        );

        Ok(())
    }

    /// Run the gas autoscaling loop, evaluating metrics and scaling every tick.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        // Run evaluation every 15 seconds
        let mut interval = tokio::time::interval(Duration::from_secs(15));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Update current_score using GasTrendCalculator
                    {
                        let mut state = self.state.lock().unwrap();
                        let samples: Vec<u64> = state.ring_buffer.iter().map(|s| s.gas_used).collect();
                        // Use a fixed alpha of 0.2 for EWMA smoothing (could be configurable)
                        state.current_score = GasTrendCalculator::compute_ewma(&samples, 0.2);
                    }

                    let decision_opt = match self.evaluate_and_scale().await {
                        Ok(decision) => Some(decision),
                        Err(e) => {
                            error!("Error evaluating gas autoscaling for {}: {}", self.node_ref.name, e);
                            None
                        }
                    };

                    if let Some(decision) = decision_opt {
                        if let Err(e) = self.patch_hpa(&decision).await {
                            error!("Failed to patch HPA for {}: {}", self.node_ref.name, e);
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Shutting down GasAutoscaler loop for {}", self.node_ref.name);
                        break;
                    }
                }
            }
        }
    }
}

/// Parse a duration string like "60s", "5m", "2h".
///
/// Supports:
/// - "Xs" for seconds
/// - "Xm" for minutes
/// - "Xh" for hours
///
/// Returns an error if the format is invalid.
pub fn parse_duration(s: &str) -> Result<Duration, Box<dyn std::error::Error + Send + Sync>> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Duration string is empty".into());
    }

    let (num_str, unit) = if let Some(stripped) = s.strip_suffix('s') {
        (stripped, 's')
    } else if let Some(stripped) = s.strip_suffix('m') {
        (stripped, 'm')
    } else if let Some(stripped) = s.strip_suffix('h') {
        (stripped, 'h')
    } else {
        return Err(format!("invalid duration format: {}", s).into());
    };

    let num: u64 = num_str.parse()?;

    let duration = match unit {
        's' => Duration::from_secs(num),
        'm' => Duration::from_secs(num * 60),
        'h' => Duration::from_secs(num * 3600),
        _ => unreachable!(),
    };

    Ok(duration)
}

// Global registry for gas autoscaler background loops
static GAS_SCALERS: OnceLock<Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>> =
    OnceLock::new();

/// Ensures that the gas autoscaler background loop is running (or stopped) for a given node.
pub fn ensure_gas_autoscaler_running(
    client: kube::Client,
    node: &crate::crd::StellarNode,
    config: &GasAutoscalingConfig,
) {
    let key = format!(
        "{}/{}",
        node.namespace().unwrap_or_default(),
        node.name_any()
    );
    let mut scalers = GAS_SCALERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();

    if !config.enabled {
        if let Some(tx) = scalers.remove(&key) {
            info!("Stopping gas autoscaler for {}", key);
            let _ = tx.send(true);
        }
        return;
    }

    if scalers.contains_key(&key) {
        // Already running, updating config dynamically is out of scope for now
        // (to update config, we'd signal shutdown and restart, but we'll stick to basic start/stop)
        return;
    }

    info!("Starting gas autoscaler for {}", key);
    let (tx, rx) = tokio::sync::watch::channel(false);
    scalers.insert(key.clone(), tx);

    let state = Arc::new(Mutex::new(GasAutoscalingState {
        ring_buffer: VecDeque::new(),
        window_size: 60,
        current_score: None,
        current_replicas: config.min_replicas as i32,
        last_scale_up_at: None,
        last_scale_down_at: None,
    }));

    // Start gas collector loop
    let collector = GasCollector {
        // Build the local cluster DNS URL for the Soroban RPC service
        rpc_url: format!(
            "http://{}.{}.svc.cluster.local:8000",
            node.name_any(),
            node.namespace().unwrap_or_else(|| "default".to_string())
        ),
        poll_interval: Duration::from_secs(2),
        max_retries: 3,
        state: state.clone(),
        metrics_labels: GasAutoscalingLabels {
            namespace: node.namespace().unwrap_or_default(),
            node_name: node.name_any(),
        },
    };

    let rx_col = rx.clone();
    tokio::spawn(async move {
        collector.run(rx_col).await;
    });

    // Start autoscaler loop
    let scaler = GasAutoscaler {
        config: config.clone(),
        state,
        k8s_client: client,
        node_ref: StellarNodeRef {
            namespace: node.namespace().unwrap_or_default(),
            name: node.name_any(),
            uid: node.metadata.uid.clone().unwrap_or_default(),
        },
    };

    tokio::spawn(async move {
        scaler.run(rx).await;
    });
}
