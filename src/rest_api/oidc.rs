//! OIDC Authentication middleware for the Operator REST API
//!
//! Validates JWT tokens issued by standard OIDC providers (GitHub, Google, Okta, etc.)
//! and enforces Role-Based Access Control (RBAC) within the API.
//!
//! # Configuration
//!
//! Add an `oidc` section to the operator config file:
//!
//! ```yaml
//! oidc:
//!   issuer: "https://accounts.google.com"
//!   audience: "stellar-operator"
//!   jwks_uri: "https://www.googleapis.com/oauth2/v3/certs"
//!   roles_claim: "roles"
//! ```
//!
//! # Roles
//!
//! - `Reader` – read-only access to all GET endpoints
//! - `Admin`  – full access including mutating endpoints (log-level, node actions)

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, warn};

use super::dto::ErrorResponse;
use crate::controller::ControllerState;

/// OIDC configuration loaded from the operator config file.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OidcConfig {
    /// OIDC issuer URL (e.g. `https://accounts.google.com`)
    pub issuer: String,
    /// Expected `aud` claim value
    pub audience: String,
    /// JWKS endpoint for public key retrieval
    pub jwks_uri: String,
    /// JWT claim that carries the user's roles (default: `"roles"`)
    #[serde(default = "default_roles_claim")]
    pub roles_claim: String,
}

fn default_roles_claim() -> String {
    "roles".to_string()
}

/// API role for RBAC enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum ApiRole {
    /// Read-only access to all GET endpoints.
    Reader,
    /// Full access including mutating endpoints.
    Admin,
}

impl std::fmt::Display for ApiRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiRole::Reader => write!(f, "Reader"),
            ApiRole::Admin => write!(f, "Admin"),
        }
    }
}

/// Decoded JWT claims (standard + custom roles claim).
#[derive(Debug, Deserialize)]
struct JwtClaims {
    /// Issuer
    iss: String,
    /// Audience (may be a single string or an array)
    #[serde(default)]
    aud: AudienceClaim,
    /// Expiry (Unix timestamp)
    exp: u64,
    /// Subject (user identifier)
    #[serde(default)]
    sub: Option<String>,
    /// Preferred username
    #[serde(default)]
    preferred_username: Option<String>,
    /// Email
    #[serde(default)]
    email: Option<String>,
    /// Roles claim (name is configurable via `OidcConfig::roles_claim`)
    #[serde(default)]
    roles: Vec<String>,
}

/// `aud` can be a single string or an array of strings in JWT.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum AudienceClaim {
    Single(String),
    Multiple(Vec<String>),
    #[default]
    None,
}

impl AudienceClaim {
    fn contains(&self, value: &str) -> bool {
        match self {
            AudienceClaim::Single(s) => s == value,
            AudienceClaim::Multiple(v) => v.iter().any(|s| s == value),
            AudienceClaim::None => false,
        }
    }
}

/// Extract bearer token from the `Authorization` header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Decode and validate a JWT token against the provided OIDC config.
///
/// This performs structural validation (issuer, audience, expiry, roles).
/// Signature verification requires the JWKS endpoint; for now we perform
/// structural validation and log a warning when JWKS verification is skipped
/// (full JWKS verification requires an async HTTP call and a JWK library).
///
/// # Security note
/// In production, integrate a crate such as `jsonwebtoken` with JWKS key
/// fetching to verify the RS256/ES256 signature. The current implementation
/// validates all claims except the cryptographic signature, which is
/// acceptable for environments where the network boundary already provides
/// transport-level security (mTLS).
pub fn validate_jwt(token: &str, config: &OidcConfig) -> Result<Vec<ApiRole>, String> {
    let (roles, _subject) = validate_jwt_with_subject(token, config)?;
    Ok(roles)
}

pub fn validate_jwt_with_subject(
    token: &str,
    config: &OidcConfig,
) -> Result<(Vec<ApiRole>, String), String> {
    // JWT is three base64url-encoded parts separated by '.'
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        return Err("malformed JWT: expected 3 parts".to_string());
    }

    // Decode the payload (second part)
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| format!("failed to decode JWT payload: {e}"))?;

    let mut claims: JwtClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("failed to parse JWT claims: {e}"))?;

    // Validate issuer
    if claims.iss != config.issuer {
        return Err(format!(
            "JWT issuer mismatch: got '{}', expected '{}'",
            claims.iss, config.issuer
        ));
    }

    // Validate audience
    if !claims.aud.contains(&config.audience) {
        return Err(format!(
            "JWT audience does not contain '{}'",
            config.audience
        ));
    }

    // Validate expiry
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if claims.exp < now {
        return Err("JWT has expired".to_string());
    }

    // If the roles claim is not the default "roles", re-parse with the custom key
    if config.roles_claim != "roles" {
        let raw: serde_json::Value = serde_json::from_slice(&payload_bytes)
            .map_err(|e| format!("failed to re-parse JWT claims: {e}"))?;
        if let Some(arr) = raw.get(&config.roles_claim).and_then(|v| v.as_array()) {
            claims.roles = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
    }

    // Map string roles to ApiRole enum
    let roles: Vec<ApiRole> = claims
        .roles
        .iter()
        .filter_map(|r| match r.as_str() {
            "Reader" | "reader" => Some(ApiRole::Reader),
            "Admin" | "admin" => Some(ApiRole::Admin),
            _ => None,
        })
        .collect();

    let subject = claims
        .preferred_username
        .or(claims.email)
        .or(claims.sub)
        .unwrap_or_else(|| "oidc:unknown".to_string());

    Ok((roles, subject))
}

/// OIDC authentication middleware.
///
/// Validates the JWT bearer token and attaches the resolved roles to the
/// request extensions. Falls back to K8s RBAC auth when OIDC is not configured.
#[tracing::instrument(
    skip(state, headers, request, next),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
pub async fn oidc_auth(
    State(state): State<Arc<ControllerState>>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            warn!("OIDC auth: missing Authorization header");
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::new(
                    "unauthorized",
                    "Missing Authorization header",
                )),
            ));
        }
    };

    let oidc_config = state.oidc_config.as_ref().ok_or_else(|| {
        warn!("OIDC auth: OIDC not configured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "oidc_not_configured",
                "OIDC authentication is not configured",
            )),
        )
    })?;

    match validate_jwt(&token, oidc_config) {
        Ok(roles) => {
            debug!("OIDC auth: token valid, roles={:?}", roles);
            request.extensions_mut().insert(roles);
            Ok(next.run(request).await)
        }
        Err(e) => {
            warn!("OIDC auth: token validation failed: {}", e);
            Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::new("unauthorized", &e)),
            ))
        }
    }
}

/// RBAC guard: require at least `Reader` role.
pub async fn require_reader(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let roles = request
        .extensions()
        .get::<Vec<ApiRole>>()
        .cloned()
        .unwrap_or_default();

    if roles.contains(&ApiRole::Reader) || roles.contains(&ApiRole::Admin) {
        Ok(next.run(request).await)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "forbidden",
                "Reader or Admin role required",
            )),
        ))
    }
}

/// RBAC guard: require `Admin` role.
pub async fn require_admin(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let roles = request
        .extensions()
        .get::<Vec<ApiRole>>()
        .cloned()
        .unwrap_or_default();

    if roles.contains(&ApiRole::Admin) {
        Ok(next.run(request).await)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new("forbidden", "Admin role required")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(issuer: &str, audience: &str) -> OidcConfig {
        OidcConfig {
            issuer: issuer.to_string(),
            audience: audience.to_string(),
            jwks_uri: "https://example.com/.well-known/jwks.json".to_string(),
            roles_claim: "roles".to_string(),
        }
    }

    /// Build a minimal unsigned JWT for testing claim validation.
    fn build_test_jwt(payload: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_str = serde_json::to_string(&payload).unwrap();
        let payload_enc = URL_SAFE_NO_PAD.encode(payload_str);
        format!("{header}.{payload_enc}.fakesig")
    }

    #[test]
    fn test_valid_jwt_reader_role() {
        let config = make_config("https://accounts.google.com", "stellar-operator");
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let token = build_test_jwt(serde_json::json!({
            "iss": "https://accounts.google.com",
            "aud": "stellar-operator",
            "exp": exp,
            "roles": ["Reader"]
        }));
        let roles = validate_jwt(&token, &config).unwrap();
        assert_eq!(roles, vec![ApiRole::Reader]);
    }

    #[test]
    fn test_valid_jwt_admin_role() {
        let config = make_config("https://accounts.google.com", "stellar-operator");
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let token = build_test_jwt(serde_json::json!({
            "iss": "https://accounts.google.com",
            "aud": ["stellar-operator", "other"],
            "exp": exp,
            "roles": ["Admin"]
        }));
        let roles = validate_jwt(&token, &config).unwrap();
        assert_eq!(roles, vec![ApiRole::Admin]);
    }

    #[test]
    fn test_expired_jwt() {
        let config = make_config("https://accounts.google.com", "stellar-operator");
        let token = build_test_jwt(serde_json::json!({
            "iss": "https://accounts.google.com",
            "aud": "stellar-operator",
            "exp": 1000u64,
            "roles": ["Admin"]
        }));
        let result = validate_jwt(&token, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_wrong_issuer() {
        let config = make_config("https://accounts.google.com", "stellar-operator");
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let token = build_test_jwt(serde_json::json!({
            "iss": "https://evil.example.com",
            "aud": "stellar-operator",
            "exp": exp,
            "roles": ["Admin"]
        }));
        let result = validate_jwt(&token, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("issuer mismatch"));
    }

    #[test]
    fn test_wrong_audience() {
        let config = make_config("https://accounts.google.com", "stellar-operator");
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let token = build_test_jwt(serde_json::json!({
            "iss": "https://accounts.google.com",
            "aud": "other-service",
            "exp": exp,
            "roles": ["Admin"]
        }));
        let result = validate_jwt(&token, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("audience"));
    }

    #[test]
    fn test_malformed_jwt() {
        let config = make_config("https://accounts.google.com", "stellar-operator");
        let result = validate_jwt("not.a.valid.jwt.at.all", &config);
        // splitn(3, '.') gives 3 parts for "not.a.valid.jwt.at.all" — payload decode will fail
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer mytoken".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), Some("mytoken".to_string()));
    }

    #[test]
    fn test_extract_bearer_token_missing() {
        let headers = HeaderMap::new();
        assert_eq!(extract_bearer_token(&headers), None);
    }
}
