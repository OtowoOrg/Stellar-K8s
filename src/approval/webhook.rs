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
//! Admission webhook for enforcing multi-party approval on privileged changes
//!
//! This is a simplified implementation that uses raw JSON handling
//! to avoid complex generic type requirements.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::approval::crd::{Approval, ApprovalDecision, ApprovalState, Approver, ChangeRequest, ChangeRequestSpec, ChangeRequestStatus, ChangeRequestPhase, PrivilegedOperation, TargetReference};
use crate::error::{Error, Result};

/// Configuration for the approval webhook
#[derive(Clone, Debug)]
pub struct ApprovalWebhookConfig {
    /// Kubernetes client
    pub client: Client,
    /// Namespace where ChangeRequests are managed
    pub namespace: String,
    /// Default TTL for change requests
    pub default_ttl: Duration,
    /// Whether to fail open on webhook errors
    pub fail_open: bool,
}

/// Simplified admission request (subset of kube's AdmissionRequest)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleAdmissionRequest {
    pub uid: String,
    pub kind: SimpleGroupVersionKind,
    pub operation: String,
    pub name: String,
    pub namespace: Option<String>,
    pub object: Option<Value>,
    pub user_info: SimpleUserInfo,
}

/// Simplified group version kind
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleGroupVersionKind {
    pub group: String,
    pub version: String,
    pub kind: String,
}

/// Simplified user info
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleUserInfo {
    pub username: String,
    pub uid: Option<String>,
    pub groups: Vec<String>,
    pub extra: BTreeMap<String, Vec<String>>,
}

/// Simplified admission response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleAdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    pub status: Option<SimpleStatus>,
    pub warnings: Vec<String>,
}

/// Simplified status
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleStatus {
    pub code: i32,
    pub message: String,
}

/// Simplified admission review
#[derive(Debug, Deserialize)]
pub struct SimpleAdmissionReview {
    pub request: Option<SimpleAdmissionRequest>,
}

/// Webhook server state
pub struct ApprovalWebhook {
    config: ApprovalWebhookConfig,
    /// In-memory cache of change class -> required approvers mapping
    approver_cache: Arc<RwLock<BTreeMap<String, Vec<Approver>>>>,
}

impl ApprovalWebhook {
    /// Create a new approval webhook
    pub fn new(config: ApprovalWebhookConfig) -> Self {
        let mut cache = BTreeMap::new();
        // Default approvers for built-in change classes
        cache.insert(
            "ClusterConfig".to_string(),
            vec![
                Approver { identity: "cluster-admin".to_string(), name: Some("Cluster Admin".to_string()), required: true },
                Approver { identity: "security-team".to_string(), name: Some("Security Team".to_string()), required: true },
            ],
        );
        cache.insert(
            "SecurityPolicy".to_string(),
            vec![
                Approver { identity: "security-team".to_string(), name: Some("Security Team".to_string()), required: true },
                Approver { identity: "platform-team".to_string(), name: Some("Platform Team".to_string()), required: true },
            ],
        );
        cache.insert(
            "NamespaceManagement".to_string(),
            vec![
                Approver { identity: "platform-team".to_string(), name: Some("Platform Team".to_string()), required: true },
            ],
        );
        cache.insert(
            "NodeUpgrade".to_string(),
            vec![
                Approver { identity: "stellar-ops".to_string(), name: Some("Stellar Operations".to_string()), required: true },
                Approver { identity: "platform-team".to_string(), name: Some("Platform Team".to_string()), required: true },
            ],
        );
        cache.insert(
            "DisasterRecovery".to_string(),
            vec![
                Approver { identity: "incident-commander".to_string(), name: Some("Incident Commander".to_string()), required: true },
                Approver { identity: "platform-team".to_string(), name: Some("Platform Team".to_string()), required: true },
            ],
        );
        cache.insert(
            "CertificateRotation".to_string(),
            vec![
                Approver { identity: "security-team".to_string(), name: Some("Security Team".to_string()), required: true },
                Approver { identity: "platform-team".to_string(), name: Some("Platform Team".to_string()), required: true },
            ],
        );

        Self {
            config,
            approver_cache: Arc::new(RwLock::new(cache)),
        }
    }

    /// Build the axum router
    pub fn router(&self) -> Router {
        Router::new()
            .route("/validate", post(validate_admission))
            .route("/mutate", post(mutate_admission))
            .route("/healthz", axum::routing::get(health_check))
            .with_state(self.clone())
    }

    /// Get required approvers for a change class
    async fn get_required_approvers(&self, class: &str) -> Vec<Approver> {
        self.approver_cache.read().await
            .get(class)
            .cloned()
            .unwrap_or_default()
    }

    /// Check if a request requires approval
    async fn requires_approval(&self, request: &SimpleAdmissionRequest) -> Result<Option<ChangeRequestSpec>> {
        // Only check CREATE and UPDATE operations
        if !matches!(request.operation.as_str(), "CREATE" | "UPDATE" | "DELETE") {
            return Ok(None);
        }

        // Check if the resource is a privileged resource type
        let privileged_kinds = [
            "StellarNode",
            "NetworkPolicy",
            "PodSecurityPolicy",
            "Role",
            "ClusterRole",
            "RoleBinding",
            "ClusterRoleBinding",
            "Namespace",
            "CertificateSigningRequest",
        ];

        if !privileged_kinds.contains(&request.kind.kind.as_str()) {
            return Ok(None);
        }

        // Check if there's an existing approved ChangeRequest for this operation
        let cr_api: Api<ChangeRequest> = Api::namespaced(self.config.client.clone(), &self.config.namespace);
        
        // Look for matching ChangeRequest
        let label_selector = format!(
            "approval.stellar.org/target-name={},approval.stellar.org/target-namespace={},approval.stellar.org/target-kind={}",
            request.name,
            request.namespace.as_deref().unwrap_or(""),
            request.kind.kind
        );

        let list = cr_api.list(&kube::api::ListParams::default().labels(&label_selector)).await?;

        for cr in list.items {
            if cr.status.as_ref().map(|s| s.approval_state) == Some(ApprovalState::Approved)
                && cr.status.as_ref().map(|s| s.phase) == Some(ChangeRequestPhase::Approved)
            {
                // Check if this ChangeRequest matches the current operation
                if let Some(operation) = &cr.spec.0.operation {
                    if self.operation_matches_request(operation, request) {
                        debug!("Found approved ChangeRequest for operation: {}", cr.name_any());
                        return Ok(None); // Already approved, allow
                    }
                }
            }
        }

        // No approved ChangeRequest found, require one
        let change_class = self.infer_change_class(request);
        let required_approvers = self.get_required_approvers(&change_class.to_string()).await;
        
        let spec = ChangeRequestSpec {
            operation: PrivilegedOperation {
                class: change_class,
                target_ref: TargetReference {
                    api_version: request.kind.group.clone() + "/" + &request.kind.version,
                    kind: request.kind.kind.clone(),
                    name: request.name.clone(),
                    namespace: request.namespace.clone(),
                },
                description: format!("Privileged operation on {}/{}", request.kind.kind, request.name),
                change_payload: request.object.clone().unwrap_or_default(),
                risk_level: crate::approval::crd::RiskLevel::Medium,
            },
            required_approvers,
            optional_approvers: vec![],
            min_approvals: 2,
            ttl: chrono::Duration::hours(24),
            auto_execute: false,
            execution_window: None,
            metadata: BTreeMap::new(),
        };
        
        Ok(Some(spec))
    }

    /// Infer the change class from the admission request
    fn infer_change_class(&self, request: &SimpleAdmissionRequest) -> crate::approval::crd::ChangeClass {
        match request.kind.kind.as_str() {
            "StellarNode" => crate::approval::crd::ChangeClass::NodeUpgrade,
            "NetworkPolicy" | "PodSecurityPolicy" => crate::approval::crd::ChangeClass::SecurityPolicy,
            "Role" | "ClusterRole" | "RoleBinding" | "ClusterRoleBinding" => crate::approval::crd::ChangeClass::SecurityPolicy,
            "Namespace" => crate::approval::crd::ChangeClass::NamespaceManagement,
            "CertificateSigningRequest" => crate::approval::crd::ChangeClass::CertificateRotation,
            _ => crate::approval::crd::ChangeClass::Custom(request.kind.kind.clone()),
        }
    }

    /// Check if an operation matches the admission request
    fn operation_matches_request(&self, operation: &PrivilegedOperation, request: &SimpleAdmissionRequest) -> bool {
        operation.target_ref.kind == request.kind.kind
            && operation.target_ref.name == request.name
            && operation.target_ref.namespace == request.namespace
    }
}

/// Validate admission webhook handler
#[instrument(skip(state, review))]
async fn validate_admission(
    State(state): State<ApprovalWebhook>,
    Json(review): Json<SimpleAdmissionReview>,
) -> impl IntoResponse {
    let request = match review.request {
        Some(req) => req,
        None => {
            error!("AdmissionReview missing request");
            return (StatusCode::BAD_REQUEST, "missing request").into_response();
        }
    };

    // Check if operation requires approval
    match state.requires_approval(&request).await {
        Ok(Some(spec)) => {
            // Create a ChangeRequest
            let cr = create_change_request(&state.config.client, &state.config.namespace, spec, &request).await;
            match cr {
                Ok(created_cr) => {
                    info!("Created ChangeRequest {} for operation on {}/{}", 
                        created_cr.name_any(), request.kind.kind, request.name);
                    
                    // Deny the original request with instructions
                    let response = SimpleAdmissionResponse {
                        uid: request.uid,
                        allowed: false,
                        status: Some(SimpleStatus {
                            code: 403,
                            message: format!(
                                "Privileged operation requires multi-party approval. Created ChangeRequest: {}. \
                                Please obtain {} approvals from required approvers before proceeding.",
                                created_cr.name_any(),
                                created_cr.spec.0.min_approvals
                            ),
                        }),
                        warnings: vec![
                            "Operation blocked pending approval".to_string(),
                            format!("ChangeRequest: {}", created_cr.name_any()),
                        ],
                    };
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Err(e) => {
                    error!("Failed to create ChangeRequest: {}", e);
                    if state.config.fail_open {
                        warn!("Failing open due to webhook error");
                        let response = SimpleAdmissionResponse {
                            uid: request.uid,
                            allowed: true,
                            status: None,
                            warnings: vec![],
                        };
                        return (StatusCode::OK, Json(response)).into_response();
                    }
                    let response = SimpleAdmissionResponse {
                        uid: request.uid,
                        allowed: false,
                        status: Some(SimpleStatus {
                            code: 500,
                            message: format!("Failed to create approval request: {}", e),
                        }),
                        warnings: vec![],
                    };
                    return (StatusCode::OK, Json(response)).into_response();
                }
            }
        }
        Ok(None) => {
            // No approval required
            let response = SimpleAdmissionResponse {
                uid: request.uid,
                allowed: true,
                status: None,
                warnings: vec![],
            };
            return (StatusCode::OK, Json(response)).into_response();
        }
        Err(e) => {
            error!("Error checking approval requirement: {}", e);
            if state.config.fail_open {
                warn!("Failing open due to webhook error");
                let response = SimpleAdmissionResponse {
                    uid: request.uid,
                    allowed: true,
                    status: None,
                    warnings: vec![],
                };
                return (StatusCode::OK, Json(response)).into_response();
            }
            let response = SimpleAdmissionResponse {
                uid: request.uid,
                allowed: false,
                status: Some(SimpleStatus {
                    code: 500,
                    message: format!("Approval check failed: {}", e),
                }),
                warnings: vec![],
            };
            return (StatusCode::OK, Json(response)).into_response();
        }
    }
}

/// Mutating admission webhook handler (for adding approval annotations)
#[instrument(skip(_state, review))]
async fn mutate_admission(
    State(_state): State<ApprovalWebhook>,
    Json(review): Json<SimpleAdmissionReview>,
) -> impl IntoResponse {
    let request = match review.request {
        Some(req) => req,
        None => {
            return (StatusCode::BAD_REQUEST, "missing request").into_response();
        }
    };

    // For now, just allow - mutations could add labels/annotations
    let response = SimpleAdmissionResponse {
        uid: request.uid,
        allowed: true,
        status: None,
        warnings: vec![],
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "healthy" })))
}

/// Create a ChangeRequest resource
async fn create_change_request(
    client: &Client,
    namespace: &str,
    spec: ChangeRequestSpec,
    request: &SimpleAdmissionRequest,
) -> Result<ChangeRequest> {
    let api: Api<ChangeRequest> = Api::namespaced(client.clone(), namespace);
    
    // Generate a unique name
    let cr_name = format!(
        "cr-{}-{}-{}",
        request.kind.kind.to_lowercase(),
        request.name,
        Uuid::new_v4().to_string().split('-').next().unwrap()
    );

    let now = chrono::Utc::now();
    let expires_at = now + spec.ttl;

    let mut labels = BTreeMap::new();
    labels.insert("approval.stellar.org/target-kind".to_string(), request.kind.kind.clone());
    labels.insert("approval.stellar.org/target-name".to_string(), request.name.clone());
    if let Some(ns) = &request.namespace {
        labels.insert("approval.stellar.org/target-namespace".to_string(), ns.clone());
    }
    labels.insert("approval.stellar.org/operation".to_string(), spec.operation.class.to_string());

    let cr = ChangeRequest {
        metadata: kube::core::ObjectMeta {
            name: Some(cr_name),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            annotations: Some(BTreeMap::from([
                ("approval.stellar.org/requested-by".to_string(), request.user_info.username.clone()),
                ("approval.stellar.org/operation".to_string(), request.operation.clone()),
            ])),
            ..Default::default()
        },
        spec,
        status: Some(ChangeRequestStatus {
            phase: ChangeRequestPhase::Pending,
            approval_state: ApprovalState::Pending,
            approvals: vec![],
            approvals_received: 0,
            approvals_required: spec.min_approvals,
            quorum_reached: false,
            expires_at: Some(expires_at),
            executed_at: None,
            execution_result: None,
            conditions: vec![],
            last_updated: now,
        }),
    };

    api.create(&kube::api::PostParams::default(), &cr).await.map_err(Error::KubeError)
}

/// Register custom approvers for a change class
pub async fn register_approvers(
    webhook: &ApprovalWebhook,
    change_class: String,
    approvers: Vec<Approver>,
) {
    let mut cache = webhook.approver_cache.write().await;
    cache.insert(change_class, approvers);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::crd::{Approver, ApprovalDecision, ApprovalState};

    #[test]
    fn test_approval_decision_serialization() {
        let decision = ApprovalDecision::Approve;
        let json = serde_json::to_string(&decision).unwrap();
        assert_eq!(json, "\"Approve\"");
    }

    #[test]
    fn test_approval_state_serialization() {
        let state = ApprovalState::Pending;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"Pending\"");
    }

    #[test]
    fn test_approver_serialization() {
        let approver = Approver {
            identity: "user@example.com".to_string(),
            name: Some("Test User".to_string()),
            required: true,
        };
        let json = serde_json::to_string(&approver).unwrap();
        assert!(json.contains("user@example.com"));
        assert!(json.contains("Test User"));
    }
}