//! Predictive Auto-Scaling for Horizon nodes
//!
//! Implements a time-series ledger volume collector and a simple forecasting
//! model (exponential smoothing / Holt-Winters) to predict the next hour's
//! load and pre-emptively adjust the HPA `minReplicas` before traffic spikes.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  Predictive Scaler Loop                     │
//! │                                                             │
//! │  1. Collect ledger volume from Prometheus / Horizon API     │
//! │  2. Store in in-memory ring buffer (time-series)            │
//! │  3. Run Holt-Winters double exponential smoothing           │
//! │  4. Forecast next-hour load                                 │
//! │  5. Map forecast → minReplicas                              │
//! │  6. Patch HPA minReplicas via Kubernetes API                │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Configuration
//!
//! ```yaml
//! spec:
//!   autoscaling:
//!     minReplicas: 2
//!     maxReplicas: 10
//!     predictiveScaling:
//!       enabled: true
//!       prometheusUrl: "http://prometheus:9090"
//!       ledgerVolumeMetric: "stellar_horizon_ledger_ingestion_rate"
//!       forecastWindowMinutes: 60
//!       scalingFactor: 1.2
//!       alpha: 0.3
//!       beta: 0.1
//! ```

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

// ============================================================================
// Configuration types (CRD-embedded)
// ============================================================================

/// Predictive auto-scaling configuration for Horizon nodes.
///
/// Uses double exponential smoothing (Holt-Winters) to forecast ledger
/// volume and pre-emptively adjust HPA `minReplicas`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PredictiveScalingConfig {
    /// Enable predictive scaling.
    #[serde(default)]
    pub enabled: bool,

    /// Prometheus base URL for scraping ledger volume metrics.
    /// Example: `"http://prometheus-operated.monitoring:9090"`
    #[serde(default = "default_prometheus_url")]
    pub prometheus_url: String,

    /// Prometheus metric name for ledger ingestion volume.
    /// Default: `"stellar_horizon_ledger_ingestion_rate"`
    #[serde(default = "default_ledger_metric")]
    pub ledger_volume_metric: String,

    /// How many minutes ahead to forecast.
    /// Default: 60
    #[serde(default = "default_forecast_window")]
    pub forecast_window_minutes: u32,

    /// Scaling factor applied to the forecast before computing minReplicas.
    /// A value of `1.2` adds a 20% safety margin.
    /// Default: 1.2
    #[serde(default = "default_scaling_factor")]
    pub scaling_factor: f64,

    /// Smoothing factor α for the level component (0 < α < 1).
    /// Higher values give more weight to recent observations.
    /// Default: 0.3
    #[serde(default = "default_alpha")]
    pub alpha: f64,

    /// Smoothing factor β for the trend component (0 < β < 1).
    /// Default: 0.1
    #[serde(default = "default_beta")]
    pub beta: f64,

    /// Ledger volume threshold per replica (transactions per second).
    /// Used to map the forecast to a replica count.
    /// Default: 1000
    #[serde(default = "default_tps_per_replica")]
    pub tps_per_replica: f64,
}

fn default_prometheus_url() -> String {
    "http://prometheus-operated.monitoring:9090".to_string()
}

fn default_ledger_metric() -> String {
    "stellar_horizon_ledger_ingestion_rate".to_string()
}

fn default_forecast_window() -> u32 {
    60
}

fn default_scaling_factor() -> f64 {
    1.2
}

fn default_alpha() -> f64 {
    0.3
}

fn default_beta() -> f64 {
    0.1
}

fn default_tps_per_replica() -> f64 {
    1000.0
}

impl Default for PredictiveScalingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prometheus_url: default_prometheus_url(),
            ledger_volume_metric: default_ledger_metric(),
            forecast_window_minutes: default_forecast_window(),
            scaling_factor: default_scaling_factor(),
            alpha: default_alpha(),
            beta: default_beta(),
            tps_per_replica: default_tps_per_replica(),
        }
    }
}

// ============================================================================
// Time-series data collector
// ============================================================================

/// A single ledger volume observation.
#[derive(Debug, Clone)]
pub struct LedgerVolumePoint {
    pub timestamp: DateTime<Utc>,
    /// Ledger ingestion rate (transactions per second)
    pub tps: f64,
}

/// In-memory ring buffer for ledger volume time-series data.
///
/// Retains the last `capacity` observations. Older points are evicted
/// automatically when the buffer is full.
pub struct LedgerVolumeCollector {
    buffer: VecDeque<LedgerVolumePoint>,
    capacity: usize,
}

impl LedgerVolumeCollector {
    /// Create a new collector with the given ring-buffer capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Record a new observation.
    pub fn record(&mut self, point: LedgerVolumePoint) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(point);
    }

    /// Return a slice of all recorded observations (oldest first).
    pub fn observations(&self) -> &VecDeque<LedgerVolumePoint> {
        &self.buffer
    }

    /// Number of recorded observations.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// True if no observations have been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

// ============================================================================
// Forecasting model: Double Exponential Smoothing (Holt-Winters, no seasonality)
// ============================================================================

/// State for the Holt-Winters double exponential smoothing model.
#[derive(Debug, Clone)]
pub struct HoltWintersState {
    /// Current level estimate
    pub level: f64,
    /// Current trend estimate
    pub trend: f64,
}

impl HoltWintersState {
    /// Initialise from the first two observations.
    pub fn init(first: f64, second: f64) -> Self {
        Self {
            level: first,
            trend: second - first,
        }
    }

    /// Update the model with a new observation.
    ///
    /// - `alpha`: smoothing factor for the level (0 < α < 1)
    /// - `beta`:  smoothing factor for the trend  (0 < β < 1)
    pub fn update(&mut self, observation: f64, alpha: f64, beta: f64) {
        let prev_level = self.level;
        self.level = alpha * observation + (1.0 - alpha) * (self.level + self.trend);
        self.trend = beta * (self.level - prev_level) + (1.0 - beta) * self.trend;
    }

    /// Forecast `h` steps ahead.
    pub fn forecast(&self, h: u32) -> f64 {
        (self.level + (h as f64) * self.trend).max(0.0)
    }
}

/// Fit a Holt-Winters model to the given observations and return the state.
///
/// Returns `None` if fewer than 2 observations are available.
pub fn fit_holt_winters(observations: &[f64], alpha: f64, beta: f64) -> Option<HoltWintersState> {
    if observations.len() < 2 {
        return None;
    }

    let mut state = HoltWintersState::init(observations[0], observations[1]);
    for &obs in &observations[2..] {
        state.update(obs, alpha, beta);
    }
    Some(state)
}

// ============================================================================
// Scaling decision
// ============================================================================

/// Compute the recommended `minReplicas` from a forecast value.
///
/// `forecast_tps`    – predicted transactions per second
/// `tps_per_replica` – capacity per replica
/// `scaling_factor`  – safety margin multiplier
/// `current_min`     – current HPA minReplicas (lower bound)
/// `max_replicas`    – HPA maxReplicas (upper bound)
pub fn compute_min_replicas(
    forecast_tps: f64,
    tps_per_replica: f64,
    scaling_factor: f64,
    current_min: i32,
    max_replicas: i32,
) -> i32 {
    if tps_per_replica <= 0.0 {
        return current_min;
    }
    let needed = ((forecast_tps * scaling_factor) / tps_per_replica).ceil() as i32;
    needed.max(current_min).min(max_replicas).max(1)
}

// ============================================================================
// Prometheus scraper (thin wrapper around reqwest)
// ============================================================================

/// Scrape the current value of a Prometheus instant query.
///
/// Returns the scalar value of the first result, or `None` if the query
/// returns no data or the request fails.
pub async fn scrape_prometheus_metric(
    prometheus_url: &str,
    metric: &str,
    label_filters: &str,
) -> Option<f64> {
    let query = if label_filters.is_empty() {
        metric.to_string()
    } else {
        format!("{metric}{{{label_filters}}}")
    };

    let url = format!(
        "{prometheus_url}/api/v1/query?query={}",
        urlencoding(&query)
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
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                vec![c]
            }
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}

// ============================================================================
// Main predictive scaler loop
// ============================================================================

/// Run the predictive scaling loop for a single Horizon node.
///
/// This function is intended to be spawned as a background task per Horizon
/// `StellarNode`. It:
///
/// 1. Scrapes ledger volume from Prometheus every `scrape_interval_secs`.
/// 2. Fits a Holt-Winters model to the collected observations.
/// 3. Forecasts the load `forecast_window_minutes` ahead.
/// 4. Patches the HPA `minReplicas` via the Kubernetes API.
///
/// The loop exits when the `shutdown` channel is closed.
pub async fn run_predictive_scaler(
    client: kube::Client,
    namespace: String,
    node_name: String,
    config: PredictiveScalingConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    if !config.enabled {
        return;
    }

    info!(
        namespace = %namespace,
        node = %node_name,
        "Predictive scaler started"
    );

    // Ring buffer: keep 2 hours of 1-minute samples = 120 points
    let mut collector = LedgerVolumeCollector::new(120);
    let scrape_interval = tokio::time::Duration::from_secs(60);
    let hpa_name = format!("{node_name}-hpa");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(scrape_interval) => {}
            _ = shutdown.changed() => {
                info!(node = %node_name, "Predictive scaler shutting down");
                return;
            }
        }

        // 1. Scrape current ledger volume
        let label_filters = format!("namespace=\"{namespace}\",node=\"{node_name}\"");
        let tps = scrape_prometheus_metric(
            &config.prometheus_url,
            &config.ledger_volume_metric,
            &label_filters,
        )
        .await;

        let tps = match tps {
            Some(v) => v,
            None => {
                warn!(node = %node_name, "Failed to scrape ledger volume metric; skipping cycle");
                continue;
            }
        };

        collector.record(LedgerVolumePoint {
            timestamp: Utc::now(),
            tps,
        });

        debug!(node = %node_name, tps, observations = collector.len(), "Recorded ledger volume");

        // Need at least 2 observations to fit the model
        if collector.len() < 2 {
            continue;
        }

        // 2. Fit Holt-Winters model
        let obs: Vec<f64> = collector.observations().iter().map(|p| p.tps).collect();
        let state = match fit_holt_winters(&obs, config.alpha, config.beta) {
            Some(s) => s,
            None => continue,
        };

        // 3. Forecast `forecast_window_minutes` steps ahead (1 step = 1 minute)
        let forecast = state.forecast(config.forecast_window_minutes);

        // 4. Fetch current HPA to get current min/max replicas
        let hpa_api: kube::Api<k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler> =
            kube::Api::namespaced(client.clone(), &namespace);

        let current_hpa = match hpa_api.get(&hpa_name).await {
            Ok(h) => h,
            Err(e) => {
                warn!(node = %node_name, hpa = %hpa_name, error = %e, "HPA not found; skipping");
                continue;
            }
        };

        let current_min = current_hpa
            .spec
            .as_ref()
            .and_then(|s| s.min_replicas)
            .unwrap_or(1);
        let max_replicas = current_hpa
            .spec
            .as_ref()
            .map(|s| s.max_replicas)
            .unwrap_or(10);

        // 5. Compute recommended minReplicas
        let new_min = compute_min_replicas(
            forecast,
            config.tps_per_replica,
            config.scaling_factor,
            current_min,
            max_replicas,
        );

        if new_min == current_min {
            debug!(node = %node_name, min_replicas = current_min, "No HPA adjustment needed");
            continue;
        }

        // 6. Patch HPA minReplicas
        info!(
            node = %node_name,
            forecast_tps = forecast,
            current_min,
            new_min,
            "Adjusting HPA minReplicas based on forecast"
        );

        let patch = serde_json::json!({
            "spec": { "minReplicas": new_min }
        });

        if let Err(e) = hpa_api
            .patch(
                &hpa_name,
                &kube::api::PatchParams::apply("stellar-predictive-scaler"),
                &kube::api::Patch::Merge(&patch),
            )
            .await
        {
            warn!(node = %node_name, error = %e, "Failed to patch HPA minReplicas");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_holt_winters_init() {
        let state = HoltWintersState::init(100.0, 110.0);
        assert_eq!(state.level, 100.0);
        assert_eq!(state.trend, 10.0);
    }

    #[test]
    fn test_holt_winters_forecast_increasing_trend() {
        let obs = vec![100.0, 110.0, 120.0, 130.0, 140.0];
        let state = fit_holt_winters(&obs, 0.3, 0.1).unwrap();
        // With a consistent +10 trend, forecast should be > current level
        let forecast = state.forecast(6);
        assert!(
            forecast > state.level,
            "forecast={forecast}, level={}",
            state.level
        );
    }

    #[test]
    fn test_holt_winters_forecast_stable() {
        let obs = vec![100.0, 100.0, 100.0, 100.0, 100.0];
        let state = fit_holt_winters(&obs, 0.3, 0.1).unwrap();
        let forecast = state.forecast(6);
        // Stable series: forecast should be close to 100
        assert!((forecast - 100.0).abs() < 20.0, "forecast={forecast}");
    }

    #[test]
    fn test_holt_winters_insufficient_data() {
        let result = fit_holt_winters(&[100.0], 0.3, 0.1);
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_min_replicas_basic() {
        // 2000 TPS forecast, 1000 TPS/replica, 1.2x safety → ceil(2.4) = 3
        let result = compute_min_replicas(2000.0, 1000.0, 1.2, 1, 10);
        assert_eq!(result, 3);
    }

    #[test]
    fn test_compute_min_replicas_respects_current_min() {
        // Forecast is low but current_min is 5 → should stay at 5
        let result = compute_min_replicas(100.0, 1000.0, 1.2, 5, 10);
        assert_eq!(result, 5);
    }

    #[test]
    fn test_compute_min_replicas_respects_max() {
        // Forecast is very high but max is 10 → capped at 10
        let result = compute_min_replicas(100_000.0, 1000.0, 1.2, 1, 10);
        assert_eq!(result, 10);
    }

    #[test]
    fn test_compute_min_replicas_zero_tps_per_replica() {
        // Division by zero guard: returns current_min
        let result = compute_min_replicas(2000.0, 0.0, 1.2, 3, 10);
        assert_eq!(result, 3);
    }

    #[test]
    fn test_ledger_volume_collector_ring_buffer() {
        let mut collector = LedgerVolumeCollector::new(3);
        for i in 0..5u64 {
            collector.record(LedgerVolumePoint {
                timestamp: Utc::now(),
                tps: i as f64 * 100.0,
            });
        }
        // Only last 3 should remain
        assert_eq!(collector.len(), 3);
        let obs: Vec<f64> = collector.observations().iter().map(|p| p.tps).collect();
        assert_eq!(obs, vec![200.0, 300.0, 400.0]);
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("foo=bar"), "foo%3Dbar");
        assert_eq!(urlencoding("simple"), "simple");
    }
}
