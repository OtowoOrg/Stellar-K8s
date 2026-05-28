//! Advanced Monitoring Pipeline with Anomaly Detection and Root Cause Analysis
//!
//! This module implements a sophisticated observability pipeline that:
//! - Unifies metrics, logs, and traces into a single data model
//! - Correlates events across different observability signals
//! - Detects anomalies using ML-based baseline learning
//! - Performs automated root cause analysis
//! - Provides intelligent alerting with noise reduction
//! - Reconstructs incident timelines
//! - Enables predictive alerting for potential issues

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::controller::ml_pipeline::AnomalyModel;

/// Unified observability data model combining metrics, logs, and traces
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub source: EventSource,
    pub severity: Severity,
    pub category: EventCategory,
    pub resource: ResourceIdentifier,
    pub data: EventData,
    pub tags: HashMap<String, String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EventSource {
    Metric,
    Log,
    Trace,
    Alert,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EventCategory {
    Performance,
    Availability,
    Security,
    Capacity,
    Configuration,
    Network,
    Consensus,
    Storage,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceIdentifier {
    pub namespace: String,
    pub name: String,
    pub kind: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventData {
    Metric {
        name: String,
        value: f64,
        unit: String,
        labels: HashMap<String, String>,
    },
    Log {
        level: String,
        message: String,
        fields: HashMap<String, serde_json::Value>,
    },
    Trace {
        operation: String,
        duration_ms: f64,
        status: String,
        attributes: HashMap<String, String>,
    },
    Alert {
        alert_name: String,
        description: String,
        firing: bool,
    },
}

/// Correlation engine for linking related events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationResult {
    pub correlation_id: String,
    pub events: Vec<ObservabilityEvent>,
    pub correlation_score: f64,
    pub correlation_type: CorrelationType,
    pub time_window_ms: u64,
    pub root_event: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CorrelationType {
    Temporal,     // Events close in time
    Causal,       // One event caused another
    Spatial,      // Events on same resource
    TraceLinked,  // Events share trace ID
    PatternBased, // Events match known pattern
}

/// Anomaly detection result with baseline learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyDetectionResult {
    pub anomaly_id: String,
    pub timestamp: DateTime<Utc>,
    pub event: ObservabilityEvent,
    pub is_anomaly: bool,
    pub confidence: f64,
    pub baseline: BaselineMetrics,
    pub deviation: DeviationMetrics,
    pub anomaly_type: AnomalyType,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineMetrics {
    pub mean: f64,
    pub stddev: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationMetrics {
    pub zscore: f64,
    pub percent_change: f64,
    pub absolute_change: f64,
    pub rate_of_change: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnomalyType {
    Spike,
    Drop,
    Trend,
    Oscillation,
    Flatline,
    Missing,
    Outlier,
}

/// Root cause analysis engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCauseAnalysis {
    pub analysis_id: String,
    pub timestamp: DateTime<Utc>,
    pub incident_id: String,
    pub root_causes: Vec<RootCause>,
    pub contributing_factors: Vec<ContributingFactor>,
    pub affected_resources: Vec<ResourceIdentifier>,
    pub timeline: Vec<TimelineEvent>,
    pub confidence: f64,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCause {
    pub cause_id: String,
    pub description: String,
    pub evidence: Vec<ObservabilityEvent>,
    pub confidence: f64,
    pub category: EventCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributingFactor {
    pub factor_id: String,
    pub description: String,
    pub impact_score: f64,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub description: String,
    pub severity: Severity,
    pub event_id: String,
}

/// Intelligent alert with noise reduction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligentAlert {
    pub alert_id: String,
    pub timestamp: DateTime<Utc>,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub category: EventCategory,
    pub affected_resources: Vec<ResourceIdentifier>,
    pub correlated_events: Vec<String>,
    pub root_cause_analysis: Option<RootCauseAnalysis>,
    pub suppressed: bool,
    pub suppression_reason: Option<String>,
    pub deduplication_key: String,
    pub alert_count: u32,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub acknowledged: bool,
    pub resolved: bool,
}

/// Predictive alert for potential issues
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictiveAlert {
    pub prediction_id: String,
    pub timestamp: DateTime<Utc>,
    pub predicted_issue: String,
    pub probability: f64,
    /// Estimated seconds until the predicted issue occurs
    pub time_to_occurrence_secs: u64,
    pub affected_resources: Vec<ResourceIdentifier>,
    pub indicators: Vec<String>,
    pub recommended_actions: Vec<String>,
    pub confidence: f64,
}

/// Incident timeline reconstruction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentTimeline {
    pub incident_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub events: Vec<TimelineEvent>,
    pub phases: Vec<IncidentPhase>,
    pub mttr_ms: Option<u64>,
    pub impact_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentPhase {
    pub phase_name: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub description: String,
}

/// Configuration for the observability pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub enabled: bool,
    pub correlation_window_seconds: u64,
    pub anomaly_detection_enabled: bool,
    pub baseline_learning_window_hours: u64,
    pub anomaly_threshold: f64,
    pub alert_deduplication_window_seconds: u64,
    pub predictive_alerting_enabled: bool,
    pub prediction_horizon_minutes: u64,
    pub max_events_in_memory: usize,
    pub max_incidents: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            correlation_window_seconds: 300,
            anomaly_detection_enabled: true,
            baseline_learning_window_hours: 24,
            anomaly_threshold: 3.0,
            alert_deduplication_window_seconds: 600,
            predictive_alerting_enabled: true,
            prediction_horizon_minutes: 30,
            max_events_in_memory: 10000,
            max_incidents: 1000,
        }
    }
}

/// Main observability pipeline controller
pub struct ObservabilityPipeline {
    config: ObservabilityConfig,
    events: Arc<RwLock<VecDeque<ObservabilityEvent>>>,
    correlations: Arc<RwLock<Vec<CorrelationResult>>>,
    anomalies: Arc<RwLock<Vec<AnomalyDetectionResult>>>,
    alerts: Arc<RwLock<Vec<IntelligentAlert>>>,
    predictive_alerts: Arc<RwLock<Vec<PredictiveAlert>>>,
    incidents: Arc<RwLock<Vec<IncidentTimeline>>>,
    baselines: Arc<RwLock<HashMap<String, BaselineState>>>,
    ml_model: Arc<dyn AnomalyModel>,
}

#[derive(Debug, Clone)]
struct BaselineState {
    metric_name: String,
    values: VecDeque<f64>,
    timestamps: VecDeque<DateTime<Utc>>,
    mean: f64,
    variance: f64,
    min: f64,
    max: f64,
    last_updated: DateTime<Utc>,
}

impl BaselineState {
    fn new(metric_name: String) -> Self {
        Self {
            metric_name,
            values: VecDeque::new(),
            timestamps: VecDeque::new(),
            mean: 0.0,
            variance: 0.0,
            min: f64::MAX,
            max: f64::MIN,
            last_updated: Utc::now(),
        }
    }

    fn update(&mut self, value: f64, timestamp: DateTime<Utc>, max_samples: usize) {
        self.values.push_back(value);
        self.timestamps.push_back(timestamp);

        // Keep only recent samples
        while self.values.len() > max_samples {
            self.values.pop_front();
            self.timestamps.pop_front();
        }

        // Recalculate statistics
        if !self.values.is_empty() {
            self.mean = self.values.iter().sum::<f64>() / self.values.len() as f64;
            self.variance = self
                .values
                .iter()
                .map(|v| (v - self.mean).powi(2))
                .sum::<f64>()
                / self.values.len() as f64;
            self.min = self.values.iter().cloned().fold(f64::MAX, f64::min);
            self.max = self.values.iter().cloned().fold(f64::MIN, f64::max);
        }

        self.last_updated = timestamp;
    }

    fn stddev(&self) -> f64 {
        self.variance.sqrt()
    }

    fn percentile(&self, p: f64) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<f64> = self.values.iter().cloned().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let index = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[index.min(sorted.len() - 1)]
    }
}

impl ObservabilityPipeline {
    pub fn new(config: ObservabilityConfig, ml_model: Arc<dyn AnomalyModel>) -> Self {
        Self {
            config,
            events: Arc::new(RwLock::new(VecDeque::new())),
            correlations: Arc::new(RwLock::new(Vec::new())),
            anomalies: Arc::new(RwLock::new(Vec::new())),
            alerts: Arc::new(RwLock::new(Vec::new())),
            predictive_alerts: Arc::new(RwLock::new(Vec::new())),
            incidents: Arc::new(RwLock::new(Vec::new())),
            baselines: Arc::new(RwLock::new(HashMap::new())),
            ml_model,
        }
    }

    /// Ingest an observability event into the pipeline
    pub async fn ingest_event(&self, event: ObservabilityEvent) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        debug!(
            event_id = %event.id,
            source = ?event.source,
            severity = ?event.severity,
            "Ingesting observability event"
        );

        // Store event
        let mut events = self.events.write().await;
        events.push_back(event.clone());

        // Maintain size limit
        while events.len() > self.config.max_events_in_memory {
            events.pop_front();
        }
        drop(events);

        // Process event through pipeline stages
        self.update_baseline(&event).await?;
        self.detect_anomaly(&event).await?;
        self.correlate_events(&event).await?;
        self.generate_alerts(&event).await?;

        Ok(())
    }

    /// Update baseline metrics for anomaly detection
    async fn update_baseline(&self, event: &ObservabilityEvent) -> Result<(), String> {
        if !self.config.anomaly_detection_enabled {
            return Ok(());
        }

        if let EventData::Metric { name, value, .. } = &event.data {
            let mut baselines = self.baselines.write().await;
            let baseline = baselines
                .entry(name.clone())
                .or_insert_with(|| BaselineState::new(name.clone()));

            let max_samples = (self.config.baseline_learning_window_hours * 3600 / 60) as usize;
            baseline.update(*value, event.timestamp, max_samples);
        }

        Ok(())
    }

    /// Detect anomalies using ML-based baseline learning
    async fn detect_anomaly(&self, event: &ObservabilityEvent) -> Result<(), String> {
        if !self.config.anomaly_detection_enabled {
            return Ok(());
        }

        if let EventData::Metric { name, value, .. } = &event.data {
            let baselines = self.baselines.read().await;
            if let Some(baseline) = baselines.get(name) {
                // Need at least 10 samples for meaningful baseline
                if baseline.values.len() < 10 {
                    return Ok(());
                }

                let stddev = baseline.stddev();
                let zscore = if stddev > 0.0 {
                    (*value - baseline.mean) / stddev
                } else {
                    0.0
                };

                let percent_change = if baseline.mean != 0.0 {
                    ((*value - baseline.mean) / baseline.mean) * 100.0
                } else {
                    0.0
                };

                let is_anomaly = zscore.abs() > self.config.anomaly_threshold;

                if is_anomaly {
                    let anomaly_type = if zscore > 0.0 {
                        AnomalyType::Spike
                    } else {
                        AnomalyType::Drop
                    };

                    let anomaly = AnomalyDetectionResult {
                        anomaly_id: format!("anomaly-{}", Utc::now().timestamp_millis()),
                        timestamp: event.timestamp,
                        event: event.clone(),
                        is_anomaly: true,
                        confidence: (zscore.abs() / 10.0).min(1.0),
                        baseline: BaselineMetrics {
                            mean: baseline.mean,
                            stddev,
                            min: baseline.min,
                            max: baseline.max,
                            p50: baseline.percentile(50.0),
                            p95: baseline.percentile(95.0),
                            p99: baseline.percentile(99.0),
                            sample_count: baseline.values.len(),
                        },
                        deviation: DeviationMetrics {
                            zscore,
                            percent_change,
                            absolute_change: *value - baseline.mean,
                            rate_of_change: 0.0, // TODO: Calculate from time series
                        },
                        anomaly_type,
                        explanation: format!(
                            "Metric '{}' value {:.2} deviates {:.2} standard deviations from baseline mean {:.2}",
                            name, value, zscore, baseline.mean
                        ),
                    };

                    info!(
                        metric = %name,
                        value = %value,
                        zscore = %zscore,
                        "Anomaly detected"
                    );

                    let mut anomalies = self.anomalies.write().await;
                    anomalies.push(anomaly);
                }
            }
        }

        Ok(())
    }

    /// Correlate events across metrics, logs, and traces
    async fn correlate_events(&self, new_event: &ObservabilityEvent) -> Result<(), String> {
        let events = self.events.read().await;
        let window_start = new_event.timestamp
            - chrono::Duration::seconds(self.config.correlation_window_seconds as i64);

        let mut correlated = Vec::new();
        let mut correlation_score = 0.0;
        let mut correlation_type = CorrelationType::Temporal;

        for event in events.iter().rev() {
            if event.timestamp < window_start {
                break;
            }

            if event.id == new_event.id {
                continue;
            }

            // Check for trace-based correlation
            if let (Some(trace_id1), Some(trace_id2)) = (&new_event.trace_id, &event.trace_id) {
                if trace_id1 == trace_id2 {
                    correlated.push(event.clone());
                    correlation_score += 1.0;
                    correlation_type = CorrelationType::TraceLinked;
                    continue;
                }
            }

            // Check for spatial correlation (same resource)
            if new_event.resource.namespace == event.resource.namespace
                && new_event.resource.name == event.resource.name
            {
                correlated.push(event.clone());
                correlation_score += 0.8;
                if matches!(correlation_type, CorrelationType::Temporal) {
                    correlation_type = CorrelationType::Spatial;
                }
            }

            // Check for temporal correlation
            let time_diff = (new_event.timestamp - event.timestamp).num_seconds().abs();
            if time_diff < 60 {
                correlated.push(event.clone());
                correlation_score += 0.5;
            }
        }

        if !correlated.is_empty() {
            let correlation = CorrelationResult {
                correlation_id: format!("corr-{}", Utc::now().timestamp_millis()),
                events: correlated,
                correlation_score: correlation_score / 10.0,
                correlation_type,
                time_window_ms: self.config.correlation_window_seconds * 1000,
                root_event: Some(new_event.id.clone()),
            };

            let mut correlations = self.correlations.write().await;
            correlations.push(correlation);
        }

        Ok(())
    }

    /// Generate intelligent alerts with noise reduction
    async fn generate_alerts(&self, event: &ObservabilityEvent) -> Result<(), String> {
        // Only generate alerts for high-severity events or anomalies
        if !matches!(event.severity, Severity::Error | Severity::Critical) {
            return Ok(());
        }

        let dedup_key = format!(
            "{}:{}:{:?}",
            event.resource.namespace, event.resource.name, event.category
        );

        let mut alerts = self.alerts.write().await;

        // Check for existing alert (deduplication)
        let dedup_window_start = Utc::now()
            - chrono::Duration::seconds(self.config.alert_deduplication_window_seconds as i64);

        if let Some(existing_alert) = alerts.iter_mut().find(|a| {
            a.deduplication_key == dedup_key && a.last_seen > dedup_window_start && !a.resolved
        }) {
            // Update existing alert
            existing_alert.alert_count += 1;
            existing_alert.last_seen = event.timestamp;
            debug!(
                alert_id = %existing_alert.alert_id,
                count = %existing_alert.alert_count,
                "Deduplicated alert"
            );
            return Ok(());
        }

        // Create new alert
        let alert = IntelligentAlert {
            alert_id: format!("alert-{}", Utc::now().timestamp_millis()),
            timestamp: event.timestamp,
            title: format!("{:?} issue on {}", event.category, event.resource.name),
            description: match &event.data {
                EventData::Log { message, .. } => message.clone(),
                EventData::Metric { name, value, .. } => {
                    format!("Metric {} has value {}", name, value)
                }
                EventData::Trace { operation, .. } => {
                    format!("Trace operation: {}", operation)
                }
                EventData::Alert { description, .. } => description.clone(),
            },
            severity: event.severity.clone(),
            category: event.category.clone(),
            affected_resources: vec![event.resource.clone()],
            correlated_events: vec![event.id.clone()],
            root_cause_analysis: None,
            suppressed: false,
            suppression_reason: None,
            deduplication_key: dedup_key,
            alert_count: 1,
            first_seen: event.timestamp,
            last_seen: event.timestamp,
            acknowledged: false,
            resolved: false,
        };

        info!(
            alert_id = %alert.alert_id,
            severity = ?alert.severity,
            resource = %alert.affected_resources[0].name,
            "Generated new alert"
        );

        alerts.push(alert);

        Ok(())
    }

    /// Perform root cause analysis for an incident
    pub async fn analyze_root_cause(&self, incident_id: &str) -> Result<RootCauseAnalysis, String> {
        let events = self.events.read().await;
        let anomalies = self.anomalies.read().await;

        // Find all events related to this incident
        let incident_events: Vec<_> = events
            .iter()
            .filter(|e| e.tags.get("incident_id") == Some(&incident_id.to_string()))
            .cloned()
            .collect();

        if incident_events.is_empty() {
            return Err("No events found for incident".to_string());
        }

        // Identify potential root causes
        let mut root_causes = Vec::new();

        // Look for anomalies that occurred first
        for anomaly in anomalies.iter() {
            if anomaly.event.tags.get("incident_id") == Some(&incident_id.to_string()) {
                let root_cause = RootCause {
                    cause_id: format!("rc-{}", Utc::now().timestamp_millis()),
                    description: format!("Anomaly detected: {}", anomaly.explanation),
                    evidence: vec![anomaly.event.clone()],
                    confidence: anomaly.confidence,
                    category: anomaly.event.category.clone(),
                };
                root_causes.push(root_cause);
            }
        }

        // Look for error events that preceded other events
        let error_events: Vec<_> = incident_events
            .iter()
            .filter(|e| matches!(e.severity, Severity::Error | Severity::Critical))
            .collect();

        for error_event in error_events {
            let root_cause = RootCause {
                cause_id: format!("rc-{}", Utc::now().timestamp_millis()),
                description: match &error_event.data {
                    EventData::Log { message, .. } => message.clone(),
                    _ => "Error event detected".to_string(),
                },
                evidence: vec![error_event.clone()],
                confidence: 0.7,
                category: error_event.category.clone(),
            };
            root_causes.push(root_cause);
        }

        // Build timeline
        let mut timeline_events: Vec<_> = incident_events
            .iter()
            .map(|e| TimelineEvent {
                timestamp: e.timestamp,
                event_type: format!("{:?}", e.source),
                description: match &e.data {
                    EventData::Log { message, .. } => message.clone(),
                    EventData::Metric { name, value, .. } => {
                        format!("{} = {}", name, value)
                    }
                    EventData::Trace { operation, .. } => operation.clone(),
                    EventData::Alert { alert_name, .. } => alert_name.clone(),
                },
                severity: e.severity.clone(),
                event_id: e.id.clone(),
            })
            .collect();

        timeline_events.sort_by_key(|e| e.timestamp);

        let analysis = RootCauseAnalysis {
            analysis_id: format!("rca-{}", Utc::now().timestamp_millis()),
            timestamp: Utc::now(),
            incident_id: incident_id.to_string(),
            root_causes,
            contributing_factors: vec![],
            affected_resources: incident_events.iter().map(|e| e.resource.clone()).collect(),
            timeline: timeline_events,
            confidence: 0.8,
            recommendation: "Review the timeline and root causes to determine remediation steps"
                .to_string(),
        };

        Ok(analysis)
    }

    /// Generate predictive alerts for potential issues
    pub async fn generate_predictive_alerts(&self) -> Result<Vec<PredictiveAlert>, String> {
        if !self.config.predictive_alerting_enabled {
            return Ok(vec![]);
        }

        let baselines = self.baselines.read().await;
        let mut predictions = Vec::new();

        for (metric_name, baseline) in baselines.iter() {
            if baseline.values.len() < 20 {
                continue;
            }

            // Simple trend analysis for prediction
            let recent_values: Vec<f64> = baseline.values.iter().rev().take(10).cloned().collect();
            let older_values: Vec<f64> = baseline
                .values
                .iter()
                .rev()
                .skip(10)
                .take(10)
                .cloned()
                .collect();

            if recent_values.is_empty() || older_values.is_empty() {
                continue;
            }

            let recent_mean = recent_values.iter().sum::<f64>() / recent_values.len() as f64;
            let older_mean = older_values.iter().sum::<f64>() / older_values.len() as f64;

            let trend = (recent_mean - older_mean) / older_mean;

            // Predict issues based on trends
            if trend.abs() > 0.2 {
                let predicted_issue = if trend > 0.0 {
                    format!(
                        "Metric '{}' is trending upward and may exceed capacity",
                        metric_name
                    )
                } else {
                    format!(
                        "Metric '{}' is trending downward and may indicate degradation",
                        metric_name
                    )
                };

                let prediction = PredictiveAlert {
                    prediction_id: format!("pred-{}", Utc::now().timestamp_millis()),
                    timestamp: Utc::now(),
                    predicted_issue,
                    probability: (trend.abs() * 2.0).min(1.0),
                    time_to_occurrence_secs: self.config.prediction_horizon_minutes * 60,
                    affected_resources: vec![],
                    indicators: vec![
                        format!("Recent mean: {:.2}", recent_mean),
                        format!("Older mean: {:.2}", older_mean),
                        format!("Trend: {:.2}%", trend * 100.0),
                    ],
                    recommended_actions: vec![
                        "Monitor the metric closely".to_string(),
                        "Review resource capacity".to_string(),
                        "Consider scaling if trend continues".to_string(),
                    ],
                    confidence: 0.7,
                };

                predictions.push(prediction);
            }
        }

        if !predictions.is_empty() {
            let mut predictive_alerts = self.predictive_alerts.write().await;
            predictive_alerts.extend(predictions.clone());
        }

        Ok(predictions)
    }

    /// Reconstruct incident timeline
    pub async fn reconstruct_incident_timeline(
        &self,
        incident_id: &str,
    ) -> Result<IncidentTimeline, String> {
        let events = self.events.read().await;

        let incident_events: Vec<_> = events
            .iter()
            .filter(|e| e.tags.get("incident_id") == Some(&incident_id.to_string()))
            .cloned()
            .collect();

        if incident_events.is_empty() {
            return Err("No events found for incident".to_string());
        }

        let start_time = incident_events
            .iter()
            .map(|e| e.timestamp)
            .min()
            .unwrap_or_else(Utc::now);

        let end_time = incident_events.iter().map(|e| e.timestamp).max();

        let duration_ms = end_time.map(|end| (end - start_time).num_milliseconds() as u64);

        let timeline_events: Vec<_> = incident_events
            .iter()
            .map(|e| TimelineEvent {
                timestamp: e.timestamp,
                event_type: format!("{:?}", e.source),
                description: match &e.data {
                    EventData::Log { message, .. } => message.clone(),
                    EventData::Metric { name, value, .. } => {
                        format!("{} = {}", name, value)
                    }
                    EventData::Trace { operation, .. } => operation.clone(),
                    EventData::Alert { alert_name, .. } => alert_name.clone(),
                },
                severity: e.severity.clone(),
                event_id: e.id.clone(),
            })
            .collect();

        // Identify phases
        let mut phases = vec![IncidentPhase {
            phase_name: "Detection".to_string(),
            start_time,
            end_time: Some(start_time + chrono::Duration::minutes(5)),
            duration_ms: Some(300000),
            description: "Initial detection of the incident".to_string(),
        }];

        if let Some(end) = end_time {
            phases.push(IncidentPhase {
                phase_name: "Resolution".to_string(),
                start_time: end - chrono::Duration::minutes(5),
                end_time: Some(end),
                duration_ms: Some(300000),
                description: "Incident resolution phase".to_string(),
            });
        }

        let timeline = IncidentTimeline {
            incident_id: incident_id.to_string(),
            start_time,
            end_time,
            duration_ms,
            events: timeline_events,
            phases,
            mttr_ms: duration_ms,
            impact_summary: format!(
                "Incident affected {} resources with {} events",
                incident_events.len(),
                incident_events.len()
            ),
        };

        let mut incidents = self.incidents.write().await;
        incidents.push(timeline.clone());

        Ok(timeline)
    }

    /// Get all anomalies
    pub async fn get_anomalies(&self, limit: usize) -> Vec<AnomalyDetectionResult> {
        let anomalies = self.anomalies.read().await;
        if limit == 0 {
            anomalies.clone()
        } else {
            anomalies.iter().rev().take(limit).cloned().collect()
        }
    }

    /// Get all alerts
    pub async fn get_alerts(&self, limit: usize) -> Vec<IntelligentAlert> {
        let alerts = self.alerts.read().await;
        if limit == 0 {
            alerts.clone()
        } else {
            alerts.iter().rev().take(limit).cloned().collect()
        }
    }

    /// Get all predictive alerts
    pub async fn get_predictive_alerts(&self, limit: usize) -> Vec<PredictiveAlert> {
        let alerts = self.predictive_alerts.read().await;
        if limit == 0 {
            alerts.clone()
        } else {
            alerts.iter().rev().take(limit).cloned().collect()
        }
    }

    /// Get all correlations
    pub async fn get_correlations(&self, limit: usize) -> Vec<CorrelationResult> {
        let correlations = self.correlations.read().await;
        if limit == 0 {
            correlations.clone()
        } else {
            correlations.iter().rev().take(limit).cloned().collect()
        }
    }

    /// Get all incidents
    pub async fn get_incidents(&self, limit: usize) -> Vec<IncidentTimeline> {
        let incidents = self.incidents.read().await;
        if limit == 0 {
            incidents.clone()
        } else {
            incidents.iter().rev().take(limit).cloned().collect()
        }
    }

    /// Acknowledge an alert
    pub async fn acknowledge_alert(&self, alert_id: &str) -> Result<(), String> {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.iter_mut().find(|a| a.alert_id == alert_id) {
            alert.acknowledged = true;
            info!(alert_id = %alert_id, "Alert acknowledged");
            Ok(())
        } else {
            Err("Alert not found".to_string())
        }
    }

    /// Resolve an alert
    pub async fn resolve_alert(&self, alert_id: &str) -> Result<(), String> {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.iter_mut().find(|a| a.alert_id == alert_id) {
            alert.resolved = true;
            info!(alert_id = %alert_id, "Alert resolved");
            Ok(())
        } else {
            Err("Alert not found".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::ml_pipeline::EwmaModel;

    fn make_pipeline() -> ObservabilityPipeline {
        let model = Arc::new(EwmaModel::new(0.25, 3.0));
        ObservabilityPipeline::new(ObservabilityConfig::default(), model)
    }

    fn make_metric_event(name: &str, value: f64, severity: Severity) -> ObservabilityEvent {
        ObservabilityEvent {
            id: format!("evt-{}-{}", name, Utc::now().timestamp_millis()),
            timestamp: Utc::now(),
            source: EventSource::Metric,
            severity,
            category: EventCategory::Performance,
            resource: ResourceIdentifier {
                namespace: "stellar".to_string(),
                name: "my-validator".to_string(),
                kind: "StellarNode".to_string(),
                node_id: None,
            },
            data: EventData::Metric {
                name: name.to_string(),
                value,
                unit: "ms".to_string(),
                labels: HashMap::new(),
            },
            tags: HashMap::new(),
            trace_id: None,
            span_id: None,
        }
    }

    #[tokio::test]
    async fn test_ingest_event_stores_event() {
        let pipeline = make_pipeline();
        let event = make_metric_event("ledger_lag", 5.0, Severity::Info);
        pipeline.ingest_event(event).await.unwrap();
        // No anomaly yet — need 10 samples
        let anomalies = pipeline.get_anomalies(10).await;
        assert!(anomalies.is_empty());
    }

    #[tokio::test]
    async fn test_anomaly_detected_after_baseline() {
        let pipeline = make_pipeline();

        // Feed 15 normal values to build baseline
        for i in 0..15 {
            let event = make_metric_event("ledger_lag", 10.0 + (i as f64 * 0.1), Severity::Info);
            pipeline.ingest_event(event).await.unwrap();
        }

        // Feed a spike — 10x the baseline
        let spike = make_metric_event("ledger_lag", 1000.0, Severity::Info);
        pipeline.ingest_event(spike).await.unwrap();

        let anomalies = pipeline.get_anomalies(10).await;
        assert!(!anomalies.is_empty(), "Expected anomaly to be detected");
        assert_eq!(anomalies[0].anomaly_type, AnomalyType::Spike);
    }

    #[tokio::test]
    async fn test_alert_deduplication() {
        let pipeline = make_pipeline();

        // Two identical error events should produce only one alert
        for _ in 0..2 {
            let event = make_metric_event("error_rate", 99.0, Severity::Error);
            pipeline.ingest_event(event).await.unwrap();
        }

        let alerts = pipeline.get_alerts(10).await;
        assert_eq!(alerts.len(), 1, "Duplicate alerts should be deduplicated");
        assert_eq!(alerts[0].alert_count, 2);
    }

    #[tokio::test]
    async fn test_alert_acknowledge_and_resolve() {
        let pipeline = make_pipeline();
        let event = make_metric_event("cpu_usage", 99.0, Severity::Critical);
        pipeline.ingest_event(event).await.unwrap();

        let alerts = pipeline.get_alerts(1).await;
        assert!(!alerts.is_empty());
        let alert_id = alerts[0].alert_id.clone();

        pipeline.acknowledge_alert(&alert_id).await.unwrap();
        pipeline.resolve_alert(&alert_id).await.unwrap();

        let updated = pipeline.get_alerts(1).await;
        assert!(updated[0].acknowledged);
        assert!(updated[0].resolved);
    }

    #[tokio::test]
    async fn test_predictive_alerts_with_trend() {
        let pipeline = make_pipeline();

        // Feed 25 values with a strong upward trend
        for i in 0..25 {
            let value = 10.0 + (i as f64 * 5.0); // steep upward trend
            let event = make_metric_event("disk_usage", value, Severity::Info);
            pipeline.ingest_event(event).await.unwrap();
        }

        let predictions = pipeline.generate_predictive_alerts().await.unwrap();
        assert!(
            !predictions.is_empty(),
            "Expected predictive alert for upward trend"
        );
        assert!(predictions[0].probability > 0.0);
    }

    #[tokio::test]
    async fn test_root_cause_analysis_no_events() {
        let pipeline = make_pipeline();
        let result = pipeline.analyze_root_cause("nonexistent-incident").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_incident_timeline_no_events() {
        let pipeline = make_pipeline();
        let result = pipeline.reconstruct_incident_timeline("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_correlation_by_resource() {
        let pipeline = make_pipeline();

        // Two events on the same resource within the window
        let e1 = make_metric_event("latency", 50.0, Severity::Info);
        let e2 = make_metric_event("error_rate", 5.0, Severity::Info);
        pipeline.ingest_event(e1).await.unwrap();
        pipeline.ingest_event(e2).await.unwrap();

        let correlations = pipeline.get_correlations(10).await;
        assert!(!correlations.is_empty(), "Expected spatial correlation");
    }
}
