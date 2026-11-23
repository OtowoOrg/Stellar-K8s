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
//! Intelligent incident correlation across Prometheus, uptime checks, and log-based detectors.
//!
//! Correlates firing alerts on a normalized entity key (`service`, `cluster`, `zone`)
//! rather than raw label sets, deduplicating multi-symptom storms into single incidents
//! with a unified timeline and an explicit correlation explanation payload.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use super::manager::{Incident, IncidentManager, IncidentSeverity, TimelineEvent};

/// Source stream that emitted an alert.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AlertStreamSource {
    Prometheus,
    UptimeCheck,
    LogDetector,
}

impl std::fmt::Display for AlertStreamSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertStreamSource::Prometheus => write!(f, "prometheus"),
            AlertStreamSource::UptimeCheck => write!(f, "uptime_check"),
            AlertStreamSource::LogDetector => write!(f, "log_detector"),
        }
    }
}

/// Normalized entity key used to correlate alerts across heterogeneous streams.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NormalizedEntityKey {
    pub service: String,
    pub cluster: String,
    pub zone: String,
}

impl NormalizedEntityKey {
    /// Normalize raw label sets from Prometheus, uptime checks, or log detectors
    /// into a canonical `(service, cluster, zone)` key.
    pub fn from_labels(labels: &HashMap<String, String>) -> Self {
        let pick = |keys: &[&str], default: &str| -> String {
            for k in keys {
                if let Some(v) = labels.get(*k) {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_lowercase();
                    }
                }
            }
            default.to_string()
        };

        Self {
            service: pick(
                &[
                    "service",
                    "app",
                    "app.kubernetes.io/name",
                    "stellar_node",
                    "job",
                    "target",
                ],
                "unknown-service",
            ),
            cluster: pick(
                &["cluster", "k8s_cluster", "cluster_name", "environment"],
                "default-cluster",
            ),
            zone: pick(
                &[
                    "zone",
                    "availability_zone",
                    "topology.kubernetes.io/zone",
                    "region",
                ],
                "global",
            ),
        }
    }

    pub fn canonical_id(&self) -> String {
        format!("{}/{}/{}", self.cluster, self.zone, self.service)
    }
}

/// Unified alert representation across all supported alert streams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedAlert {
    pub id: String,
    pub source: AlertStreamSource,
    pub name: String,
    pub severity: IncidentSeverity,
    pub summary: String,
    pub labels: HashMap<String, String>,
    pub fired_at: DateTime<Utc>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub is_root_cause_signal: bool,
}

impl UnifiedAlert {
    /// Compute a stable deduplication fingerprint for the alert.
    pub fn fingerprint(&self, key: &NormalizedEntityKey) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.source.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(self.name.to_lowercase().as_bytes());
        hasher.update(b":");
        hasher.update(key.canonical_id().as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Reason why an alert was correlated or created a new incident.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CorrelationReason {
    NewIncidentOpened,
    DuplicateFingerprintSuppressed,
    MatchedNormalizedEntityKey,
    RootCauseCascadeGrouped,
}

/// Explanation payload attached to every incident decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationDecision {
    pub alert_id: String,
    pub incident_id: String,
    pub source: AlertStreamSource,
    pub matched_key: NormalizedEntityKey,
    pub reason: CorrelationReason,
    pub explanation: String,
    pub suppressed_page: bool,
    pub root_cause_candidate: Option<String>,
    pub confidence: f64,
    pub decided_at: DateTime<Utc>,
}

/// Correlated incident aggregating multi-stream alerts and a unified timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelatedIncident {
    pub incident: Incident,
    pub entity_key: NormalizedEntityKey,
    pub alerts: Vec<UnifiedAlert>,
    pub decisions: Vec<CorrelationDecision>,
    pub sources: Vec<AlertStreamSource>,
    pub root_cause_alert: Option<String>,
    pub pages_suppressed: u64,
    pub first_alert_at: DateTime<Utc>,
    pub last_alert_at: DateTime<Utc>,
}

/// Aggregate correlation telemetry for evaluating page reduction and latency.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CorrelationMetrics {
    pub total_alerts_ingested: u64,
    pub deduplicated_alerts: u64,
    pub incidents_created: u64,
    pub pages_suppressed: u64,
}

impl CorrelationMetrics {
    /// Percentage reduction in pages compared to 1-page-per-alert baseline.
    pub fn page_reduction_percent(&self) -> f64 {
        if self.total_alerts_ingested == 0 {
            return 0.0;
        }
        (self.pages_suppressed as f64 / self.total_alerts_ingested as f64) * 100.0
    }
}

/// Case for replay-based precision/recall evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayIncidentCase {
    pub case_id: String,
    pub expected_root_cause: String,
    pub alerts: Vec<UnifiedAlert>,
}

/// Evaluation report over a historical replay set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationReplayReport {
    pub total_cases: usize,
    pub total_alerts: u64,
    pub total_incidents: u64,
    pub page_reduction_percent: f64,
    pub missed_root_causes: Vec<String>,
    pub root_cause_recall: f64,
}

/// Stateful correlation engine deduplicating and grouping alerts across streams.
pub struct IncidentCorrelator {
    manager: Arc<IncidentManager>,
    correlation_window: Duration,
    incidents: RwLock<HashMap<String, CorrelatedIncident>>,
    active_by_key: RwLock<HashMap<String, String>>,
    seen_fingerprints: RwLock<HashMap<String, (String, DateTime<Utc>)>>,
    metrics: RwLock<CorrelationMetrics>,
}

impl IncidentCorrelator {
    pub fn new(manager: Arc<IncidentManager>, correlation_window: Duration) -> Self {
        Self {
            manager,
            correlation_window,
            incidents: RwLock::new(HashMap::new()),
            active_by_key: RwLock::new(HashMap::new()),
            seen_fingerprints: RwLock::new(HashMap::new()),
            metrics: RwLock::new(CorrelationMetrics::default()),
        }
    }

    /// Ingest an alert from Prometheus, uptime checks, or log detectors.
    pub async fn ingest(&self, alert: UnifiedAlert) -> CorrelationDecision {
        let entity_key = NormalizedEntityKey::from_labels(&alert.labels);
        let canonical = entity_key.canonical_id();
        let fingerprint = alert.fingerprint(&entity_key);

        {
            let mut m = self.metrics.write().await;
            m.total_alerts_ingested += 1;
        }

        // 1. Check exact fingerprint deduplication within the correlation window.
        if let Some((existing_id, last_seen)) =
            self.seen_fingerprints.read().await.get(&fingerprint).cloned()
        {
            if alert.fired_at - last_seen <= self.correlation_window {
                let mut incidents = self.incidents.write().await;
                if let Some(corr) = incidents.get_mut(&existing_id) {
                    corr.pages_suppressed += 1;
                    corr.last_alert_at = corr.last_alert_at.max(alert.fired_at);
                    let decision = CorrelationDecision {
                        alert_id: alert.id.clone(),
                        incident_id: existing_id.clone(),
                        source: alert.source,
                        matched_key: entity_key,
                        reason: CorrelationReason::DuplicateFingerprintSuppressed,
                        explanation: format!(
                            "Suppressed duplicate {} alert '{}' matching fingerprint within {}s window",
                            alert.source,
                            alert.name,
                            self.correlation_window.num_seconds()
                        ),
                        suppressed_page: true,
                        root_cause_candidate: corr.root_cause_alert.clone(),
                        confidence: 0.99,
                        decided_at: Utc::now(),
                    };
                    corr.decisions.push(decision.clone());
                    corr.incident.timeline.push(TimelineEvent {
                        timestamp: alert.fired_at,
                        actor: format!("correlator:{}", alert.source),
                        action: format!("Deduplicated alert '{}'", alert.name),
                        note: Some(decision.explanation.clone()),
                    });
                    let mut m = self.metrics.write().await;
                    m.deduplicated_alerts += 1;
                    m.pages_suppressed += 1;
                    return decision;
                }
            }
        }

        // 2. Check active incident for the same normalized entity key within window.
        let maybe_existing = self.active_by_key.read().await.get(&canonical).cloned();
        if let Some(existing_id) = maybe_existing {
            let mut incidents = self.incidents.write().await;
            if let Some(corr) = incidents.get_mut(&existing_id) {
                if alert.fired_at - corr.last_alert_at <= self.correlation_window {
                    corr.pages_suppressed += 1;
                    corr.last_alert_at = corr.last_alert_at.max(alert.fired_at);
                    if !corr.sources.contains(&alert.source) {
                        corr.sources.push(alert.source);
                    }
                    if alert.severity < corr.incident.severity {
                        corr.incident.severity = alert.severity;
                    }
                    if alert.is_root_cause_signal && corr.root_cause_alert.is_none() {
                        corr.root_cause_alert = Some(alert.name.clone());
                    }

                    let reason = if alert.is_root_cause_signal {
                        CorrelationReason::RootCauseCascadeGrouped
                    } else {
                        CorrelationReason::MatchedNormalizedEntityKey
                    };

                    let explanation = format!(
                        "Correlated {} alert '{}' into {} via normalized key ({}) within {}s window",
                        alert.source,
                        alert.name,
                        existing_id,
                        canonical,
                        self.correlation_window.num_seconds()
                    );

                    let decision = CorrelationDecision {
                        alert_id: alert.id.clone(),
                        incident_id: existing_id.clone(),
                        source: alert.source,
                        matched_key: entity_key,
                        reason,
                        explanation: explanation.clone(),
                        suppressed_page: true,
                        root_cause_candidate: corr.root_cause_alert.clone(),
                        confidence: 0.92,
                        decided_at: Utc::now(),
                    };

                    corr.incident.timeline.push(TimelineEvent {
                        timestamp: alert.fired_at,
                        actor: format!("correlator:{}", alert.source),
                        action: format!("Correlated {} alert '{}'", alert.source, alert.name),
                        note: Some(explanation),
                    });
                    corr.incident.labels.insert(
                        "correlation_decision".to_string(),
                        serde_json::to_string(&decision).unwrap_or_default(),
                    );
                    corr.alerts.push(alert);
                    corr.decisions.push(decision.clone());

                    self.seen_fingerprints
                        .write()
                        .await
                        .insert(fingerprint, (existing_id, Utc::now()));
                    self.metrics.write().await.pages_suppressed += 1;
                    return decision;
                }
            }
        }

        // 3. Open a new correlated incident.
        let mut affected = Vec::new();
        if let Some(node) = alert.labels.get("node").or_else(|| alert.labels.get("pod")) {
            affected.push(node.clone());
        } else {
            affected.push(entity_key.service.clone());
        }

        let mut incident = self
            .manager
            .create_incident(
                format!("[CORRELATED] {} ({})", alert.name, canonical),
                format!(
                    "Correlated incident for entity '{}' triggered by {} alert '{}': {}",
                    canonical, alert.source, alert.name, alert.summary
                ),
                alert.severity,
                affected,
            )
            .await;

        let root_cause = if alert.is_root_cause_signal {
            Some(alert.name.clone())
        } else {
            None
        };

        let decision = CorrelationDecision {
            alert_id: alert.id.clone(),
            incident_id: incident.id.clone(),
            source: alert.source,
            matched_key: entity_key.clone(),
            reason: CorrelationReason::NewIncidentOpened,
            explanation: format!(
                "Opened new incident {} for normalized key ({}) from {} alert '{}'",
                incident.id, canonical, alert.source, alert.name
            ),
            suppressed_page: false,
            root_cause_candidate: root_cause.clone(),
            confidence: 0.90,
            decided_at: Utc::now(),
        };

        incident.labels.insert(
            "normalized_entity_key".to_string(),
            canonical.clone(),
        );
        incident.labels.insert(
            "correlation_decision".to_string(),
            serde_json::to_string(&decision).unwrap_or_default(),
        );

        let incident_id = incident.id.clone();
        let fired_at = alert.fired_at;
        let source = alert.source;

        let correlated = CorrelatedIncident {
            incident,
            entity_key,
            alerts: vec![alert],
            decisions: vec![decision.clone()],
            sources: vec![source],
            root_cause_alert: root_cause,
            pages_suppressed: 0,
            first_alert_at: fired_at,
            last_alert_at: fired_at,
        };

        self.incidents
            .write()
            .await
            .insert(incident_id.clone(), correlated);
        self.active_by_key
            .write()
            .await
            .insert(canonical, incident_id.clone());
        self.seen_fingerprints
            .write()
            .await
            .insert(fingerprint, (incident_id.clone(), fired_at));
        self.metrics.write().await.incidents_created += 1;

        info!(incident_id = %incident_id, "Opened correlated incident");
        decision
    }

    pub async fn get_incident(&self, id: &str) -> Option<CorrelatedIncident> {
        self.incidents.read().await.get(id).cloned()
    }

    pub async fn metrics(&self) -> CorrelationMetrics {
        self.metrics.read().await.clone()
    }

    /// Replay a labeled set of multi-alert cases and compute correlation metrics.
    pub async fn replay_cases(&self, cases: Vec<ReplayIncidentCase>) -> CorrelationReplayReport {
        let mut missed = Vec::new();
        let mut total_alerts = 0u64;
        let mut total_incidents = 0u64;
        let mut pages_suppressed = 0u64;

        for case in &cases {
            let case_correlator =
                IncidentCorrelator::new(Arc::new(IncidentManager::new()), self.correlation_window);
            let mut matched_root_cause = false;
            for alert in &case.alerts {
                let decision = case_correlator.ingest(alert.clone()).await;
                if alert.name == case.expected_root_cause
                    || decision.root_cause_candidate.as_deref() == Some(&case.expected_root_cause)
                {
                    matched_root_cause = true;
                }
            }
            if !matched_root_cause {
                missed.push(case.case_id.clone());
            }
            let m = case_correlator.metrics().await;
            total_alerts += m.total_alerts_ingested;
            total_incidents += m.incidents_created;
            pages_suppressed += m.pages_suppressed;
        }

        let total_cases = cases.len();
        let recall = if total_cases == 0 {
            1.0
        } else {
            (total_cases - missed.len()) as f64 / total_cases as f64
        };
        let reduction = if total_alerts == 0 {
            0.0
        } else {
            (pages_suppressed as f64 / total_alerts as f64) * 100.0
        };

        CorrelationReplayReport {
            total_cases,
            total_alerts,
            total_incidents,
            page_reduction_percent: reduction,
            missed_root_causes: missed,
            root_cause_recall: recall,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[tokio::test]
    async fn correlates_multi_stream_alerts_with_page_reduction_and_explanation() {
        let manager = Arc::new(IncidentManager::new());
        let correlator = IncidentCorrelator::new(manager, Duration::minutes(15));
        let now = Utc::now();

        let d1 = correlator
            .ingest(UnifiedAlert {
                id: "a1".into(),
                source: AlertStreamSource::Prometheus,
                name: "StellarNodeSyncStalled".into(),
                severity: IncidentSeverity::Critical,
                summary: "Ledger close lag > 60s".into(),
                labels: labels(&[
                    ("service", "stellar-core"),
                    ("cluster", "prod-eu"),
                    ("zone", "eu-west-1a"),
                ]),
                fired_at: now,
                resolved: false,
                is_root_cause_signal: true,
            })
            .await;

        let d2 = correlator
            .ingest(UnifiedAlert {
                id: "a2".into(),
                source: AlertStreamSource::UptimeCheck,
                name: "HorizonHealthcheckTimeout".into(),
                severity: IncidentSeverity::High,
                summary: "HTTP /health timed out".into(),
                labels: labels(&[
                    ("app", "stellar-core"),
                    ("k8s_cluster", "prod-eu"),
                    ("availability_zone", "eu-west-1a"),
                ]),
                fired_at: now + Duration::seconds(20),
                resolved: false,
                is_root_cause_signal: false,
            })
            .await;

        let d3 = correlator
            .ingest(UnifiedAlert {
                id: "a3".into(),
                source: AlertStreamSource::LogDetector,
                name: "ConsensusTimeoutSpike".into(),
                severity: IncidentSeverity::High,
                summary: "Detected 42SCP timeout log lines".into(),
                labels: labels(&[
                    ("app.kubernetes.io/name", "stellar-core"),
                    ("cluster_name", "prod-eu"),
                    ("topology.kubernetes.io/zone", "eu-west-1a"),
                ]),
                fired_at: now + Duration::seconds(45),
                resolved: false,
                is_root_cause_signal: false,
            })
            .await;

        assert!(!d1.suppressed_page);
        assert!(d2.suppressed_page);
        assert!(d3.suppressed_page);
        assert_eq!(d1.incident_id, d2.incident_id);
        assert_eq!(d2.incident_id, d3.incident_id);
        assert!(d2.explanation.contains("normalized key"));

        let metrics = correlator.metrics().await;
        assert_eq!(metrics.incidents_created, 1);
        assert!(metrics.page_reduction_percent() >= 60.0);

        let inc = correlator.get_incident(&d1.incident_id).await.unwrap();
        assert_eq!(inc.sources.len(), 3);
        assert_eq!(inc.alerts.len(), 3);
        assert!(inc.incident.labels.contains_key("correlation_decision"));
    }
}
