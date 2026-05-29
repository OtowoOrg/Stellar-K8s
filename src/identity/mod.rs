//! Advanced Identity Management System with SSO and Federation
//!
//! Provides comprehensive identity management supporting:
//! - Single Sign-On (SSO) with multiple OIDC providers
//! - Identity federation across multiple identity providers
//! - Multi-factor authentication (MFA) with TOTP and WebAuthn
//! - Fine-grained access control with attribute-based policies
//! - Identity lifecycle management and audit trails
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Identity Management System                              │
//! ├─────────────────────────────────────────────────────────┤
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
//! │  │ SSO Provider │  │ Federation   │  │ MFA Engine   │   │
//! │  │ (OIDC/SAML)  │  │ (Cross-Realm)│  │ (TOTP/WebA)  │   │
//! │  └──────────────┘  └──────────────┘  └──────────────┘   │
//! │         │                 │                 │             │
//! │         └─────────────────┴─────────────────┘             │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Identity Context Store  │                      │
//! │         │ (In-Memory + Redis)     │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Access Control Engine   │                      │
//! │         │ (ABAC + RBAC)           │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Audit & Compliance      │                      │
//! │         │ (Event Log + Metrics)   │                      │
//! │         └────────────────────────┘                      │
//! └─────────────────────────────────────────────────────────┘
//! ```

pub mod access_control;
pub mod audit;
pub mod federation;
pub mod mfa;
pub mod provider;
pub mod session;
pub mod store;
pub mod types;

pub use access_control::{AccessControlEngine, AccessDecision, AccessRequest};
pub use audit::{IdentityAuditLog, IdentityAuditEvent};
pub use federation::{FederationManager, FederatedIdentity};
pub use mfa::{MfaEngine, MfaChallenge, MfaMethod};
pub use provider::{IdentityProvider, ProviderConfig};
pub use session::{SessionManager, Session};
pub use store::{IdentityStore, IdentityContext};
pub use types::{Identity, IdentityAttributes, IdentityStatus};

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Main Identity Management System
pub struct IdentityManagementSystem {
    /// Identity provider registry
    providers: Arc<RwLock<Vec<Arc<dyn IdentityProvider>>>>,
    /// Federation manager for cross-realm identity
    federation: Arc<FederationManager>,
    /// MFA engine for multi-factor authentication
    mfa: Arc<MfaEngine>,
    /// Session manager for identity sessions
    sessions: Arc<SessionManager>,
    /// Identity context store
    store: Arc<IdentityStore>,
    /// Access control engine
    access_control: Arc<AccessControlEngine>,
    /// Audit log
    audit: Arc<IdentityAuditLog>,
}

impl IdentityManagementSystem {
    /// Create a new identity management system
    pub async fn new(config: IdentitySystemConfig) -> crate::error::Result<Self> {
        info!("Initializing Identity Management System");

        let store = Arc::new(IdentityStore::new(config.store_config).await?);
        let federation = Arc::new(FederationManager::new(config.federation_config).await?);
        let mfa = Arc::new(MfaEngine::new(config.mfa_config).await?);
        let sessions = Arc::new(SessionManager::new(config.session_config).await?);
        let access_control = Arc::new(AccessControlEngine::new(config.access_control_config).await?);
        let audit = Arc::new(IdentityAuditLog::new(config.audit_config).await?);

        Ok(Self {
            providers: Arc::new(RwLock::new(Vec::new())),
            federation,
            mfa,
            sessions,
            store,
            access_control,
            audit,
        })
    }

    /// Register an identity provider
    pub async fn register_provider(&self, provider: Arc<dyn IdentityProvider>) -> crate::error::Result<()> {
        let mut providers = self.providers.write().await;
        providers.push(provider);
        Ok(())
    }

    /// Authenticate a user with SSO
    pub async fn authenticate_sso(&self, provider_name: &str, token: &str) -> crate::error::Result<Identity> {
        let providers = self.providers.read().await;
        let provider = providers
            .iter()
            .find(|p| p.name() == provider_name)
            .ok_or_else(|| crate::error::Error::NotFound(format!("Provider {} not found", provider_name)))?;

        let identity = provider.validate_token(token).await?;
        
        // Log authentication event
        self.audit.log_authentication(&identity, provider_name).await?;

        Ok(identity)
    }

    /// Get federation manager
    pub fn federation(&self) -> Arc<FederationManager> {
        self.federation.clone()
    }

    /// Get MFA engine
    pub fn mfa(&self) -> Arc<MfaEngine> {
        self.mfa.clone()
    }

    /// Get session manager
    pub fn sessions(&self) -> Arc<SessionManager> {
        self.sessions.clone()
    }

    /// Get identity store
    pub fn store(&self) -> Arc<IdentityStore> {
        self.store.clone()
    }

    /// Get access control engine
    pub fn access_control(&self) -> Arc<AccessControlEngine> {
        self.access_control.clone()
    }

    /// Get audit log
    pub fn audit(&self) -> Arc<IdentityAuditLog> {
        self.audit.clone()
    }
}

/// Configuration for the identity management system
#[derive(Clone, Debug)]
pub struct IdentitySystemConfig {
    pub store_config: store::IdentityStoreConfig,
    pub federation_config: federation::FederationConfig,
    pub mfa_config: mfa::MfaConfig,
    pub session_config: session::SessionConfig,
    pub access_control_config: access_control::AccessControlConfig,
    pub audit_config: audit::AuditConfig,
}

impl Default for IdentitySystemConfig {
    fn default() -> Self {
        Self {
            store_config: Default::default(),
            federation_config: Default::default(),
            mfa_config: Default::default(),
            session_config: Default::default(),
            access_control_config: Default::default(),
            audit_config: Default::default(),
        }
    }
}
