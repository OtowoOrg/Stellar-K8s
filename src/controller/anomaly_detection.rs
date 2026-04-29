use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::controller::audit_log::{AdminAction, AuditEntry};
use crate::controller::background_jobs::{JobKind, JobRegistry};
use crate::controller::ControllerState;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyDetectionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_detection_interval_seconds")]
    pub interval_seconds: u64,
    #[serde(default = "default_detection_window_seconds")]
    pub window_seconds: u64,
    #[serde(default = "default_zscore_threshold")]
    pub zscore_threshold: f64,
    #[serde(default = "default_ewma_alpha")]
    pub ewma_alpha: f64,
    #[serde(default = "default_max_anomalies")]
    pub max_anomalies: usize,
}

fn default_true() -> bool {
    true
}

fn default_detection_interval_seconds() -> u64 {
    30
}

fn default_detection_window_seconds() -> u64 {
    300
}

fn default_zscore_threshold() -> f64 {
    3.0
}

fn default_ewma_alpha() -> f64 {
    0.25
}

fn default_max_anomalies() -> usize {
    500
}

impl Default for AnomalyDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            interval_seconds: default_detection_interval_seconds(),
            window_seconds: default_detection_window_seconds(),
            zscore_threshold: default_zscore_threshold(),
            ewma_alpha: default_ewma_alpha(),
            max_anomalies: default_max_anomalies(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyEvent {
    pub id: String,
    pub timestamp: String,
    pub actor: String,
    pub action: AdminAction,
    pub window_count: u64,
    pub baseline_mean: f64,
    pub baseline_stddev: f64,
    pub zscore: f64,
    pub message: String,
}

#[derive(Debug, Default)]
struct EwmaState {
    mean: f64,
    variance: f64,
    initialized: bool,
}

impl EwmaState {
    fn update(&mut self, value: f64, alpha: f64) {
        if !self.initialized {
            self.mean = value;
            self.variance = 0.0;
            self.initialized = true;
            return;
        }
        let prev_mean = self.mean;
        self.mean = alpha * value + (1.0 - alpha) * self.mean;
        let delta = value - prev_mean;
        self.variance = alpha * delta * delta + (1.0 - alpha) * self.variance;
    }

    fn stddev(&self) -> f64 {
        self.variance.max(1e-6).sqrt()
    }
}

#[derive(Default)]
struct DetectorState {
    ewma: HashMap<String, EwmaState>,
    seen_ids: HashSet<String>,
    seen_queue: VecDeque<String>,
}


use crate::controller::ml_pipeline::{AnomalyModel, EwmaModel, extract_features};

#[derive(Clone)]
pub struct AnomalyDetector {
    config: AnomalyDetectionConfig,
    state: Arc<RwLock<DetectorState>>,
    anomalies: Arc<RwLock<VecDeque<AnomalyEvent>>>,
    model: Arc<dyn AnomalyModel>,
}

impl AnomalyDetector {
    pub fn new(config: AnomalyDetectionConfig) -> Self {
        let model = Arc::new(EwmaModel::new(config.ewma_alpha, config.zscore_threshold));
        Self {
            config,
            state: Arc::new(RwLock::new(DetectorState::default())),
            anomalies: Arc::new(RwLock::new(VecDeque::new())),
            model,
        }
    }

    pub async fn list(&self, limit: usize) -> Vec<AnomalyEvent> {
        let anomalies = self.anomalies.read().await;
        let iter = anomalies.iter().rev();
        if limit == 0 {
            iter.cloned().collect()
        } else {
            iter.take(limit).cloned().collect()
        }
    }

    async fn record_anomaly(&self, event: AnomalyEvent) {
        let mut anomalies = self.anomalies.write().await;
        if anomalies.len() >= self.config.max_anomalies {
            anomalies.pop_front();
        }
        anomalies.push_back(event);
    }

    async fn mark_seen(&self, ids: impl IntoIterator<Item = String>) {
        let mut state = self.state.write().await;
        for id in ids {
            if state.seen_ids.insert(id.clone()) {
                state.seen_queue.push_back(id);
            }
        }
        while state.seen_queue.len() > self.config.max_anomalies * 10 {
            if let Some(old) = state.seen_queue.pop_front() {
                state.seen_ids.remove(&old);
            }
        }
    }

    async fn is_seen(&self, id: &str) -> bool {
        let state = self.state.read().await;
        state.seen_ids.contains(id)
    }

    async fn update_model(
        &self,
        key: &str,
        count: f64,
        timestamp: DateTime<Utc>,
    ) -> Option<AnomalyEvent> {
        let mut state = self.state.write().await;
        let ewma = state.ewma.entry(key.to_string()).or_default();
        ewma.update(count, self.config.ewma_alpha);

        let stddev = ewma.stddev();
        let zscore = (count - ewma.mean) / stddev;

        if zscore >= self.config.zscore_threshold {
            let parts: Vec<&str> = key.splitn(2, '|').collect();
            let actor = parts.get(0).copied().unwrap_or("unknown").to_string();
            let action = parts
                .get(1)
                .and_then(|s| serde_json::from_str::<AdminAction>(s).ok())
                .unwrap_or(AdminAction::Other("unknown".to_string()));

            let event = AnomalyEvent {
                id: format!("anomaly-{}", timestamp.timestamp_millis()),
                timestamp: timestamp.to_rfc3339(),
                actor,
                action,
                window_count: count as u64,
                baseline_mean: ewma.mean,
                baseline_stddev: stddev,
                zscore,
                message: format!(
                    "Spike in admin actions: count={}, zscore={:.2}",
                    count, zscore
                ),
            };

            return Some(event);
        }

        None
    }
}

pub async fn run_anomaly_detection(
    state: Arc<ControllerState>,
    detector: Arc<AnomalyDetector>,
) {
    if !detector.config.enabled {
        return;
    }

    let registry: Arc<JobRegistry> = state.job_registry.clone();
    loop {
        let handle = registry.register("anomaly-detector", JobKind::Other("anomaly_detector".to_string()), None);
        handle.start();

        if let Err(e) = detect_once(&state, &detector).await {
            handle.fail(e);
        } else {
            handle.succeed();
        }

        sleep(Duration::from_secs(detector.config.interval_seconds)).await;
    }
}

async fn detect_once(state: &ControllerState, detector: &AnomalyDetector) -> Result<(), String> {
    let now = Utc::now();
    let window_start = now - chrono::Duration::seconds(detector.config.window_seconds as i64);

    let entries = state.audit_log.list(None, None, None, 0);
    
    // Traditional Z-Score based detection
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut new_ids = Vec::new();
    let mut window_entries = Vec::new();

    for entry in entries.iter() {
        if entry.timestamp < window_start {
            continue;
        }
        window_entries.push(entry.clone());
        if detector.is_seen(&entry.id).await {
            continue;
        }
        let key = format!("{}|{}", entry.actor, serde_json::to_string(&entry.action).unwrap_or_default());
        *counts.entry(key).or_insert(0) += 1;
        new_ids.push(entry.id.clone());
    }

    detector.mark_seen(new_ids).await;

    for (key, count) in counts {
        if let Some(event) = detector
            .update_model(&key, count as f64, now)
            .await
        {
            info!(
                actor = %event.actor,
                action = %event.action,
                zscore = %event.zscore,
                "Anomalous operator activity detected (Z-Score)"
            );
            detector.record_anomaly(event.clone()).await;
            perform_remediation(state, &event).await;
        }
    }

    // Advanced ML-based detection
    if !window_entries.is_empty() {
        let features = extract_features(&window_entries);
        let prediction = detector.model.predict(&features);

        if prediction.is_anomaly {
            let event = AnomalyEvent {
                id: format!("anomaly-ml-{}", now.timestamp_millis()),
                timestamp: now.to_rfc3339(),
                actor: "aggregate".to_string(),
                action: AdminAction::Other("ml_detected".to_string()),
                window_count: window_entries.len() as u64,
                baseline_mean: 0.0,
                baseline_stddev: 0.0,
                zscore: prediction.confidence,
                message: prediction.explanation.clone(),
            };

            info!(explanation = %prediction.explanation, "ML-based anomaly detected");
            detector.record_anomaly(event.clone()).await;
            perform_remediation(state, &event).await;
        }
    }

    Ok(())
}

async fn perform_remediation(state: &ControllerState, event: &AnomalyEvent) {
    warn!(actor = %event.actor, message = %event.message, "Performing automated remediation");
    
    // In a real implementation, this would:
    // 1. Check a RemediationPolicy CRD
    // 2. Trigger actions like suspending a node, revoking a token, or alerting via PagerDuty
    
    match event.action {
        AdminAction::NodeUpdate | AdminAction::NodeDelete => {
            // Potentially suspicious mass updates/deletes
            info!("Remediation: throttling administrative actions for {}", event.actor);
        }
        _ => {}
    }
}
