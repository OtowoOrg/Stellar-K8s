// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Graceful degradation helpers for partial failure scenarios
//!
//! When a request can partially succeed (e.g. listing nodes where one namespace's
//! API call fails), return `degraded: true` with `ERR_PARTIAL_DEGRADATION` and HTTP 207
//! (Multi-Status) instead of 500. This lets callers distinguish total vs partial failure.

use axum::{http::StatusCode, Json};
use serde_json::json;

use crate::rest_api::dto::{ApiErrorCode, ErrorResponse};

/// Context describing which sub-operations failed during a degraded response
#[derive(Debug, Clone)]
pub struct DegradationContext {
    pub failed: Vec<String>,
    pub succeeded: Vec<String>,
    pub message: String,
}

impl DegradationContext {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            failed: Vec::new(),
            succeeded: Vec::new(),
            message: message.into(),
        }
    }

    pub fn with_failed(mut self, items: Vec<String>) -> Self {
        self.failed = items;
        self
    }

    pub fn with_succeeded(mut self, items: Vec<String>) -> Self {
        self.succeeded = items;
        self
    }

    pub fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }
}

/// Build a degraded JSON error response with 207 Multi-Status
pub fn degraded_response(
    ctx: DegradationContext,
    correlation_id: Option<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    let details = json!({
        "failed": ctx.failed,
        "succeeded": ctx.succeeded,
        "degraded": true,
    });
    (
        StatusCode::MULTI_STATUS, // 207
        Json(ErrorResponse::degraded(
            ApiErrorCode::ErrPartialDegradation,
            &ctx.message,
            details,
            correlation_id,
        )),
    )
}

/// Map `crate::Error` variants to (HTTP status, ApiErrorCode) for consistent API errors
pub fn map_error_to_api_code(err: &crate::Error) -> (StatusCode, ApiErrorCode) {
    use crate::Error;
    match err {
        Error::NotFound { .. } => (StatusCode::NOT_FOUND, ApiErrorCode::ErrNotFound),
        Error::ValidationError(_) | Error::InvalidNodeType(_) | Error::MissingRequiredField { .. } => {
            (StatusCode::BAD_REQUEST, ApiErrorCode::ErrBadRequest)
        }
        Error::ConfigError(_) | Error::CertificateError(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::ErrServiceUnavailable)
        }
        Error::KubeError(_) | Error::KubeconfigError(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::ErrServiceUnavailable)
        }
        Error::FinalizerError(_) | Error::RemediationError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, ApiErrorCode::ErrInternalServerError)
        }
        Error::NetworkSafetyViolation(_) => (StatusCode::FORBIDDEN, ApiErrorCode::ErrForbidden),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, ApiErrorCode::ErrInternalServerError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_response_has_207_and_flag() {
        let ctx = DegradationContext::new("partial failure")
            .with_failed(vec!["ns/a".to_string()])
            .with_succeeded(vec!["ns/b".to_string()]);
        let (code, Json(body)) = degraded_response(ctx, Some("req-1".to_string()));
        assert_eq!(code, StatusCode::MULTI_STATUS);
        assert!(body.degraded);
        assert_eq!(body.error_code, "ERR_PARTIAL_DEGRADATION");
        assert_eq!(body.correlation_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn map_not_found() {
        let err = crate::Error::NotFound {
            kind: "Pod".to_string(),
            name: "x".to_string(),
            namespace: "default".to_string(),
        };
        let (code, api) = map_error_to_api_code(&err);
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert_eq!(api, ApiErrorCode::ErrNotFound);
    }
}
