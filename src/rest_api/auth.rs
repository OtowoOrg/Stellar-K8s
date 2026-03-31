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
use crate::controller::ControllerState;

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
        Ok(true) => {
            debug!("Token validated successfully");
            Ok(next.run(request).await)
        }
        Ok(false) => {
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
async fn validate_k8s_token(state: &ControllerState, token: &str) -> Result<bool, kube::Error> {
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

    Ok(result.status.and_then(|s| s.authenticated).unwrap_or(false))
}

/// Check if user has required permissions using SubjectAccessReview
pub async fn check_rbac_permission(
    state: &ControllerState,
    _token: &str,
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
