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
//! Namespace security baseline evaluation and auto-remediation

pub mod baseline;
pub mod evaluator;
pub mod remediation;
pub mod audit;
pub mod policy_drift;

pub use baseline::{SecurityBaseline, BaselineSpec, BaselineStatus, BaselineCheck, CheckResult, CheckSeverity};
pub use evaluator::{BaselineEvaluator, EvaluatorConfig, EvaluationResult};
pub use remediation::{BaselineRemediator, RemediationConfig, RemediationAction, RemediationResult};
pub use audit::{BaselineAuditLog, AuditEntry, AuditConfig};
pub use policy_drift::{GitOpsProposalClient, GitOpsProposal, GitOpsPullRequest, PolicyDriftCase, PolicyDriftCaseStatus, PolicyDriftLoop, WebhookGitOpsClient};