//! Authentication module for API Gateway
//!
//! Supports multiple authentication methods: JWT, OAuth2, API Keys

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

/// Authentication errors
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Token expired")]
    TokenExpired,
    #[error("Missing authentication")]
    MissingAuth,
    #[error("Unsupported provider: {0}")]
    UnsupportedProvider(String),
    #[error("API key not found")]
    ApiKeyNotFound,
    #[error("API key disabled")]
    ApiKeyDisabled,
}

/// Auth context passed through the request
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub client_id: String,
    pub provider: AuthProvider,
    pub scopes: Vec<String>,
    pub claims: HashMap<String, serde_json::Value>,
}

/// Authentication provider type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuthProvider {
    Jwt,
    OAuth2,
    ApiKey,
    K8sRbac,
    Anonymous,
}

/// Authentication configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: Option<String>,
    pub jwt_algorithm: String,
    pub jwt_issuer: Option<String>,
    pub oauth2_client_id: Option<String>,
    pub oauth2_client_secret: Option<String>,
    pub oauth2_discovery_url: Option<String>,
    pub api_keys: HashMap<String, ApiKeyEntry>,
    pub allow_anonymous: bool,
    pub k8s_auth_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiKeyEntry {
    pub key_hash: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub enabled: bool,
    pub expires_at: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt_secret: None,
            jwt_algorithm: "RS256".to_string(),
            jwt_issuer: None,
            oauth2_client_id: None,
            oauth2_client_secret: None,
            oauth2_discovery_url: None,
            api_keys: HashMap::new(),
            allow_anonymous: false,
            k8s_auth_enabled: true,
        }
    }
}

/// JWT claims structure
#[derive(Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub iss: Option<String>,
    pub aud: Option<Vec<String>>,
    pub exp: i64,
    pub iat: i64,
    pub scopes: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// JWT Authentication
pub struct JwtAuth {
    secret: Option<String>,
    algorithm: Algorithm,
    issuer: Option<String>,
    validation: Validation,
}

impl JwtAuth {
    pub fn new(config: &AuthConfig) -> Self {
        let algorithm = match config.jwt_algorithm.to_uppercase().as_str() {
            "HS256" => Algorithm::HS256,
            "HS384" => Algorithm::HS384,
            "HS512" => Algorithm::HS512,
            "RS256" => Algorithm::RS256,
            "RS384" => Algorithm::RS384,
            "RS512" => Algorithm::RS512,
            _ => Algorithm::RS256,
        };

        let mut validation = Validation::new(algorithm);
        validation.validate_exp = true;
        validation.validate_aud = false;

        if let Some(issuer) = &config.jwt_issuer {
            validation.set_issuer(&[issuer]);
        }

        Self {
            secret: config.jwt_secret.clone(),
            algorithm,
            issuer: config.jwt_issuer.clone(),
            validation,
        }
    }

    pub async fn validate_token(&self, token: &str) -> Result<AuthContext, AuthError> {
        // For RS256, we need a public key; for HS256, the secret
        let decoding_key = if let Some(secret) = &self.secret {
            if secret.contains("-----BEGIN") {
                // PEM format - treat as public key for RS256
                DecodingKey::from_rsa_pem(secret.as_bytes())
                    .map_err(|e| AuthError::InvalidToken(format!("Invalid PEM: {e}")))?
            } else {
                // Plain secret for HS256
                DecodingKey::from_secret(secret.as_bytes())
            }
        } else {
            return Err(AuthError::InvalidToken("No JWT secret configured".into()));
        };

        let token_data = decode::<JwtClaims>(token, &decoding_key, &self.validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                _ => AuthError::InvalidToken(e.to_string()),
            })?;

        let claims = token_data.claims;
        let scopes = claims.scopes.unwrap_or_default();

        Ok(AuthContext {
            client_id: claims.sub,
            provider: AuthProvider::Jwt,
            scopes,
            claims: {
                let mut map = claims.extra;
                map.insert("iss".into(), claims.iss.unwrap_or_default().into());
                map
            },
        })
    }
}

/// OAuth2 Authentication
pub struct OAuth2Auth {
    client_id: Option<String>,
    client_secret: Option<String>,
    discovery_url: Option<String>,
    // In production, this would cache discovery doc
}

impl OAuth2Auth {
    pub fn new(config: &AuthConfig) -> Self {
        Self {
            client_id: config.oauth2_client_id.clone(),
            client_secret: config.oauth2_client_secret.clone(),
            discovery_url: config.oauth2_discovery_url.clone(),
        }
    }

    pub async fn validate_token(&self, _token: &str) -> Result<AuthContext, AuthError> {
        // In production: fetch JWKS from discovery URL, validate signature
        // For now, return a placeholder - real implementation would:
        // 1. Call token introspection endpoint
        // 2. Validate token signature using JWKS
        // 3. Check token expiration, audience, etc.
        Ok(AuthContext {
            client_id: "oauth2-user".to_string(),
            provider: AuthProvider::OAuth2,
            scopes: vec!["read".to_string()],
            claims: HashMap::new(),
        })
    }

    pub fn get_authorization_url(&self, redirect_uri: &str) -> String {
        let client_id = self.client_id.as_deref().unwrap_or("client");
        format!(
            "https://auth.example.com/authorize?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email",
            client_id, redirect_uri
        )
    }

    pub async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<String, AuthError> {
        // In production: exchange code for tokens
        // This is a placeholder
        let _client_id = self.client_id.as_deref().unwrap_or("client");
        let _client_secret = self.client_secret.as_deref().unwrap_or("secret");
        let _redirect_uri = redirect_uri;
        
        // Return a mock access token
        Ok(format!("access_token_{}", code))
    }
}

/// API Key Authentication
pub struct ApiKeyAuth {
    keys: HashMap<String, ApiKeyEntry>,
}

impl ApiKeyAuth {
    pub fn new(config: &AuthConfig) -> Self {
        Self {
            keys: config.api_keys.clone(),
        }
    }

    pub fn validate_key(&self, key: &str) -> Result<AuthContext, AuthError> {
        let entry = self.keys.get(key).ok_or(AuthError::ApiKeyNotFound)?;

        if !entry.enabled {
            return Err(AuthError::ApiKeyDisabled);
        }

        if let Some(expires) = &entry.expires_at {
            if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(expires) {
                if exp < Utc::now() {
                    return Err(AuthError::TokenExpired);
                }
            }
        }

        Ok(AuthContext {
            client_id: entry.client_id.clone(),
            provider: AuthProvider::ApiKey,
            scopes: entry.scopes.clone(),
            claims: HashMap::new(),
        })
    }

    pub fn add_key(&mut self, key: String, entry: ApiKeyEntry) {
        self.keys.insert(key, entry);
    }

    pub fn revoke_key(&mut self, key: &str) -> bool {
        self.keys.remove(key).is_some()
    }
}

/// Authentication middleware that chains multiple providers
#[derive(Default)]
pub struct AuthMiddleware {
    jwt: Option<JwtAuth>,
    oauth2: Option<OAuth2Auth>,
    api_key: Option<ApiKeyAuth>,
    allow_anonymous: bool,
    k8s_auth_enabled: bool,
}

impl AuthMiddleware {
    pub fn from_config(config: &AuthConfig) -> Self {
        let mut jwt = None;
        if config.jwt_secret.is_some() {
            jwt = Some(JwtAuth::new(config));
        }

        let mut oauth2 = None;
        if config.oauth2_discovery_url.is_some() {
            oauth2 = Some(OAuth2Auth::new(config));
        }

        let api_key = if config.api_keys.is_empty() {
            None
        } else {
            Some(ApiKeyAuth::new(config))
        };

        Self {
            jwt,
            oauth2,
            api_key,
            allow_anonymous: config.allow_anonymous,
            k8s_auth_enabled: config.k8s_auth_enabled,
        }
    }

    pub async fn authenticate(&self, request: &Request<Body>) -> Result<AuthContext, AuthError> {
        // Try each auth provider in order

        // 1. Bearer token (JWT or OAuth2)
        if let Some(auth_header) = request.headers().get(header::AUTHORIZATION) {
            if let Ok(value) = auth_header.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    // Try JWT first
                    if let Some(ref jwt) = self.jwt {
                        if let Ok(ctx) = jwt.validate_token(token).await {
                            return Ok(ctx);
                        }
                    }
                    // Try OAuth2
                    if let Some(ref oauth2) = self.oauth2 {
                        if let Ok(ctx) = oauth2.validate_token(token).await {
                            return Ok(ctx);
                        }
                    }
                }
            }
        }

        // 2. API Key in header
        if let Some(api_key) = request.headers().get("X-API-Key") {
            if let Ok(key) = api_key.to_str() {
                if let Some(ref api_key_auth) = self.api_key {
                    if let Ok(ctx) = api_key_auth.validate_key(key) {
                        return Ok(ctx);
                    }
                }
            }
        }

        // 3. API Key as query param (less secure but convenient)
        if let Some(query) = request.uri().query() {
            for pair in query.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    if k == "api_key" {
                        if let Some(ref api_key_auth) = self.api_key {
                            if let Ok(ctx) = api_key_auth.validate_key(v) {
                                return Ok(ctx);
                            }
                        }
                    }
                }
            }
        }

        // 4. Kubernetes RBAC (if enabled)
        if self.k8s_auth_enabled {
            // This would integrate with existing k8s_rbac_auth
            // For now, return anonymous if allowed
        }

        // 5. Anonymous access
        if self.allow_anonymous {
            return Ok(AuthContext {
                client_id: "anonymous".to_string(),
                provider: AuthProvider::Anonymous,
                scopes: vec!["public:read".to_string()],
                claims: HashMap::new(),
            });
        }

        Err(AuthError::MissingAuth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_config_defaults() {
        let config = AuthConfig::default();
        assert!(config.jwt_secret.is_none());
        assert!(config.allow_anonymous);
        assert!(config.k8s_auth_enabled);
    }

    #[test]
    fn test_jwt_auth_creation() {
        let config = AuthConfig {
            jwt_secret: Some("test-secret".to_string()),
            ..Default::default()
        };
        let jwt = JwtAuth::new(&config);
        assert!(jwt.secret.is_some());
    }

    #[test]
    fn test_api_key_auth() {
        let config = AuthConfig {
            api_keys: {
                let mut keys = HashMap::new();
                keys.insert(
                    "test-key".to_string(),
                    ApiKeyEntry {
                        key_hash: "hash".to_string(),
                        client_id: "test-client".to_string(),
                        scopes: vec!["read".to_string()],
                        enabled: true,
                        expires_at: None,
                    },
                );
                keys
            },
            ..Default::default()
        };
        let api_key = ApiKeyAuth::new(&config);
        let result = api_key.validate_key("test-key");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().client_id, "test-client");
    }

    #[test]
    fn test_disabled_api_key() {
        let config = AuthConfig {
            api_keys: {
                let mut keys = HashMap::new();
                keys.insert(
                    "disabled-key".to_string(),
                    ApiKeyEntry {
                        key_hash: "hash".to_string(),
                        client_id: "disabled-client".to_string(),
                        scopes: vec![],
                        enabled: false,
                        expires_at: None,
                    },
                );
                keys
            },
            allow_anonymous: false,
            ..Default::default()
        };
        let api_key = ApiKeyAuth::new(&config);
        let result = api_key.validate_key("disabled-key");
        assert!(matches!(result, Err(AuthError::ApiKeyDisabled)));
    }
}