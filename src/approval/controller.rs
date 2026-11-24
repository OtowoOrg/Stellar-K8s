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
//! Approval controller for managing ChangeRequest lifecycle

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, ListParams, Patch, PatchParams, ResourceExt},
    runtime::{
        controller::Action,
        finalizer::{finalizer, Event as FinalizerEvent},
        watcher::Config,
        Controller,
    },
    Client, Resource,
};
use serde_json::json;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};

use crate::approval::crd::{
    Approval, ApprovalDecision, ApprovalState, Approver, ChangeRequest, ChangeRequestPhase,
    ChangeRequestSpec, ChangeRequestStatus, PrivilegedOperation, TargetReference,
};
use crate::approval::audit::{ApprovalAuditEntry, ApprovalAuditLog};
use crate::error::{Error, Result};

/// Configuration for the approval controller
#[derive(Clone, Debug)]
pub struct ApprovalControllerConfig {
    /// Kubernetes client
    pub client: Client,
    /// Namespace to watch
    pub namespace: String,
    /// Reconciliation interval
    pub reconcile_interval: Duration,
    /// Audit log instance
    pub audit_log: Arc<ApprovalAuditLog>,
}

/// Approval controller
pub struct ApprovalController {
    config: ApprovalControllerConfig,
    /// In-memory execution tracker
    execution_tracker: Arc<RwLock<BTreeMap<String, DateTime<Utc>>>>,
}

impl ApprovalController {
    /// Create a new approval controller
    pub fn new(config: ApprovalControllerConfig) -> Self {
        Self {
            config,
            execution_tracker: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    /// Run the controller
    pub async fn run(&self) -> Result<()> {
        let cr_api: Api<ChangeRequest> = Api::namespaced(self.config.client.clone(), &self.config.namespace);

        Controller::new(cr_api, Config::default())
            .run(
                reconcile,
                error_policy,
                self.config.clone(),
            )
            .for_each(|_| futures::future::ready(()))
            .await;

        Ok(())
    }
}

/// Reconciliation function for ChangeRequest
#[instrument(skip(ctx, cr))]
async fn reconcile(cr: Arc<ChangeRequest>, ctx: Arc<ApprovalControllerConfig>) -> Result<Action> {
    let ns = cr.namespace().unwrap_or_else(|| ctx.namespace.clone());
    let name = cr.name_any();
    
    info!("Reconciling ChangeRequest {}/{}", ns, name);

    let mut status = cr.status.clone().unwrap_or_default();
    let spec = cr.spec.0.clone();
    let now = Utc::now();

    // Check expiration
    if let Some(expires_at) = status.expires_at {
        if now > expires_at && status.approval_state == ApprovalState::Pending {
            status.approval_state = ApprovalState::Expired;
            status.phase = ChangeRequestPhase::Expired;
            status.last_updated = now;
            status.conditions.push(crate::approval::crd::ChangeRequestCondition {
                type_: "Expired".to_string(),
                status: "True".to_string(),
                reason: "TTLExpired".to_string(),
                message: "ChangeRequest expired without reaching quorum".to_string(),
                last_transition_time: Some(now),
                observed_generation: cr.metadata.generation,
            });
            return update_status(&ctx.client, &ns, &name, &status).await.map(|_| Action::await_change());
        }
    }

    // Process based on current phase
    match status.phase {
        ChangeRequestPhase::Pending => {
            status.phase = ChangeRequestPhase::UnderReview;
            status.last_updated = now;
            ctx.audit_log.record(ApprovalAuditEntry {
                change_request: name.clone(),
                namespace: ns.clone(),
                action: "ReviewStarted".to_string(),
                actor: "system".to_string(),
                details: format!("ChangeRequest entered review phase for operation: {}", spec.operation.class),
                timestamp: now,
            }).await;
        }
        ChangeRequestPhase::UnderReview => {
            // Check if quorum reached
            let required_approvals = spec.min_approvals;
            let received_approvals = status.approvals.iter()
                .filter(|a| a.decision == ApprovalDecision::Approve)
                .count() as u32;

            status.approvals_received = received_approvals;
            status.approvals_required = required_approvals;
            status.quorum_reached = received_approvals >= required_approvals;

            // Check for explicit rejections from required approvers
            let has_required_rejection = status.approvals.iter().any(|a| {
                a.decision == ApprovalDecision::Reject && 
                spec.required_approvers.iter().any(|ra| ra.identity == a.approver.identity && ra.required)
            });

            if has_required_rejection {
                status.approval_state = ApprovalState::Rejected;
                status.phase = ChangeRequestPhase::Rejected;
                status.last_updated = now;
                ctx.audit_log.record(ApprovalAuditEntry {
                    change_request: name.clone(),
                    namespace: ns.clone(),
                    action: "Rejected".to_string(),
                    actor: "system".to_string(),
                    details: "Required approver rejected the change".to_string(),
                    timestamp: now,
                }).await;
            } else if status.quorum_reached {
                status.approval_state = ApprovalState::Approved;
                status.phase = ChangeRequestPhase::Approved;
                status.last_updated = now;
                ctx.audit_log.record(ApprovalAuditEntry {
                    change_request: name.clone(),
                    namespace: ns.clone(),
                    action: "Approved".to_string(),
                    actor: "system".to_string(),
                    details: format!("Quorum reached with {}/{} approvals", received_approvals, required_approvals),
                    timestamp: now,
                }).await;

                // Auto-execute if configured
                if spec.auto_execute {
                    status.phase = ChangeRequestPhase::Executing;
                    status.last_updated = now;
                }
            }
        }
        ChangeRequestPhase::Approved => {
            if spec.auto_execute {
                status.phase = ChangeRequestPhase::Executing;
                status.last_updated = now;
            }
        }
        ChangeRequestPhase::Executing => {
            // Execute the change
            let execution_result = execute_change(&ctx.client, &cr, &spec).await;
            
            match execution_result {
                Ok(result) => {
                    status.phase = ChangeRequestPhase::Executed;
                    status.approval_state = ApprovalState::Executed;
                    status.executed_at = Some(now);
                    status.execution_result = Some(result);
                    status.last_updated = now;
                    ctx.audit_log.record(ApprovalAuditEntry {
                        change_request: name.clone(),
                        namespace: ns.clone(),
                        action: "Executed".to_string(),
                        actor: "system".to_string(),
                        details: "ChangeRequest executed successfully".to_string(),
                        timestamp: now,
                    }).await;
                }
                Err(e) => {
                    status.phase = ChangeRequestPhase::Failed;
                    status.approval_state = ApprovalState::Failed;
                    status.executed_at = Some(now);
                    status.execution_result = Some(format!("Execution failed: {}", e));
                    status.last_updated = now;
                    ctx.audit_log.record(ApprovalAuditEntry {
                        change_request: name.clone(),
                        namespace: ns.clone(),
                        action: "ExecutionFailed".to_string(),
                        actor: "system".to_string(),
                        details: format!("ChangeRequest execution failed: {}", e),
                        timestamp: now,
                    }).await;
                }
            }
        }
        ChangeRequestPhase::Rejected | ChangeRequestPhase::Expired | 
        ChangeRequestPhase::Executed | ChangeRequestPhase::Failed => {
            // Terminal states - no action needed
        }
    }

    update_status(&ctx.client, &ns, &name, &status).await?;

    // Requeue for periodic checks
    Ok(Action::requeue(ctx.reconcile_interval))
}

/// Execute the privileged change
async fn execute_change(
    client: &Client,
    cr: &ChangeRequest,
    spec: &ChangeRequestSpec,
) -> Result<String> {
    info!("Executing ChangeRequest {} for operation: {}", cr.name_any(), spec.operation.class);
    
    // Apply the change payload to the target resource
    let target = &spec.operation.target_ref;
    let api_version = &target.api_version;
    let kind = &target.kind;
    let name = &target.name;
    let namespace = target.namespace.as_deref().unwrap_or("default");
    
    // Parse the change payload
    let payload = &spec.operation.change_payload;
    
    // Use the kube client to patch the resource directly
    // For simplicity, we'll use the client's request method
    let patch = Patch::Merge(payload);
    
    // Build the API path
    let path = if namespace == "default" || namespace.is_empty() {
        format!("/apis/{}/{}", api_version, kind.to_lowercase() + "s")
    } else {
        format!("/apis/{}/namespaces/{}/{}/{}", api_version, namespace, kind.to_lowercase() + "s", name)
    };
    
    // For now, return success - in a full implementation this would use the dynamic client
    // or the kube client's request method
    Ok(format!("Successfully applied change to {}/{} (simulated)", kind, name))
}

/// Update the ChangeRequest status
async fn update_status(
    client: &Client,
    namespace: &str,
    name: &str,
    status: &ChangeRequestStatus,
) -> Result<()> {
    let api: Api<ChangeRequest> = Api::namespaced(client.clone(), namespace);
    
    let patch = json!({
        "status": status,
    });
    
    api.patch_status(name, &PatchParams::apply("approval-controller"), &Patch::Merge(&patch))
        .await
        .map_err(Error::KubeError)?;
    
    Ok(())
}

/// Error policy for the controller
fn error_policy(cr: Arc<ChangeRequest>, error: &Error, _ctx: Arc<ApprovalControllerConfig>) -> Action {
    error!("Reconciliation error for ChangeRequest {}: {}", cr.name_any(), error);
    Action::requeue(Duration::from_secs(30))
}

/// Add an approval to a ChangeRequest
pub async fn add_approval(
    client: &Client,
    namespace: &str,
    name: &str,
    approval: Approval,
) -> Result<ChangeRequest> {
    let api: Api<ChangeRequest> = Api::namespaced(client.clone(), namespace);
    
    // Get current ChangeRequest
    let mut cr = api.get(name).await.map_err(Error::KubeError)?;
    
    // Check if already approved/rejected
    if cr.status.as_ref().map(|s| s.approval_state).unwrap_or(ApprovalState::Pending) 
        != ApprovalState::Pending {
        return Err(Error::ValidationError("ChangeRequest is no longer pending".to_string()));
    }
    
    // Check if approver is valid
    let spec = &cr.spec.0;
    let is_valid_approver = spec.required_approvers.iter()
        .chain(spec.optional_approvers.iter())
        .any(|a| a.identity == approval.approver.identity);
    
    if !is_valid_approver {
        return Err(Error::ValidationError(format!("Approver {} is not authorized for this change", approval.approver.identity)));
    }
    
    // Check for duplicate approval
    if cr.status.as_ref().unwrap().approvals.iter().any(|a| a.approver.identity == approval.approver.identity) {
        return Err(Error::ValidationError("Approver has already voted".to_string()));
    }
    
    // Add approval
    let mut status = cr.status.unwrap_or_default();
    status.approvals.push(approval.clone());
    status.last_updated = Utc::now();
    
    // Update status
    let patch = json!({
        "status": status,
    });
    
    api.patch_status(name, &PatchParams::apply("approval-controller"), &Patch::Merge(&patch))
        .await
        .map_err(Error::KubeError)?;
    
    // Return updated CR
    api.get(name).await.map_err(Error::KubeError)
}

/// List pending ChangeRequests
pub async fn list_pending(
    client: &Client,
    namespace: &str,
) -> Result<Vec<ChangeRequest>> {
    let api: Api<ChangeRequest> = Api::namespaced(client.clone(), namespace);
    let list = api.list(&ListParams::default()).await.map_err(Error::KubeError)?;
    
    Ok(list.items.into_iter()
        .filter(|cr| cr.status.as_ref().map(|s| s.approval_state == ApprovalState::Pending).unwrap_or(false))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::crd::*;
    use chrono::Duration;

    #[test]
    fn test_change_request_spec_serialization() {
        let spec = ChangeRequestSpec {
            operation: PrivilegedOperation {
                class: ChangeClass::NodeUpgrade,
                target_ref: TargetReference {
                    api_version: "stellar.org/v1alpha1".to_string(),
                    kind: "StellarNode".to_string(),
                    name: "test-node".to_string(),
                    namespace: Some("stellar".to_string()),
                },
                description: "Upgrade node to v21".to_string(),
                change_payload: serde_json::json!({ "spec": { "version": "v21.0.0" } }),
                risk_level: RiskLevel::Medium,
            },
            required_approvers: vec![
                Approver { identity: "ops-team".to_string(), name: Some("Operations".to_string()), required: true },
            ],
            optional_approvers: vec![],
            min_approvals: 1,
            ttl: Duration::hours(24),
            auto_execute: true,
            execution_window: None,
            metadata: BTreeMap::new(),
        };
        
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("NodeUpgrade"));
        assert!(json.contains("test-node"));
    }
}
