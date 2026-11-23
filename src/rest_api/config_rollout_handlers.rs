// Copyright 2026 Stellar-K8s Contributors
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
//! REST API for progressive operator config rollouts (#1998).
//!
//! # Endpoints
//!
//! | Method | Path                                         | Description |
//! |--------|----------------------------------------------|-------------|
//! | GET    | `/api/v1/config/rollouts`                    | Every rollout: stage, blast radius, canary set, failing gates, last rollback. |
//! | GET    | `/api/v1/config/rollouts/:id`                | One rollout. |
//! | GET    | `/api/v1/config/versions`                    | The config version each component runs. Optional `status` filter: `canary`, `restored`, `promoted`. |
//! | GET    | `/api/v1/config/versions/:namespace/:node`   | The config version of one component. |
//!
//! ```bash
//! curl https://operator:9090/api/v1/config/versions/stellar/validator-1
//! curl 'https://operator:9090/api/v1/config/versions?status=canary'
//! ```

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::config_rollout_registry::{self as registry, ConfigRolloutSnapshot};
use crate::progressive_config::ComponentConfigVersion;

use super::dto::{ApiErrorCode, ErrorResponse};

type ApiError = (StatusCode, Json<ErrorResponse>);

/// Query parameters for `GET /api/v1/config/versions`.
#[derive(Debug, Deserialize)]
pub struct VersionQuery {
    /// Filter by version status (`canary`, `restored`, `promoted`).
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RolloutListResponse {
    pub items: Vec<ConfigRolloutSnapshot>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct VersionListResponse {
    pub items: Vec<ComponentConfigVersion>,
    pub total: usize,
}

fn not_found(message: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::structured(
            ApiErrorCode::ErrNotFound,
            message,
            None,
        )),
    )
}

const STATUSES: [&str; 3] = ["canary", "restored", "promoted"];

/// `GET /api/v1/config/rollouts`
pub async fn list_rollouts() -> Json<RolloutListResponse> {
    let items = registry::global().list();
    let total = items.len();
    Json(RolloutListResponse { items, total })
}

/// `GET /api/v1/config/rollouts/:id`
pub async fn get_rollout(Path(id): Path<String>) -> Result<Json<ConfigRolloutSnapshot>, ApiError> {
    registry::global()
        .get(&id)
        .map(Json)
        .ok_or_else(|| not_found(&format!("no config rollout named {id}")))
}

/// Versions matching an optional status filter, ordered by `namespace/node`.
fn filter_versions(
    versions: Vec<ComponentConfigVersion>,
    status: Option<&str>,
) -> Vec<ComponentConfigVersion> {
    match status {
        Some(status) => versions
            .into_iter()
            .filter(|v| v.status.to_string() == status)
            .collect(),
        None => versions,
    }
}

/// `GET /api/v1/config/versions`
pub async fn list_versions(
    Query(query): Query<VersionQuery>,
) -> Result<Json<VersionListResponse>, ApiError> {
    if let Some(status) = query.status.as_deref() {
        if !STATUSES.contains(&status) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::structured(
                    ApiErrorCode::ErrBadRequest,
                    &format!("status must be one of {}", STATUSES.join(", ")),
                    None,
                )),
            ));
        }
    }
    let all: Vec<ComponentConfigVersion> = registry::global()
        .component_versions()
        .into_values()
        .collect();
    let items = filter_versions(all, query.status.as_deref());
    let total = items.len();
    Ok(Json(VersionListResponse { items, total }))
}

/// `GET /api/v1/config/versions/:namespace/:node`
pub async fn get_version(
    Path((namespace, node)): Path<(String, String)>,
) -> Result<Json<ComponentConfigVersion>, ApiError> {
    registry::global()
        .component_version(&namespace, &node)
        .map(Json)
        .ok_or_else(|| {
            not_found(&format!(
                "no config version recorded for component {namespace}/{node}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progressive_config::VersionStatus;
    use chrono::Utc;

    fn version(node: &str, status: VersionStatus) -> ComponentConfigVersion {
        ComponentConfigVersion {
            component: "stellar-core".to_string(),
            namespace: "ns".to_string(),
            node: node.to_string(),
            version: "1.0.0".to_string(),
            digest: "abc".to_string(),
            status,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn status_filter_keeps_only_matching_versions() {
        let all = vec![
            version("a", VersionStatus::Canary),
            version("b", VersionStatus::Promoted),
            version("c", VersionStatus::Canary),
        ];
        let canary = filter_versions(all.clone(), Some("canary"));
        assert_eq!(canary.len(), 2);
        assert!(canary.iter().all(|v| v.status == VersionStatus::Canary));
        assert_eq!(filter_versions(all, None).len(), 3);
    }

    #[test]
    fn the_known_statuses_match_the_engine_display_names() {
        for status in [
            VersionStatus::Canary,
            VersionStatus::Restored,
            VersionStatus::Promoted,
        ] {
            assert!(STATUSES.contains(&status.to_string().as_str()));
        }
    }

    #[tokio::test]
    async fn unknown_rollout_and_component_are_404() {
        let rollout = get_rollout(Path("no-such-rollout".to_string())).await;
        assert_eq!(rollout.unwrap_err().0, StatusCode::NOT_FOUND);

        let component = get_version(Path(("no-ns".to_string(), "no-node".to_string()))).await;
        assert_eq!(component.unwrap_err().0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_unknown_status_filter_is_rejected() {
        let result = list_versions(Query(VersionQuery {
            status: Some("bogus".to_string()),
        }))
        .await;
        assert_eq!(result.unwrap_err().0, StatusCode::BAD_REQUEST);
    }
}
