//! Incident lifecycle management system.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum IncidentSeverity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Detected,
    Acknowledged,
    Investigating,
    Mitigating,
    Resolved,
    PostMortem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: IncidentSeverity,
    pub status: IncidentStatus,
    pub detected_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub affected_nodes: Vec<String>,
    pub labels: HashMap<String, String>,
    pub timeline: Vec<TimelineEvent>,
    pub assignee: Option<String>,
    pub runbook_url: Option<String>,
}

impl Incident {
    pub fn new(
        id: String,
        title: String,
        description: String,
        severity: IncidentSeverity,
        affected_nodes: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.clone(),
            title: title.clone(),
            description,
            severity,
            status: IncidentStatus::Detected,
            detected_at: now,
            acknowledged_at: None,
            resolved_at: None,
            affected_nodes,
            labels: HashMap::new(),
            timeline: vec![TimelineEvent {
                timestamp: now,
                actor: "system".to_string(),
                action: format!("Incident {id} created: {title}"),
                note: None,
            }],
            assignee: None,
            runbook_url: None,
        }
    }

    pub fn duration_seconds(&self) -> Option<i64> {
        self.resolved_at
            .map(|r| (r - self.detected_at).num_seconds())
    }

    pub fn is_sla_breached(&self, sla_seconds: i64) -> bool {
        let elapsed = (Utc::now() - self.detected_at).num_seconds();
        self.resolved_at.is_none() && elapsed > sla_seconds
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentDashboard {
    pub total_active: usize,
    pub by_severity: HashMap<String, usize>,
    pub mttr_seconds: Option<f64>,
    pub mttd_seconds: Option<f64>,
    pub sla_breaches: usize,
    pub recent_incidents: Vec<String>,
}

pub struct IncidentManager {
    incidents: Arc<RwLock<HashMap<String, Incident>>>,
    counter: Arc<AtomicU64>,
    /// SLA threshold in seconds per severity
    sla_thresholds: HashMap<String, i64>,
}

impl IncidentManager {
    pub fn new() -> Self {
        let mut sla = HashMap::new();
        sla.insert("critical".to_string(), 3600); // 1h
        sla.insert("high".to_string(), 14400); // 4h
        sla.insert("medium".to_string(), 86400); // 24h
        sla.insert("low".to_string(), 259200); // 72h
        Self {
            incidents: Arc::new(RwLock::new(HashMap::new())),
            counter: Arc::new(AtomicU64::new(1)),
            sla_thresholds: sla,
        }
    }

    pub fn next_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("INC-{n:06}")
    }

    pub async fn create_incident(
        &self,
        title: String,
        description: String,
        severity: IncidentSeverity,
        affected_nodes: Vec<String>,
    ) -> Incident {
        let id = self.next_id();
        let incident = Incident::new(id.clone(), title, description, severity, affected_nodes);
        info!(incident_id = %id, severity = ?incident.severity, "Incident created");
        self.incidents.write().await.insert(id, incident.clone());
        incident
    }

    pub async fn acknowledge(&self, id: &str, assignee: &str) -> Option<Incident> {
        let mut store = self.incidents.write().await;
        let inc = store.get_mut(id)?;
        inc.status = IncidentStatus::Acknowledged;
        inc.acknowledged_at = Some(Utc::now());
        inc.assignee = Some(assignee.to_string());
        inc.timeline.push(TimelineEvent {
            timestamp: Utc::now(),
            actor: assignee.to_string(),
            action: "Acknowledged incident".to_string(),
            note: None,
        });
        info!(incident_id = %id, assignee, "Incident acknowledged");
        Some(inc.clone())
    }

    pub async fn update_status(
        &self,
        id: &str,
        status: IncidentStatus,
        actor: &str,
        note: Option<String>,
    ) -> Option<Incident> {
        let mut store = self.incidents.write().await;
        let inc = store.get_mut(id)?;
        let prev = inc.status;
        inc.status = status;
        inc.timeline.push(TimelineEvent {
            timestamp: Utc::now(),
            actor: actor.to_string(),
            action: format!("Status changed from {prev:?} to {status:?}"),
            note,
        });
        Some(inc.clone())
    }

    pub async fn resolve(&self, id: &str, actor: &str, resolution: &str) -> Option<Incident> {
        let mut store = self.incidents.write().await;
        let inc = store.get_mut(id)?;
        inc.status = IncidentStatus::Resolved;
        inc.resolved_at = Some(Utc::now());
        inc.timeline.push(TimelineEvent {
            timestamp: Utc::now(),
            actor: actor.to_string(),
            action: "Incident resolved".to_string(),
            note: Some(resolution.to_string()),
        });
        info!(incident_id = %id, "Incident resolved");
        Some(inc.clone())
    }

    pub async fn get_by_id(&self, id: &str) -> Option<Incident> {
        self.incidents.read().await.get(id).cloned()
    }

    pub async fn list_active(&self) -> Vec<Incident> {
        self.incidents
            .read()
            .await
            .values()
            .filter(|i| {
                i.status != IncidentStatus::Resolved && i.status != IncidentStatus::PostMortem
            })
            .cloned()
            .collect()
    }

    pub async fn list_all(&self) -> Vec<Incident> {
        self.incidents.read().await.values().cloned().collect()
    }

    pub async fn dashboard(&self) -> IncidentDashboard {
        let store = self.incidents.read().await;
        let active: Vec<_> = store
            .values()
            .filter(|i| {
                i.status != IncidentStatus::Resolved && i.status != IncidentStatus::PostMortem
            })
            .collect();

        let mut by_severity: HashMap<String, usize> = HashMap::new();
        for inc in &active {
            *by_severity
                .entry(format!("{:?}", inc.severity).to_lowercase())
                .or_default() += 1;
        }

        let resolved: Vec<_> = store.values().filter(|i| i.resolved_at.is_some()).collect();

        let mttr = if resolved.is_empty() {
            None
        } else {
            let total: i64 = resolved.iter().filter_map(|i| i.duration_seconds()).sum();
            Some(total as f64 / resolved.len() as f64)
        };

        let sla_breaches = active
            .iter()
            .filter(|i| {
                let key = format!("{:?}", i.severity).to_lowercase();
                let threshold = self.sla_thresholds.get(&key).copied().unwrap_or(86400);
                i.is_sla_breached(threshold)
            })
            .count();

        let mut recent: Vec<_> = store.values().collect();
        recent.sort_by(|a, b| b.detected_at.cmp(&a.detected_at));
        let recent_ids = recent.iter().take(5).map(|i| i.id.clone()).collect();

        if sla_breaches > 0 {
            warn!(count = sla_breaches, "SLA breaches detected");
        }

        IncidentDashboard {
            total_active: active.len(),
            by_severity,
            mttr_seconds: mttr,
            mttd_seconds: None, // populated by detector
            sla_breaches,
            recent_incidents: recent_ids,
        }
    }
}

impl Default for IncidentManager {
    fn default() -> Self {
        Self::new()
    }
}
