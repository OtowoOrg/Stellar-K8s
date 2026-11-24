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
//! Multi-party approval workflow for privileged changes
//!
//! This module implements a secure approval system using ChangeRequest CRs
//! and admission webhooks to enforce N-of-M signed approval quorums.

pub mod crd;
pub mod webhook;
pub mod controller;
pub mod audit;

pub use crd::{
    ChangeRequest, ChangeRequestSpec, ChangeRequestStatus, ChangeClass, Approval,
    ApprovalState, Approver, ChangeRequestPhase, PrivilegedOperation,
};
pub use webhook::{ApprovalWebhook, ApprovalWebhookConfig};
pub use controller::{ApprovalController, ApprovalControllerConfig};
pub use audit::{ApprovalAuditEntry, ApprovalAuditLog};