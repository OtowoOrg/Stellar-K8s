//! Post-incident review and Root Cause Analysis (RCA) automation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::manager::Incident;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcaSection {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcaReport {
    pub incident_id: String,
    pub title: String,
    pub generated_at: DateTime<Utc>,
    pub severity: String,
    pub duration_minutes: Option<i64>,
    pub affected_nodes: Vec<String>,
    pub timeline_summary: Vec<String>,
    pub sections: Vec<RcaSection>,
    pub action_items: Vec<ActionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    pub description: String,
    pub owner: Option<String>,
    pub due_date: Option<String>,
    pub priority: String,
}

pub struct RcaGenerator;

impl RcaGenerator {
    pub fn generate(incident: &Incident) -> RcaReport {
        let duration_minutes = incident.duration_seconds().map(|s| s / 60);

        let timeline_summary: Vec<String> = incident
            .timeline
            .iter()
            .map(|e| {
                format!(
                    "[{}] {}: {}",
                    e.timestamp.format("%H:%M:%S"),
                    e.actor,
                    e.action
                )
            })
            .collect();

        let sections = vec![
            RcaSection {
                title: "Executive Summary".to_string(),
                content: format!(
                    "Incident {} ({:?}) was detected at {} and {}. \
                     {} node(s) were affected.",
                    incident.id,
                    incident.severity,
                    incident.detected_at.format("%Y-%m-%d %H:%M UTC"),
                    duration_minutes
                        .map(|d| format!("resolved after {d} minutes"))
                        .unwrap_or_else(|| "is still ongoing".to_string()),
                    incident.affected_nodes.len()
                ),
            },
            RcaSection {
                title: "Impact".to_string(),
                content: format!(
                    "Affected nodes: {}",
                    if incident.affected_nodes.is_empty() {
                        "Unknown".to_string()
                    } else {
                        incident.affected_nodes.join(", ")
                    }
                ),
            },
            RcaSection {
                title: "Root Cause".to_string(),
                content: "TODO: Fill in root cause analysis based on collected evidence."
                    .to_string(),
            },
            RcaSection {
                title: "Contributing Factors".to_string(),
                content: "TODO: List contributing factors identified during investigation."
                    .to_string(),
            },
            RcaSection {
                title: "Detection".to_string(),
                content: format!(
                    "Incident was detected via automated alert monitoring at {}.",
                    incident.detected_at.format("%Y-%m-%d %H:%M UTC")
                ),
            },
            RcaSection {
                title: "Response".to_string(),
                content: format!(
                    "Response timeline had {} events. {}",
                    incident.timeline.len(),
                    incident
                        .assignee
                        .as_deref()
                        .map(|a| format!("Assigned to: {a}"))
                        .unwrap_or_else(|| "No assignee recorded.".to_string())
                ),
            },
            RcaSection {
                title: "Lessons Learned".to_string(),
                content: "TODO: Document lessons learned from this incident.".to_string(),
            },
        ];

        let action_items = vec![
            ActionItem {
                description: "Document root cause in this RCA".to_string(),
                owner: incident.assignee.clone(),
                due_date: Some(
                    (Utc::now() + chrono::Duration::days(3))
                        .format("%Y-%m-%d")
                        .to_string(),
                ),
                priority: "high".to_string(),
            },
            ActionItem {
                description: "Add monitoring/alerting to detect this class of issue earlier"
                    .to_string(),
                owner: None,
                due_date: Some(
                    (Utc::now() + chrono::Duration::days(7))
                        .format("%Y-%m-%d")
                        .to_string(),
                ),
                priority: "medium".to_string(),
            },
            ActionItem {
                description: "Update runbook with findings from this incident".to_string(),
                owner: None,
                due_date: Some(
                    (Utc::now() + chrono::Duration::days(14))
                        .format("%Y-%m-%d")
                        .to_string(),
                ),
                priority: "low".to_string(),
            },
        ];

        RcaReport {
            incident_id: incident.id.clone(),
            title: format!("RCA: {}", incident.title),
            generated_at: Utc::now(),
            severity: format!("{:?}", incident.severity),
            duration_minutes,
            affected_nodes: incident.affected_nodes.clone(),
            timeline_summary,
            sections,
            action_items,
        }
    }

    pub fn to_markdown(report: &RcaReport) -> String {
        let mut md = format!(
            "# {}\n\n**Incident ID**: {}  \n**Generated**: {}  \n**Severity**: {}  \n",
            report.title,
            report.incident_id,
            report.generated_at.format("%Y-%m-%d %H:%M UTC"),
            report.severity,
        );

        if let Some(d) = report.duration_minutes {
            md.push_str(&format!("**Duration**: {d} minutes  \n"));
        }

        md.push_str("\n## Timeline\n\n");
        for event in &report.timeline_summary {
            md.push_str(&format!("- {event}\n"));
        }

        for section in &report.sections {
            md.push_str(&format!("\n## {}\n\n{}\n", section.title, section.content));
        }

        md.push_str("\n## Action Items\n\n");
        for item in &report.action_items {
            let owner = item.owner.as_deref().unwrap_or("TBD");
            let due = item.due_date.as_deref().unwrap_or("TBD");
            md.push_str(&format!(
                "- [ ] **[{}]** {} (Owner: {}, Due: {})\n",
                item.priority.to_uppercase(),
                item.description,
                owner,
                due
            ));
        }

        md
    }
}
