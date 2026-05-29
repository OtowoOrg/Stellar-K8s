//! Identity Audit Logging and Compliance

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::error::Result;
use super::types::{Identity, IdentityEvent, IdentityEventType};

/// Audit configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditConfig {
    /// Maximum audit log entries
    pub max_entries: usize,
    /// Enable audit logging
    pub enabled: bool,
    /// Log authentication events
    pub log_authentication: bool,
    /// Log access control decisions
    pub log_access_control: bool,
    /// Log MFA events
    pub log_mfa: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            enabled: true,
            log_authentication: true,
            log_access_control: true,
            log_mfa: true,
        }
    }
}

/// Identity Audit Log
pub struct IdentityAuditLog {
    config: AuditConfig,
    events: tokio::sync::RwLock<VecDeque<IdentityEvent>>,
}

impl IdentityAuditLog {
    /// Create a new audit log
    pub async fn new(config: AuditConfig) -> Result<Self> {
        debug!("Initializing Identity Audit Log");
        Ok(Self {
            config,
            events: tokio::sync::RwLock::new(VecDeque::new()),
        })
    }

    /// Log authentication event
    pub async fn log_authentication(&self, identity: &Identity, provider: &str) -> Result<()> {
        if !self.config.enabled || !self.config.log_authentication {
            return Ok(());
        }

        let event = IdentityEvent {
            event_type: IdentityEventType::Authenticated,
            identity_id: identity.id.clone(),
            provider: provider.to_string(),
            timestamp: Utc::now(),
            details: Default::default(),
            source_ip: None,
            user_agent: None,
        };

        self.log_event(event).await?;
        Ok(())
    }

    /// Log access control decision
    pub async fn log_access_control(
        &self,
        identity: &Identity,
        resource: &str,
        action: &str,
        allowed: bool,
    ) -> Result<()> {
        if !self.config.enabled || !self.config.log_access_control {
            return Ok(());
        }

        let event_type = if allowed {
            IdentityEventType::AccessGranted
        } else {
            IdentityEventType::AccessDenied
        };

        let mut details = std::collections::HashMap::new();
        details.insert("resource".to_string(), serde_json::json!(resource));
        details.insert("action".to_string(), serde_json::json!(action));

        let event = IdentityEvent {
            event_type,
            identity_id: identity.id.clone(),
            provider: identity.provider.clone(),
            timestamp: Utc::now(),
            details,
            source_ip: None,
            user_agent: None,
        };

        self.log_event(event).await?;
        Ok(())
    }

    /// Log MFA event
    pub async fn log_mfa_event(
        &self,
        identity: &Identity,
        event_type: IdentityEventType,
    ) -> Result<()> {
        if !self.config.enabled || !self.config.log_mfa {
            return Ok(());
        }

        let event = IdentityEvent {
            event_type,
            identity_id: identity.id.clone(),
            provider: identity.provider.clone(),
            timestamp: Utc::now(),
            details: Default::default(),
            source_ip: None,
            user_agent: None,
        };

        self.log_event(event).await?;
        Ok(())
    }

    /// Log custom event
    pub async fn log_event(&self, event: IdentityEvent) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        debug!(
            "Logging identity event: {:?} for {}",
            event.event_type, event.identity_id.0
        );

        let mut events = self.events.write().await;

        // Maintain max size
        if events.len() >= self.config.max_entries {
            events.pop_front();
        }

        events.push_back(event);
        Ok(())
    }

    /// Get audit events
    pub async fn get_events(
        &self,
        identity_id: Option<&str>,
        event_type: Option<IdentityEventType>,
        limit: Option<usize>,
    ) -> Result<Vec<IdentityEvent>> {
        let events = self.events.read().await;
        let limit = limit.unwrap_or(100);

        let filtered: Vec<_> = events
            .iter()
            .filter(|e| {
                if let Some(id) = identity_id {
                    if e.identity_id.0 != id {
                        return false;
                    }
                }
                if let Some(ref et) = event_type {
                    if e.event_type != *et {
                        return false;
                    }
                }
                true
            })
            .rev()
            .take(limit)
            .cloned()
            .collect();

        Ok(filtered)
    }

    /// Get audit statistics
    pub async fn get_statistics(&self) -> Result<AuditStatistics> {
        let events = self.events.read().await;

        let total_events = events.len();
        let authentication_events = events
            .iter()
            .filter(|e| e.event_type == IdentityEventType::Authenticated)
            .count();
        let access_granted = events
            .iter()
            .filter(|e| e.event_type == IdentityEventType::AccessGranted)
            .count();
        let access_denied = events
            .iter()
            .filter(|e| e.event_type == IdentityEventType::AccessDenied)
            .count();
        let mfa_events = events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    IdentityEventType::MfaEnabled
                        | IdentityEventType::MfaDisabled
                        | IdentityEventType::MfaChallengeCompleted
                        | IdentityEventType::MfaChallengeFailed
                )
            })
            .count();

        Ok(AuditStatistics {
            total_events,
            authentication_events,
            access_granted,
            access_denied,
            mfa_events,
            timestamp: Utc::now(),
        })
    }

    /// Export audit log
    pub async fn export_audit_log(&self) -> Result<Vec<IdentityEvent>> {
        let events = self.events.read().await;
        Ok(events.iter().cloned().collect())
    }

    /// Clear audit log
    pub async fn clear_audit_log(&self) -> Result<()> {
        debug!("Clearing audit log");
        let mut events = self.events.write().await;
        events.clear();
        Ok(())
    }
}

/// Audit statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditStatistics {
    pub total_events: usize,
    pub authentication_events: usize,
    pub access_granted: usize,
    pub access_denied: usize,
    pub mfa_events: usize,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audit_log_creation() {
        let config = AuditConfig::default();
        let log = IdentityAuditLog::new(config).await.unwrap();
        assert!(log.config.enabled);
    }

    #[tokio::test]
    async fn test_log_authentication_event() {
        let config = AuditConfig::default();
        let log = IdentityAuditLog::new(config).await.unwrap();

        let identity = Identity::new(
            super::super::types::IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        log.log_authentication(&identity, "google").await.unwrap();

        let events = log.get_events(Some(&identity.id.0), None, None).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, IdentityEventType::Authenticated);
    }

    #[tokio::test]
    async fn test_audit_statistics() {
        let config = AuditConfig::default();
        let log = IdentityAuditLog::new(config).await.unwrap();

        let identity = Identity::new(
            super::super::types::IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        log.log_authentication(&identity, "google").await.unwrap();
        log.log_access_control(&identity, "resource1", "read", true)
            .await
            .unwrap();

        let stats = log.get_statistics().await.unwrap();
        assert_eq!(stats.total_events, 2);
        assert_eq!(stats.authentication_events, 1);
        assert_eq!(stats.access_granted, 1);
    }
}
