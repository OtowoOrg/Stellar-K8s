use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::approval::crd::{
    Approver, ChangeClass, ChangeRequestSpec, ChangeRequestSpecWrap, PrivilegedOperation,
    RiskLevel, TargetReference,
};
use crate::error::{Error, Result};
use crate::namespace_security::baseline::{
    CheckResult, CheckSeverity, CheckStatus, RemediationAction,
};
use crate::namespace_security::evaluator::EvaluationResult;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PolicyDriftCaseStatus {
    Detected,
    PrProposed,
    AwaitingApproval,
    Approved,
    Merged,
    Verifying,
    Cleared,
    Reopened,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyDriftCase {
    pub case_id: String,
    pub baseline: String,
    pub namespace: String,
    pub check_id: String,
    pub severity: CheckSeverity,
    pub status: PolicyDriftCaseStatus,
    pub remediation: RemediationAction,
    pub detected_at: DateTime<Utc>,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
    pub cleared_at: Option<DateTime<Utc>>,
    pub verification_attempts: u32,
    pub pull_request_url: Option<String>,
    pub pull_request_number: Option<u64>,
    pub change_request_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOpsPullRequest {
    pub url: String,
    pub number: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOpsProposal {
    pub case_id: String,
    pub branch: String,
    pub title: String,
    pub commit_message: String,
    pub namespace: String,
    pub check_id: String,
    pub action: RemediationAction,
}

#[async_trait]
pub trait GitOpsProposalClient: Send + Sync {
    async fn propose(&self, proposal: GitOpsProposal) -> Result<GitOpsPullRequest>;
}

pub struct WebhookGitOpsClient {
    client: reqwest::Client,
    endpoint: String,
}

impl WebhookGitOpsClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self { client: reqwest::Client::new(), endpoint: endpoint.into() }
    }
}

#[async_trait]
impl GitOpsProposalClient for WebhookGitOpsClient {
    async fn propose(&self, proposal: GitOpsProposal) -> Result<GitOpsPullRequest> {
        self.client
            .post(&self.endpoint)
            .json(&proposal)
            .send()
            .await
            .map_err(|error| Error::ValidationError(format!("GitOps proposal failed: {error}")))?
            .error_for_status()
            .map_err(|error| Error::ValidationError(format!("GitOps proposal rejected: {error}")))?
            .json::<GitOpsPullRequest>()
            .await
            .map_err(|error| Error::SerializationError(error.to_string()))
    }
}

pub struct PolicyDriftLoop {
    required_approvers: Vec<Approver>,
}

impl PolicyDriftLoop {
    pub fn new(required_approvers: Vec<Approver>) -> Self {
        Self { required_approvers }
    }

    pub fn detect(&self, evaluation: &EvaluationResult) -> Vec<PolicyDriftCase> {
        evaluation
            .namespace_results
            .iter()
            .flat_map(|(namespace, result)| result.check_results.iter().filter_map(|check| {
                if check.status != CheckStatus::Fail {
                    return None;
                }
                let remediation = check.remediation_action.clone()?;
                Some(self.new_case(evaluation, namespace, check, remediation))
            }))
            .collect()
    }

    pub async fn propose<C: GitOpsProposalClient>(
        &self,
        case: &mut PolicyDriftCase,
        client: &C,
    ) -> Result<GitOpsPullRequest> {
        if !matches!(case.status, PolicyDriftCaseStatus::Detected | PolicyDriftCaseStatus::Reopened) {
            return Err(Error::ValidationError(format!("case {} is not ready for proposal", case.case_id)));
        }

        let proposal = GitOpsProposal {
            case_id: case.case_id.clone(),
            branch: format!("security-baseline/{}", case.case_id),
            title: format!("fix(security): remediate {} in {}", case.check_id, case.namespace),
            commit_message: format!("fix(security): remediate {}", case.check_id),
            namespace: case.namespace.clone(),
            check_id: case.check_id.clone(),
            action: case.remediation.clone(),
        };
        let pull_request = client.propose(proposal).await?;
        case.pull_request_url = Some(pull_request.url.clone());
        case.pull_request_number = pull_request.number;
        case.status = PolicyDriftCaseStatus::AwaitingApproval;
        Ok(pull_request)
    }

    pub fn approval_request(&self, case: &mut PolicyDriftCase) -> Result<ChangeRequestSpecWrap> {
        if case.status != PolicyDriftCaseStatus::AwaitingApproval {
            return Err(Error::ValidationError(format!("case {} has no proposed PR", case.case_id)));
        }
        case.change_request_name = Some(format!("policy-drift-{}", case.case_id));
        Ok(ChangeRequestSpecWrap(ChangeRequestSpec {
            operation: PrivilegedOperation {
                class: ChangeClass::SecurityPolicy,
                target_ref: TargetReference {
                    api_version: case.remediation.target_resource.api_version.clone(),
                    kind: case.remediation.target_resource.kind.clone(),
                    name: case.remediation.target_resource.name.clone(),
                    namespace: case.remediation.target_resource.namespace.clone().or_else(|| Some(case.namespace.clone())),
                },
                description: case.remediation.description.clone(),
                change_payload: serde_json::json!({
                    "caseId": case.case_id,
                    "pullRequest": case.pull_request_url,
                    "remediation": case.remediation,
                }),
                risk_level: risk_level(&case.severity),
            },
            required_approvers: self.required_approvers.clone(),
            optional_approvers: vec![],
            min_approvals: 1,
            ttl: "24h".to_string(),
            auto_execute: false,
            execution_window: None,
            metadata: BTreeMap::from([
                ("policyDriftCase".to_string(), case.case_id.clone()),
                ("pullRequest".to_string(), case.pull_request_url.clone().unwrap_or_default()),
            ]),
        }))
    }

    pub fn mark_merged(&self, case: &mut PolicyDriftCase, merged_at: DateTime<Utc>) -> Result<()> {
        if !matches!(case.status, PolicyDriftCaseStatus::Approved | PolicyDriftCaseStatus::AwaitingApproval) {
            return Err(Error::ValidationError(format!("case {} cannot be marked merged", case.case_id)));
        }
        case.merged_at = Some(merged_at);
        case.status = PolicyDriftCaseStatus::Verifying;
        Ok(())
    }

    pub fn verify(&self, case: &mut PolicyDriftCase, evaluation: &EvaluationResult) -> bool {
        case.verification_attempts += 1;
        case.last_verified_at = Some(evaluation.timestamp);
        let still_violates = evaluation.namespace_results.get(&case.namespace)
            .and_then(|result| result.check_results.iter().find(|check| check.check_id == case.check_id))
            .map(|check| check.status == CheckStatus::Fail)
            .unwrap_or(false);
        if still_violates {
            case.status = PolicyDriftCaseStatus::Reopened;
            false
        } else {
            case.status = PolicyDriftCaseStatus::Cleared;
            case.cleared_at = Some(evaluation.timestamp);
            true
        }
    }

    fn new_case(&self, evaluation: &EvaluationResult, namespace: &str, check: &CheckResult, remediation: RemediationAction) -> PolicyDriftCase {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}:{}:{}", evaluation.baseline_name, namespace, check.check_id));
        let case_id = hex::encode(hasher.finalize())[..16].to_string();
        PolicyDriftCase {
            case_id,
            baseline: evaluation.baseline_name.clone(),
            namespace: namespace.to_string(),
            check_id: check.check_id.clone(),
            severity: check.severity.clone(),
            status: PolicyDriftCaseStatus::Detected,
            remediation,
            detected_at: evaluation.timestamp,
            last_verified_at: None,
            merged_at: None,
            cleared_at: None,
            verification_attempts: 0,
            pull_request_url: None,
            pull_request_number: None,
            change_request_name: None,
        }
    }
}

fn risk_level(severity: &CheckSeverity) -> RiskLevel {
    match severity {
        CheckSeverity::Critical => RiskLevel::Critical,
        CheckSeverity::High => RiskLevel::High,
        CheckSeverity::Warning => RiskLevel::Medium,
        CheckSeverity::Info => RiskLevel::Low,
    }
}
