//! Session Management for Identity

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};
use tracing::{debug, warn};

use crate::error::Result;
use super::types::{Identity, IdentityId};

/// Session configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Session timeout in seconds
    pub session_timeout_secs: u64,
    /// Idle timeout in seconds
    pub idle_timeout_secs: u64,
    /// Maximum sessions per identity
    pub max_sessions_per_identity: usize,
    /// Enable session persistence
    pub persist_sessions: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            session_timeout_secs: 3600,      // 1 hour
            idle_timeout_secs: 1800,         // 30 minutes
            max_sessions_per_identity: 5,
            persist_sessions: false,
        }
    }
}

/// Session
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Session ID
    pub id: String,
    /// Identity
    pub identity: Identity,
    /// When session was created
    pub created_at: DateTime<Utc>,
    /// When session expires
    pub expires_at: DateTime<Utc>,
    /// When session was last accessed
    pub last_accessed_at: DateTime<Utc>,
    /// Session metadata
    pub metadata: HashMap<String, serde_json::Value>,
    /// Whether session is active
    pub active: bool,
}

impl Session {
    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Check if session is idle
    pub fn is_idle(&self, idle_timeout_secs: u64) -> bool {
        let idle_duration = Duration::seconds(idle_timeout_secs as i64);
        Utc::now() - self.last_accessed_at > idle_duration
    }

    /// Check if session is valid
    pub fn is_valid(&self, idle_timeout_secs: u64) -> bool {
        self.active && !self.is_expired() && !self.is_idle(idle_timeout_secs)
    }

    /// Update last accessed time
    pub fn touch(&mut self) {
        self.last_accessed_at = Utc::now();
    }
}

/// Session Manager
pub struct SessionManager {
    config: SessionConfig,
    sessions: tokio::sync::RwLock<HashMap<String, Session>>,
    identity_sessions: tokio::sync::RwLock<HashMap<String, Vec<String>>>,
}

impl SessionManager {
    /// Create a new session manager
    pub async fn new(config: SessionConfig) -> Result<Self> {
        debug!("Initializing Session Manager");
        Ok(Self {
            config,
            sessions: tokio::sync::RwLock::new(HashMap::new()),
            identity_sessions: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Create a new session
    pub async fn create_session(&self, identity: Identity) -> Result<Session> {
        debug!("Creating session for identity: {}", identity.id.0);

        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(self.config.session_timeout_secs as i64);

        let session = Session {
            id: session_id.clone(),
            identity: identity.clone(),
            created_at: now,
            expires_at,
            last_accessed_at: now,
            metadata: HashMap::new(),
            active: true,
        };

        // Check max sessions per identity
        let mut identity_sessions = self.identity_sessions.write().await;
        let sessions_for_identity = identity_sessions
            .entry(identity.id.0.clone())
            .or_insert_with(Vec::new);

        if sessions_for_identity.len() >= self.config.max_sessions_per_identity {
            // Remove oldest session
            if let Some(oldest_id) = sessions_for_identity.first() {
                let mut sessions = self.sessions.write().await;
                sessions.remove(oldest_id);
                sessions_for_identity.remove(0);
            }
        }

        sessions_for_identity.push(session_id.clone());

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), session.clone());

        Ok(session)
    }

    /// Get session
    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            if session.is_valid(self.config.idle_timeout_secs) {
                session.touch();
                return Ok(Some(session.clone()));
            } else {
                sessions.remove(session_id);
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Terminate session
    pub async fn terminate_session(&self, session_id: &str) -> Result<()> {
        debug!("Terminating session: {}", session_id);

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.active = false;
        }
        sessions.remove(session_id);

        Ok(())
    }

    /// Terminate all sessions for identity
    pub async fn terminate_all_sessions(&self, identity_id: &IdentityId) -> Result<()> {
        debug!("Terminating all sessions for identity: {}", identity_id.0);

        let mut identity_sessions = self.identity_sessions.write().await;
        if let Some(session_ids) = identity_sessions.remove(&identity_id.0) {
            let mut sessions = self.sessions.write().await;
            for session_id in session_ids {
                sessions.remove(&session_id);
            }
        }

        Ok(())
    }

    /// Get all sessions for identity
    pub async fn get_identity_sessions(&self, identity_id: &IdentityId) -> Result<Vec<Session>> {
        let identity_sessions = self.identity_sessions.read().await;
        if let Some(session_ids) = identity_sessions.get(&identity_id.0) {
            let sessions = self.sessions.read().await;
            let mut result = Vec::new();
            for session_id in session_ids {
                if let Some(session) = sessions.get(session_id) {
                    if session.is_valid(self.config.idle_timeout_secs) {
                        result.push(session.clone());
                    }
                }
            }
            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired_sessions(&self) -> Result<usize> {
        debug!("Cleaning up expired sessions");

        let mut sessions = self.sessions.write().await;
        let mut identity_sessions = self.identity_sessions.write().await;

        let initial_count = sessions.len();

        // Remove expired sessions
        sessions.retain(|_, session| {
            !session.is_expired() && !session.is_idle(self.config.idle_timeout_secs)
        });

        // Clean up identity_sessions mappings
        for session_ids in identity_sessions.values_mut() {
            session_ids.retain(|id| sessions.contains_key(id));
        }

        let removed = initial_count - sessions.len();
        debug!("Cleaned up {} expired sessions", removed);

        Ok(removed)
    }

    /// Get session statistics
    pub async fn get_statistics(&self) -> Result<SessionStatistics> {
        let sessions = self.sessions.read().await;
        let identity_sessions = self.identity_sessions.read().await;

        let total_sessions = sessions.len();
        let active_sessions = sessions.values().filter(|s| s.active).count();
        let total_identities = identity_sessions.len();

        Ok(SessionStatistics {
            total_sessions,
            active_sessions,
            total_identities,
            timestamp: Utc::now(),
        })
    }
}

/// Session statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionStatistics {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub total_identities: usize,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_manager_creation() {
        let config = SessionConfig::default();
        let manager = SessionManager::new(config).await.unwrap();
        assert_eq!(manager.config.session_timeout_secs, 3600);
    }

    #[tokio::test]
    async fn test_session_creation() {
        let config = SessionConfig::default();
        let manager = SessionManager::new(config).await.unwrap();

        let identity = Identity::new(
            IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        let session = manager.create_session(identity).await.unwrap();
        assert!(session.active);
        assert!(!session.is_expired());
    }

    #[tokio::test]
    async fn test_session_retrieval() {
        let config = SessionConfig::default();
        let manager = SessionManager::new(config).await.unwrap();

        let identity = Identity::new(
            IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        let session = manager.create_session(identity).await.unwrap();
        let retrieved = manager.get_session(&session.id).await.unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, session.id);
    }

    #[tokio::test]
    async fn test_session_termination() {
        let config = SessionConfig::default();
        let manager = SessionManager::new(config).await.unwrap();

        let identity = Identity::new(
            IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        let session = manager.create_session(identity).await.unwrap();
        manager.terminate_session(&session.id).await.unwrap();

        let retrieved = manager.get_session(&session.id).await.unwrap();
        assert!(retrieved.is_none());
    }
}
