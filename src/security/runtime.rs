//! Runtime Security Monitoring
//!
//! Integrates with Falco for real-time threat detection and incident response.

use crate::security::{SecurityFinding, SecuritySeverity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub timestamp: String,
    pub rule: String,
    pub priority: String,
    pub container_id: String,
    pub output: String,
}

pub struct RuntimeMonitor;

impl RuntimeMonitor {
    /// Processes a Falco event and generates security findings
    pub fn process_event(event: &RuntimeEvent) -> SecurityFinding {
        let severity = match event.priority.as_str() {
            "Emergency" | "Alert" | "Critical" => SecuritySeverity::Critical,
            "Error" | "Warning" => SecuritySeverity::High,
            _ => SecuritySeverity::Medium,
        };

        SecurityFinding {
            id: format!("RUNTIME-{}", event.rule),
            component: event.container_id.clone(),
            severity,
            description: event.output.clone(),
            remediation: Some("Investigate pod for potential compromise".to_string()),
        }
    }

    /// Triggers automated response to high-priority events
    pub fn trigger_incident_response(finding: &SecurityFinding) {
        if finding.severity == SecuritySeverity::Critical {
            // Logic to isolate pod, rotate keys, or alert security team
            tracing::error!("CRITICAL SECURITY INCIDENT: {}", finding.description);
        }
    }
}
