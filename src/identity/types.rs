//! Core identity types and data structures

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// Unique identity identifier
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityId(pub String);

impl IdentityId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Identity status
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum IdentityStatus {
    /// Active and usable
    Active,
    /// Temporarily suspended
    Suspended,
    /// Permanently disabled
    Disabled,
    /// Pending verification
    PendingVerification,
    /// Locked due to security incident
    Locked,
}

/// Identity attributes for fine-grained access control
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IdentityAttributes {
    /// User's email address
    pub email: Option<String>,
    /// User's display name
    pub display_name: Option<String>,
    /// User's organization
    pub organization: Option<String>,
    /// User's department
    pub department: Option<String>,
    /// User's job title
    pub job_title: Option<String>,
    /// User's location
    pub location: Option<String>,
    /// User's groups/teams
    pub groups: Vec<String>,
    /// User's roles
    pub roles: Vec<String>,
    /// Custom attributes for policy evaluation
    pub custom_attributes: HashMap<String, serde_json::Value>,
}

/// Core identity representation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    /// Unique identity identifier
    pub id: IdentityId,
    /// Identity provider that issued this identity
    pub provider: String,
    /// Subject claim from the identity provider
    pub subject: String,
    /// Identity status
    pub status: IdentityStatus,
    /// Identity attributes
    pub attributes: IdentityAttributes,
    /// When this identity was created
    pub created_at: DateTime<Utc>,
    /// When this identity was last updated
    pub updated_at: DateTime<Utc>,
    /// When this identity was last authenticated
    pub last_authenticated_at: Option<DateTime<Utc>>,
    /// When this identity expires (if applicable)
    pub expires_at: Option<DateTime<Utc>>,
    /// MFA status
    pub mfa_enabled: bool,
    /// Federated identity mappings
    pub federated_identities: Vec<FederatedIdentityMapping>,
}

impl Identity {
    /// Create a new identity
    pub fn new(id: IdentityId, provider: String, subject: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            provider,
            subject,
            status: IdentityStatus::Active,
            attributes: IdentityAttributes::default(),
            created_at: now,
            updated_at: now,
            last_authenticated_at: None,
            expires_at: None,
            mfa_enabled: false,
            federated_identities: Vec::new(),
        }
    }

    /// Check if identity is active
    pub fn is_active(&self) -> bool {
        self.status == IdentityStatus::Active
    }

    /// Check if identity has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }

    /// Check if identity is valid (active and not expired)
    pub fn is_valid(&self) -> bool {
        self.is_active() && !self.is_expired()
    }

    /// Add a federated identity mapping
    pub fn add_federated_identity(&mut self, mapping: FederatedIdentityMapping) {
        self.federated_identities.push(mapping);
    }

    /// Get federated identity by provider
    pub fn get_federated_identity(&self, provider: &str) -> Option<&FederatedIdentityMapping> {
        self.federated_identities.iter().find(|m| m.provider == provider)
    }
}

/// Federated identity mapping
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FederatedIdentityMapping {
    /// Identity provider name
    pub provider: String,
    /// Subject in the federated provider
    pub subject: String,
    /// When this mapping was created
    pub created_at: DateTime<Utc>,
    /// When this mapping was last used
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Identity context for request processing
#[derive(Clone, Debug)]
pub struct IdentityContext {
    /// The authenticated identity
    pub identity: Identity,
    /// Session ID if authenticated via session
    pub session_id: Option<String>,
    /// Authentication method used
    pub auth_method: AuthMethod,
    /// When this context was created
    pub created_at: DateTime<Utc>,
}

/// Authentication method
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// OIDC/OAuth2 token
    OidcToken,
    /// SAML assertion
    SamlAssertion,
    /// Session cookie
    SessionCookie,
    /// API key
    ApiKey,
    /// Mutual TLS certificate
    MutualTls,
    /// Kubernetes service account token
    K8sServiceAccount,
}

/// Identity event for audit logging
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityEvent {
    /// Event type
    pub event_type: IdentityEventType,
    /// Identity involved
    pub identity_id: IdentityId,
    /// Identity provider
    pub provider: String,
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    /// Event details
    pub details: HashMap<String, serde_json::Value>,
    /// Source IP address
    pub source_ip: Option<String>,
    /// User agent
    pub user_agent: Option<String>,
}

/// Identity event types
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityEventType {
    /// Identity authenticated
    Authenticated,
    /// Identity created
    Created,
    /// Identity updated
    Updated,
    /// Identity deleted
    Deleted,
    /// Identity suspended
    Suspended,
    /// Identity resumed
    Resumed,
    /// MFA enabled
    MfaEnabled,
    /// MFA disabled
    MfaDisabled,
    /// MFA challenge completed
    MfaChallengeCompleted,
    /// MFA challenge failed
    MfaChallengeFailed,
    /// Federated identity linked
    FederatedIdentityLinked,
    /// Federated identity unlinked
    FederatedIdentityUnlinked,
    /// Access granted
    AccessGranted,
    /// Access denied
    AccessDenied,
    /// Session created
    SessionCreated,
    /// Session terminated
    SessionTerminated,
    /// Password changed
    PasswordChanged,
    /// Suspicious activity detected
    SuspiciousActivityDetected,
}

/// Identity provider configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityProviderConfig {
    /// Provider name
    pub name: String,
    /// Provider type
    pub provider_type: ProviderType,
    /// Provider-specific configuration
    pub config: HashMap<String, serde_json::Value>,
    /// Whether this provider is enabled
    pub enabled: bool,
    /// Priority for provider selection (lower = higher priority)
    pub priority: u32,
}

/// Identity provider type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// OpenID Connect provider
    Oidc,
    /// SAML 2.0 provider
    Saml,
    /// OAuth2 provider
    Oauth2,
    /// LDAP directory
    Ldap,
    /// Active Directory
    ActiveDirectory,
    /// Custom provider
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_creation() {
        let id = IdentityId::new("user123");
        let identity = Identity::new(id.clone(), "google".to_string(), "user@example.com".to_string());
        
        assert_eq!(identity.id, id);
        assert_eq!(identity.provider, "google");
        assert!(identity.is_active());
        assert!(!identity.is_expired());
        assert!(identity.is_valid());
    }

    #[test]
    fn test_identity_expiration() {
        let id = IdentityId::new("user123");
        let mut identity = Identity::new(id, "google".to_string(), "user@example.com".to_string());
        identity.expires_at = Some(Utc::now() - chrono::Duration::hours(1));
        
        assert!(identity.is_expired());
        assert!(!identity.is_valid());
    }

    #[test]
    fn test_federated_identity_mapping() {
        let id = IdentityId::new("user123");
        let mut identity = Identity::new(id, "google".to_string(), "user@example.com".to_string());
        
        let mapping = FederatedIdentityMapping {
            provider: "okta".to_string(),
            subject: "okta_user_123".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
        };
        
        identity.add_federated_identity(mapping.clone());
        
        let retrieved = identity.get_federated_identity("okta");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().subject, "okta_user_123");
    }
}
