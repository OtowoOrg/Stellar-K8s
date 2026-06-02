//! Incident metrics and SLA tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::manager::{Incident, IncidentSeverity, IncidentStatus};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncidentMetrics {
    pub total_incidents: u64,
    pub resolved_incidents: u64,
    pub active_incidents: u64,
    /// Mean Time To Detect (seconds)
    pub mttd_seconds: Option<f64>,
    /// Mean Time To Resolve (seconds)
    pub mttr_seconds: Option<f64>,
    /// Mean Time To Acknowledge (seconds)
    pub mtta_seconds: Option<f64>,
    pub sla_compliance_pct: f64,
    pub by_severity: HashMap<String, SeverityMetrics>,
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SeverityMetrics {
    pub count: u64,
    pub resolved: u64,
    pub sla_breaches: u64,
    pub avg_resolution_minutes: Option<f64>,
}

/// SLA thresholds in seconds per severity level.
pub fn default_sla_thresholds() -> HashMap<String, i64> {
    let mut m = HashMap::new();
    m.insert("critical".to_string(), 3600); // 1h
    m.insert("high".to_string(), 14400); // 4h
    m.insert("medium".to_string(), 86400); // 24h
    m.insert("low".to_string(), 259200); // 72h
    m
}

pub struct SlaTracker {
    thresholds: HashMap<String, i64>,
}

impl SlaTracker {
    pub fn new(thresholds: HashMap<String, i64>) -> Self {
        Self { thresholds }
    }

    pub fn is_breached(&self, incident: &Incident) -> bool {
        let key = format!("{:?}", incident.severity).to_lowercase();
        let threshold = self.thresholds.get(&key).copied().unwrap_or(86400);
        incident.is_sla_breached(threshold)
    }

    pub fn compute_metrics(&self, incidents: &[Incident]) -> IncidentMetrics {
        let total = incidents.len() as u64;
        let resolved: Vec<_> = incidents
            .iter()
            .filter(|i| {
                i.status == IncidentStatus::Resolved || i.status == IncidentStatus::PostMortem
            })
            .collect();
        let active = incidents
            .iter()
            .filter(|i| {
                i.status != IncidentStatus::Resolved && i.status != IncidentStatus::PostMortem
            })
            .count() as u64;

        let mttr = if resolved.is_empty() {
            None
        } else {
            let sum: i64 = resolved.iter().filter_map(|i| i.duration_seconds()).sum();
            Some(sum as f64 / resolved.len() as f64)
        };

        let mtta = {
            let acked: Vec<_> = incidents
                .iter()
                .filter_map(|i| i.acknowledged_at.map(|a| (a - i.detected_at).num_seconds()))
                .collect();
            if acked.is_empty() {
                None
            } else {
                Some(acked.iter().sum::<i64>() as f64 / acked.len() as f64)
            }
        };

        let mut sla_breaches = 0u64;
        let mut by_severity: HashMap<String, SeverityMetrics> = HashMap::new();

        for inc in incidents {
            let key = format!("{:?}", inc.severity).to_lowercase();
            let entry = by_severity.entry(key.clone()).or_default();
            entry.count += 1;
            if inc.status == IncidentStatus::Resolved || inc.status == IncidentStatus::PostMortem {
                entry.resolved += 1;
            }
            if self.is_breached(inc) {
                entry.sla_breaches += 1;
                sla_breaches += 1;
            }
        }

        // Compute avg resolution per severity
        for (key, metrics) in &mut by_severity {
            let sev_resolved: Vec<_> = incidents
                .iter()
                .filter(|i| {
                    format!("{:?}", i.severity).to_lowercase() == *key && i.resolved_at.is_some()
                })
                .collect();
            if !sev_resolved.is_empty() {
                let sum: i64 = sev_resolved
                    .iter()
                    .filter_map(|i| i.duration_seconds())
                    .sum();
                metrics.avg_resolution_minutes =
                    Some(sum as f64 / sev_resolved.len() as f64 / 60.0);
            }
        }

        let sla_compliance_pct = if total == 0 {
            100.0
        } else {
            ((total - sla_breaches) as f64 / total as f64) * 100.0
        };

        IncidentMetrics {
            total_incidents: total,
            resolved_incidents: resolved.len() as u64,
            active_incidents: active,
            mttd_seconds: None, // populated externally from alert timestamps
            mttr_seconds: mttr,
            mtta_seconds: mtta,
            sla_compliance_pct,
            by_severity,
            computed_at: Utc::now(),
        }
    }
}

impl Default for SlaTracker {
    fn default() -> Self {
        Self::new(default_sla_thresholds())
    }
}
