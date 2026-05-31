//! Automatic incident detection from Prometheus alerts and Kubernetes events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use super::manager::{Incident, IncidentManager, IncidentSeverity};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusAlert {
    pub name: String,
    pub severity: String,
    pub summary: String,
    pub labels: std::collections::HashMap<String, String>,
    pub fired_at: DateTime<Utc>,
}

impl PrometheusAlert {
    fn to_severity(&self) -> IncidentSeverity {
        match self.severity.to_lowercase().as_str() {
            "critical" => IncidentSeverity::Critical,
            "high" => IncidentSeverity::High,
            "warning" | "medium" => IncidentSeverity::Medium,
            _ => IncidentSeverity::Low,
        }
    }

    fn affected_nodes(&self) -> Vec<String> {
        let mut nodes = Vec::new();
        if let Some(node) = self.labels.get("node") {
            nodes.push(node.clone());
        }
        if let Some(pod) = self.labels.get("pod") {
            nodes.push(pod.clone());
        }
        nodes
    }
}

/// Watches Prometheus alertmanager webhook and creates incidents automatically.
pub struct AlertDetector {
    manager: Arc<IncidentManager>,
    alertmanager_url: String,
}

impl AlertDetector {
    pub fn new(manager: Arc<IncidentManager>, alertmanager_url: String) -> Self {
        Self {
            manager,
            alertmanager_url,
        }
    }

    /// Process a batch of alerts from Alertmanager webhook payload.
    pub async fn process_alerts(&self, alerts: Vec<PrometheusAlert>) -> Vec<Incident> {
        let mut created = Vec::new();
        for alert in alerts {
            // Skip resolved alerts
            if alert
                .labels
                .get("status")
                .map(|s| s == "resolved")
                .unwrap_or(false)
            {
                continue;
            }
            let severity = alert.to_severity();
            let affected = alert.affected_nodes();
            let description = format!(
                "Alert '{}' fired at {}. Summary: {}",
                alert.name, alert.fired_at, alert.summary
            );
            info!(alert = %alert.name, severity = ?severity, "Creating incident from alert");
            let incident = self
                .manager
                .create_incident(
                    format!("[AUTO] {}", alert.name),
                    description,
                    severity,
                    affected,
                )
                .await;
            created.push(incident);
        }
        created
    }

    /// Poll Alertmanager for firing alerts and create incidents.
    pub async fn poll_once(&self) -> anyhow::Result<Vec<Incident>> {
        let url = format!(
            "{}/api/v2/alerts?active=true&silenced=false",
            self.alertmanager_url
        );
        let resp = reqwest::get(&url).await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let raw: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
                let alerts: Vec<PrometheusAlert> = raw
                    .into_iter()
                    .filter_map(|v| parse_alertmanager_alert(v))
                    .collect();
                Ok(self.process_alerts(alerts).await)
            }
            Ok(r) => {
                warn!(status = %r.status(), "Alertmanager returned non-success");
                Ok(vec![])
            }
            Err(e) => {
                error!(error = %e, "Failed to poll Alertmanager");
                Ok(vec![])
            }
        }
    }

    /// Run continuous polling loop.
    pub async fn run(&self, interval_secs: u64) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            if let Err(e) = self.poll_once().await {
                error!(error = %e, "Alert polling error");
            }
        }
    }
}

fn parse_alertmanager_alert(v: serde_json::Value) -> Option<PrometheusAlert> {
    let labels = v.get("labels")?.as_object()?;
    let name = labels.get("alertname")?.as_str()?.to_string();
    let severity = labels
        .get("severity")
        .and_then(|s| s.as_str())
        .unwrap_or("low")
        .to_string();
    let annotations = v.get("annotations").and_then(|a| a.as_object());
    let summary = annotations
        .and_then(|a| a.get("summary"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let label_map = labels
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
        .collect();
    Some(PrometheusAlert {
        name,
        severity,
        summary,
        labels: label_map,
        fired_at: Utc::now(),
    })
}
