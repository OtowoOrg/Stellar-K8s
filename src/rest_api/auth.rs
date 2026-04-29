//! Authentication and Authorization for Dashboard
//!
//! Supports Kubernetes RBAC via ServiceAccount tokens and optional OIDC

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use std::sync::Arc;
use tracing::{debug, warn};

use super::dto::ErrorResponse;
use super::oidc::ApiRole;
use crate::controller::ControllerState;
use crate::rest_api::oidc;

#[derive(Clone, Debug)]
pub struct RequestIdentity {
    pub subject: String,
    pub roles: Vec<ApiRole>,
    pub auth_type: String,
    pub groups: Vec<String>,
}

/// Extract bearer token from Authorization header
fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Kubernetes RBAC authentication middleware
///
/// Validates ServiceAccount tokens using TokenReview API
#[tracing::instrument(
    skip(state, headers, request, next),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
pub async fn k8s_rbac_auth(
    State(state): State<Arc<ControllerState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    // Extract token from Authorization header
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            warn!("Missing Authorization header");
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::new(
                    "unauthorized",
                    "Missing Authorization header",
                )),
            ));
        }
    };

    // Validate token using Kubernetes TokenReview API
    match validate_k8s_token(&state, &token).await {
        Ok(auth) if auth.authenticated => {
            debug!("Token validated successfully");
            Ok(next.run(request).await)
        }
        Ok(_) => {
            warn!("Token validation failed");
            Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse::new("forbidden", "Invalid token")),
            ))
        }
        Err(e) => {
            warn!("Token validation error: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "validation_error",
                    &format!("Token validation error: {e}"),
                )),
            ))
        }
    }
}

/// Validate Kubernetes ServiceAccount token using TokenReview API
#[derive(Debug, Clone)]
struct K8sAuthResult {
    authenticated: bool,
    username: Option<String>,
    groups: Vec<String>,
}

async fn validate_k8s_token(
    state: &ControllerState,
    token: &str,
) -> Result<K8sAuthResult, kube::Error> {
    use k8s_openapi::api::authentication::v1::TokenReview;
    use kube::api::{Api, PostParams};

    let api: Api<TokenReview> = Api::all(state.client.clone());

    let token_review = serde_json::json!({
        "apiVersion": "authentication.k8s.io/v1",
        "kind": "TokenReview",
        "spec": {
            "token": token
        }
    });

    let review: TokenReview =
        serde_json::from_value(token_review).map_err(kube::Error::SerdeError)?;

    let result = api.create(&PostParams::default(), &review).await?;

    let status = result.status;
    Ok(K8sAuthResult {
        authenticated: status.as_ref().and_then(|s| s.authenticated).unwrap_or(false),
        username: status.as_ref().and_then(|s| s.user.clone()).map(|u| u.username),
        groups: status
            .as_ref()
            .and_then(|s| s.user.clone())
            .map(|u| u.groups)
            .unwrap_or_default(),
    })
}

/// Check if user has required permissions using SubjectAccessReview
pub async fn check_rbac_permission(
    state: &ControllerState,
    user: &str,
    groups: &[String],
    namespace: &str,
    verb: &str,
    resource: &str,
) -> Result<bool, kube::Error> {
    use k8s_openapi::api::authorization::v1::SubjectAccessReview;
    use kube::api::{Api, PostParams};

    let api: Api<SubjectAccessReview> = Api::all(state.client.clone());

    let sar = serde_json::json!({
        "apiVersion": "authorization.k8s.io/v1",
        "kind": "SubjectAccessReview",
        "spec": {
            "user": user,
            "groups": groups,
            "resourceAttributes": {
                "namespace": namespace,
                "verb": verb,
                "group": "stellar.org",
                "resource": resource
            }
        }
    });

    let review: SubjectAccessReview =
        serde_json::from_value(sar).map_err(kube::Error::SerdeError)?;

    let result = api.create(&PostParams::default(), &review).await?;

    Ok(result.status.map(|s| s.allowed).unwrap_or(false))
}


use crate::crd::OperatorRole;

/// Unified API auth middleware for read-only access.
pub async fn api_reader(
    State(state): State<Arc<ControllerState>>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let token = extract_bearer_token(&headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new(
                "unauthorized",
                "Missing Authorization header",
            )),
        )
    })?;

    let mut subject = "system:unknown".to_string();
    let mut groups: Vec<String> = Vec::new();
    let mut roles: Vec<ApiRole> = Vec::new();
    let mut op_roles: Vec<OperatorRole> = Vec::new();
    let mut auth_type = "k8s".to_string();

    if let Some(oidc_config) = state.oidc_config.as_ref() {
        let (oidc_roles, oidc_subject) = oidc::validate_jwt_with_subject(&token, oidc_config)
            .map_err(|e| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse::new("unauthorized", &e)),
                )
            })?;

        if oidc_roles.is_empty() {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse::new("forbidden", "Reader role required")),
            ));
        }

        roles = oidc_roles;
        subject = oidc_subject;
        auth_type = "oidc".to_string();
        op_roles.push(OperatorRole::Viewer);
    } else {
        let auth = validate_k8s_token(&state, &token)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        "validation_error",
                        &format!("Token validation error: {e}"),
                    )),
                )
            })?;

        if !auth.authenticated {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse::new("forbidden", "Invalid token")),
            ));
        }

        subject = auth.username.unwrap_or_else(|| "system:unknown".to_string());
        groups = auth.groups;
        roles.push(ApiRole::Reader);
        op_roles.push(OperatorRole::Viewer);

        let namespace = extract_namespace(&request)
            .unwrap_or_else(|| state.operator_namespace.clone());
        
        // Fine-grained RBAC check using custom verbs
        if check_rbac_permission(
            &state,
            &subject,
            &groups,
            &namespace,
            "admin",
            "stellarnodes",
        )
        .await
        .unwrap_or(false)
        {
            roles.push(ApiRole::Admin);
            op_roles.push(OperatorRole::SuperAdmin);
        } else if check_rbac_permission(
            &state,
            &subject,
            &groups,
            &namespace,
            "operate",
            "stellarnodes",
        )
        .await
        .unwrap_or(false)
        {
            op_roles.push(OperatorRole::Operator);
        } else if check_rbac_permission(
            &state,
            &subject,
            &groups,
            &namespace,
            "audit",
            "stellarnodes",
        )
        .await
        .unwrap_or(false)
        {
            op_roles.push(OperatorRole::Auditor);
        }
    }

    // OPA Policy Check
    let verb = match request.method().as_str() {
        "GET" => "read",
        "POST" | "PUT" | "PATCH" => "write",
        "DELETE" => "delete",
        _ => "other",
    };

    if let Err(e) = check_opa_policy(&state, &subject, verb, request.uri().path()).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new("forbidden", &format!("OPA Policy Denied: {e}"))),
        ));
    }

    request.extensions_mut().insert(roles.clone());
    request.extensions_mut().insert(op_roles);
    request.extensions_mut().insert(RequestIdentity {
        subject,
        roles,
        auth_type,
        groups,
    });

    Ok(next.run(request).await)
}

async fn check_opa_policy(
    state: &ControllerState,
    user: &str,
    action: &str,
    resource: &str,
) -> Result<(), String> {
    // In a real implementation, this would call the OPA endpoint configured in PolicyConfig
    debug!(user = %user, action = %action, resource = %resource, "Checking OPA policy");
    Ok(())
}

/// Enforce Admin role after api_reader has run.
pub async fn api_admin(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let identity = request.extensions().get::<RequestIdentity>().cloned();
    let roles = identity
        .as_ref()
        .map(|i| i.roles.clone())
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

fn extract_namespace(request: &Request) -> Option<String> {
    let path = request.uri().path();
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 4 && parts[0] == "api" && parts[1] == "v1" && parts[2] == "nodes" {
        return Some(parts[3].to_string());
    }
    if parts.len() >= 5
        && parts[0] == "api"
        && parts[1] == "v1"
        && parts[2] == "dashboard"
        && parts[3] == "nodes"
    {
        return Some(parts[4].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer test-token-123".parse().unwrap());

        let token = extract_bearer_token(&headers);
        assert_eq!(token, Some("test-token-123".to_string()));
    }

    #[test]
    fn test_extract_bearer_token_missing() {
        let headers = HeaderMap::new();
        let token = extract_bearer_token(&headers);
        assert_eq!(token, None);
    }

    #[test]
    fn test_extract_bearer_token_invalid_format() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Basic dXNlcjpwYXNz".parse().unwrap());

        let token = extract_bearer_token(&headers);
        assert_eq!(token, None);
    }
}
