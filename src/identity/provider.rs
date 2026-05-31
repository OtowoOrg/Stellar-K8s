//! Identity provider abstraction and implementations

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error};

use crate::error::Result;
use super::types::{Identity, IdentityId, IdentityAttributes, ProviderType};

/// Configuration for an identity provider
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: ProviderType,
    pub config: HashMap<String, serde_json::Value>,
    pub enabled: bool,
    pub priority: u32,
}

/// Identity provider trait
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &str;

    /// Get provider type
    fn provider_type(&self) -> ProviderType;

    /// Validate a token and return the identity
    async fn validate_token(&self, token: &str) -> Result<Identity>;

    /// Validate a SAML assertion
    async fn validate_saml_assertion(&self, _assertion: &str) -> Result<Identity> {
        Err(crate::error::Error::NotImplemented(
            "SAML validation not implemented for this provider".to_string(),
        ))
    }

    /// Refresh an identity
    async fn refresh_identity(&self, identity: &Identity) -> Result<Identity>;

    /// Revoke an identity
    async fn revoke_identity(&self, identity: &Identity) -> Result<()>;

    /// Get user info from provider
    async fn get_user_info(&self, token: &str) -> Result<UserInfo>;
}

/// User information from identity provider
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub subject: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    pub roles: Vec<String>,
    pub custom_attributes: HashMap<String, serde_json::Value>,
}

/// OIDC Identity Provider
pub struct OidcProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl OidcProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn get_issuer(&self) -> Result<String> {
        self.config
            .config
            .get("issuer")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| crate::error::Error::InvalidConfig("Missing OIDC issuer".to_string()))
    }

    fn get_jwks_uri(&self) -> Result<String> {
        self.config
            .config
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| crate::error::Error::InvalidConfig("Missing JWKS URI".to_string()))
    }

    fn get_audience(&self) -> Result<String> {
        self.config
            .config
            .get("audience")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| crate::error::Error::InvalidConfig("Missing audience".to_string()))
    }
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Oidc
    }

    async fn validate_token(&self, token: &str) -> Result<Identity> {
        debug!("Validating OIDC token for provider: {}", self.name());

        // In production, this would validate JWT signature using JWKS
        // For now, we'll parse the JWT and extract claims
        let claims = parse_jwt_claims(token)?;

        let subject = claims
            .get("sub")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::Error::InvalidToken("Missing subject claim".to_string()))?
            .to_string();

        let email = claims.get("email").and_then(|v| v.as_str()).map(|s| s.to_string());
        let display_name = claims
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let groups: Vec<String> = claims
            .get("groups")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let roles: Vec<String> = claims
            .get("roles")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let id = IdentityId::new(format!("{}:{}", self.name(), subject));
        let mut identity = Identity::new(id, self.name().to_string(), subject);

        identity.attributes.email = email;
        identity.attributes.display_name = display_name;
        identity.attributes.groups = groups;
        identity.attributes.roles = roles;

        Ok(identity)
    }

    async fn refresh_identity(&self, identity: &Identity) -> Result<Identity> {
        debug!("Refreshing identity: {}", identity.id.0);
        // In production, this would use refresh tokens
        Ok(identity.clone())
    }

    async fn revoke_identity(&self, identity: &Identity) -> Result<()> {
        debug!("Revoking identity: {}", identity.id.0);
        // In production, this would revoke tokens at the provider
        Ok(())
    }

    async fn get_user_info(&self, token: &str) -> Result<UserInfo> {
        let claims = parse_jwt_claims(token)?;

        let subject = claims
            .get("sub")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::Error::InvalidToken("Missing subject claim".to_string()))?
            .to_string();

        let email = claims.get("email").and_then(|v| v.as_str()).map(|s| s.to_string());
        let display_name = claims
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let groups: Vec<String> = claims
            .get("groups")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let roles: Vec<String> = claims
            .get("roles")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(UserInfo {
            subject,
            email,
            display_name,
            groups,
            roles,
            custom_attributes: HashMap::new(),
        })
    }
}

/// Parse JWT claims (simplified - in production use jsonwebtoken crate)
fn parse_jwt_claims(token: &str) -> Result<serde_json::Map<String, serde_json::Value>> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(crate::error::Error::InvalidToken("Invalid JWT format".to_string()));
    }

    let payload = parts[1];
    let decoded = base64_decode(payload)?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| crate::error::Error::InvalidToken(format!("Failed to parse claims: {}", e)))?;

    claims
        .as_object()
        .cloned()
        .ok_or_else(|| crate::error::Error::InvalidToken("Claims is not an object".to_string()))
}

/// Base64 decode with padding
fn base64_decode(input: &str) -> Result<Vec<u8>> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    
    // Add padding if needed
    let padding = (4 - (input.len() % 4)) % 4;
    let padded = format!("{}{}", input, "=".repeat(padding));
    
    URL_SAFE_NO_PAD
        .decode(&padded)
        .map_err(|e| crate::error::Error::InvalidToken(format!("Base64 decode failed: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oidc_provider_creation() {
        let mut config_map = HashMap::new();
        config_map.insert("issuer".to_string(), serde_json::json!("https://accounts.google.com"));
        config_map.insert("jwks_uri".to_string(), serde_json::json!("https://www.googleapis.com/oauth2/v3/certs"));
        config_map.insert("audience".to_string(), serde_json::json!("stellar-operator"));

        let config = ProviderConfig {
            name: "google".to_string(),
            provider_type: ProviderType::Oidc,
            config: config_map,
            enabled: true,
            priority: 1,
        };

        let provider = OidcProvider::new(config);
        assert_eq!(provider.name(), "google");
        assert_eq!(provider.provider_type(), ProviderType::Oidc);
    }
}
