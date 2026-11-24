//! Prometheus alert expression validation endpoint
//!
//! Backs the frontend Alert Rule Builder's "Test against Prometheus" button.
//! Accepts a raw PromQL expression, executes it as an instant query against
//! the configured Prometheus instance, and reports back whether the syntax
//! is valid and whether the condition currently evaluates true.

use axum::{extract::State, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

use crate::controller::ControllerState;

/// Request body: the raw PromQL expression to test.
#[derive(Deserialize, Debug)]
pub struct AlertTestRequest {
    pub expr: String,
}

/// Response body returned to the frontend.
#[derive(Serialize, Debug)]
pub struct AlertTestResponse {
    pub valid: bool,
    #[serde(rename = "currentlyFiring")]
    pub currently_firing: bool,
    #[serde(rename = "sampleCount")]
    pub sample_count: usize,
    pub message: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct AlertTestError {
    pub message: String,
}

/// Prometheus's instant query API response shape (subset we care about).
#[derive(Deserialize, Debug)]
struct PromQueryResponse {
    status: String,
    data: Option<PromQueryData>,
    #[serde(rename = "errorType")]
    error_type: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize, Debug)]
struct PromQueryData {
    #[serde(rename = "resultType")]
    #[allow(dead_code)]
    result_type: String,
    result: Vec<serde_json::Value>,
}

/// Resolve the Prometheus base URL, in priority order:
/// 1. `PROMETHEUS_URL` env var (explicit override)
/// 2. Default in-cluster kube-prometheus-stack service DNS name
fn prometheus_base_url() -> String {
    std::env::var("PROMETHEUS_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://prometheus-operated.monitoring.svc:9090".to_string())
}

/// `POST /api/v1/alerts/test`
///
/// Validates a PromQL expression by running it as an instant query against
/// the configured Prometheus instance. Does not require or use `ControllerState`
/// directly today, but takes it via `State` for consistency with other handlers
/// and to allow future per-cluster Prometheus routing.
#[tracing::instrument(skip(_state, payload), fields(expr_len = payload.expr.len()))]
pub async fn test_alert_expr(
    State(_state): State<Arc<ControllerState>>,
    Json(payload): Json<AlertTestRequest>,
) -> Response {
    let expr = payload.expr.trim();

    if expr.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(AlertTestError {
                message: "Expression must not be empty".to_string(),
            }),
        )
            .into_response();
    }

    let base_url = prometheus_base_url();
    let query_url = format!("{}/api/v1/query", base_url.trim_end_matches('/'));

    debug!("Testing PromQL expression against {}: {}", query_url, expr);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            warn!("Failed to build reqwest client: {}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(AlertTestError {
                    message: format!("Internal error constructing HTTP client: {err}"),
                }),
            )
                .into_response();
        }
    };

    let response = match client
        .get(&query_url)
        .query(&[("query", expr)])
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            warn!("Failed to reach Prometheus at {}: {}", query_url, err);
            return (
                StatusCode::BAD_GATEWAY,
                Json(AlertTestError {
                    message: format!("Could not reach Prometheus at {base_url}: {err}"),
                }),
            )
                .into_response();
        }
    };

    let status = response.status();
    let body: PromQueryResponse = match response.json().await {
        Ok(b) => b,
        Err(err) => {
            warn!("Failed to parse Prometheus response: {}", err);
            return (
                StatusCode::BAD_GATEWAY,
                Json(AlertTestError {
                    message: format!("Prometheus returned an unexpected response format: {err}"),
                }),
            )
                .into_response();
        }
    };

    if body.status != "success" {
        let reason = body
            .error
            .unwrap_or_else(|| "Unknown error from Prometheus".to_string());
        let error_type = body.error_type.unwrap_or_else(|| "invalid_query".to_string());
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(AlertTestError {
                message: format!("Invalid PromQL ({error_type}): {reason}"),
            }),
        )
            .into_response();
    }

    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(AlertTestError {
                message: format!("Prometheus responded with HTTP {status}"),
            }),
        )
            .into_response();
    }

    let sample_count = body.data.as_ref().map(|d| d.result.len()).unwrap_or(0);

    Json(AlertTestResponse {
        valid: true,
        currently_firing: sample_count > 0,
        sample_count,
        message: None,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prometheus_base_url_defaults_to_in_cluster_service() {
        std::env::remove_var("PROMETHEUS_URL");
        assert_eq!(
            prometheus_base_url(),
            "http://prometheus-operated.monitoring.svc:9090"
        );
    }

    #[test]
    fn test_prometheus_base_url_respects_env_override() {
        std::env::set_var("PROMETHEUS_URL", "http://custom-prom:9091");
        assert_eq!(prometheus_base_url(), "http://custom-prom:9091");
        std::env::remove_var("PROMETHEUS_URL");
    }

    #[test]
    fn test_prometheus_base_url_ignores_empty_env_value() {
        std::env::set_var("PROMETHEUS_URL", "");
        assert_eq!(
            prometheus_base_url(),
            "http://prometheus-operated.monitoring.svc:9090"
        );
        std::env::remove_var("PROMETHEUS_URL");
    }

    #[test]
    fn test_alert_test_request_deserializes() {
        let json = r#"{"expr": "up == 0"}"#;
        let req: AlertTestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.expr, "up == 0");
    }

    #[test]
    fn test_alert_test_response_serializes_with_camel_case_fields() {
        let resp = AlertTestResponse {
            valid: true,
            currently_firing: true,
            sample_count: 2,
            message: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"currentlyFiring\":true"));
        assert!(json.contains("\"sampleCount\":2"));
        assert!(json.contains("\"valid\":true"));
    }
}
