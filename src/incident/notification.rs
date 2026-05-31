//! Incident communication and notifications.

use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::manager::{Incident, IncidentSeverity};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NotificationChannel {
    Slack {
        webhook_url: String,
        channel: String,
    },
    PagerDuty {
        routing_key: String,
    },
    Email {
        smtp_url: String,
        recipients: Vec<String>,
    },
    Webhook {
        url: String,
        secret: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub channels: Vec<NotificationChannel>,
    /// Only notify for these severities
    pub min_severity: IncidentSeverity,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            channels: vec![],
            min_severity: IncidentSeverity::Medium,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SlackPayload {
    text: String,
    channel: String,
    username: String,
    icon_emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PagerDutyPayload {
    routing_key: String,
    event_action: String,
    payload: PagerDutyEventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PagerDutyEventPayload {
    summary: String,
    severity: String,
    source: String,
}

pub struct NotificationManager {
    config: NotificationConfig,
    client: reqwest::Client,
}

impl NotificationManager {
    pub fn new(config: NotificationConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub async fn notify_incident_created(&self, incident: &Incident) {
        if incident.severity > self.config.min_severity {
            return;
        }
        let msg = format!(
            "🚨 *[{}] Incident Created*: {} ({})\nAffected: {}",
            incident.id,
            incident.title,
            format!("{:?}", incident.severity),
            incident.affected_nodes.join(", ")
        );
        self.send_all(&msg, incident).await;
    }

    pub async fn notify_incident_resolved(&self, incident: &Incident) {
        let duration = incident
            .duration_seconds()
            .map(|s| format!(" in {}m", s / 60))
            .unwrap_or_default();
        let msg = format!(
            "✅ *[{}] Incident Resolved*{}: {}",
            incident.id, duration, incident.title
        );
        self.send_all(&msg, incident).await;
    }

    pub async fn notify_sla_breach(&self, incident: &Incident) {
        let msg = format!(
            "⚠️ *SLA BREACH*: Incident {} ({:?}) has exceeded SLA threshold",
            incident.id, incident.severity
        );
        self.send_all(&msg, incident).await;
    }

    async fn send_all(&self, message: &str, incident: &Incident) {
        for channel in &self.config.channels {
            if let Err(e) = self.send(channel, message, incident).await {
                warn!(error = %e, "Failed to send notification");
            }
        }
    }

    async fn send(
        &self,
        channel: &NotificationChannel,
        message: &str,
        incident: &Incident,
    ) -> anyhow::Result<()> {
        match channel {
            NotificationChannel::Slack {
                webhook_url,
                channel: ch,
            } => {
                let payload = SlackPayload {
                    text: message.to_string(),
                    channel: ch.clone(),
                    username: "stellar-operator".to_string(),
                    icon_emoji: ":satellite:".to_string(),
                };
                let resp = self.client.post(webhook_url).json(&payload).send().await?;
                if !resp.status().is_success() {
                    error!(status = %resp.status(), "Slack notification failed");
                } else {
                    info!(channel = %ch, "Slack notification sent");
                }
            }
            NotificationChannel::PagerDuty { routing_key } => {
                let severity = match incident.severity {
                    IncidentSeverity::Critical => "critical",
                    IncidentSeverity::High => "error",
                    IncidentSeverity::Medium => "warning",
                    IncidentSeverity::Low => "info",
                };
                let payload = PagerDutyPayload {
                    routing_key: routing_key.clone(),
                    event_action: "trigger".to_string(),
                    payload: PagerDutyEventPayload {
                        summary: message.to_string(),
                        severity: severity.to_string(),
                        source: "stellar-operator".to_string(),
                    },
                };
                self.client
                    .post("https://events.pagerduty.com/v2/enqueue")
                    .json(&payload)
                    .send()
                    .await?;
                info!("PagerDuty notification sent");
            }
            NotificationChannel::Webhook { url, secret } => {
                let mut req = self.client.post(url).json(&serde_json::json!({
                    "incident_id": incident.id,
                    "message": message,
                    "severity": format!("{:?}", incident.severity),
                }));
                if let Some(s) = secret {
                    req = req.header("X-Webhook-Secret", s);
                }
                req.send().await?;
                info!(url = %url, "Webhook notification sent");
            }
            NotificationChannel::Email {
                smtp_url: _,
                recipients,
            } => {
                // Email sending requires an SMTP library; log intent
                info!(
                    recipients = ?recipients,
                    message = %message,
                    "Email notification (SMTP not configured)"
                );
            }
        }
        Ok(())
    }
}
