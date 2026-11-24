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
//! ServiceOwnershipRegistry Custom Resource Definition (epic #1522).
//!
//! A cluster-scoped registry mapping every running workload to its owning
//! team and on-call rotation. The spec declares where ownership comes from
//! (labels, deploy metadata, CODEOWNERS) and the teams' rotations; the status
//! is continuously derived by [`crate::controller::ownership_registry`].

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ServiceOwnershipRegistry",
    status = "ServiceOwnershipRegistryStatus",
    shortname = "sor",
    printcolumn = r#"{"name":"Owned","type":"integer","jsonPath":".status.ownedCount"}"#,
    printcolumn = r#"{"name":"Unowned","type":"integer","jsonPath":".status.unownedCount"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ServiceOwnershipRegistrySpec {
    /// Workload label naming the owning team (highest precedence).
    #[serde(default = "default_owner_label")]
    pub owner_label: String,
    /// Annotation set by the deploy pipeline naming the deploying team.
    #[serde(default = "default_deploy_annotation")]
    pub deploy_annotation: String,
    /// Annotation holding the workload's source path, matched against
    /// [`codeowners`](Self::codeowners).
    #[serde(default = "default_source_path_annotation")]
    pub source_path_annotation: String,
    /// CODEOWNERS file contents (lowest precedence).
    #[serde(default)]
    pub codeowners: Option<String>,
    /// Known teams and their on-call rotations.
    pub teams: Vec<TeamRotation>,
    /// Namespaces excluded from attribution (e.g. `kube-system`).
    #[serde(default)]
    pub excluded_namespaces: Vec<String>,
    /// Alertmanager base URL; unowned/stale workloads are alerted each cycle.
    #[serde(default)]
    pub alertmanager_url: Option<String>,
    /// Maximum ownership change records kept in status.
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
}

fn default_owner_label() -> String {
    "stellar.org/owner".into()
}
fn default_deploy_annotation() -> String {
    "stellar.org/deployed-by-team".into()
}
fn default_source_path_annotation() -> String {
    "stellar.org/source-path".into()
}
fn default_history_limit() -> usize {
    100
}

/// A team and its on-call rotation.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamRotation {
    pub team: String,
    /// CODEOWNERS handles that map to this team (e.g. `@devops-team`).
    #[serde(default)]
    pub handles: Vec<String>,
    /// On-call rotation / schedule identifier.
    pub rotation: String,
    /// Alertmanager receiver for this team.
    pub receiver: String,
    /// Whether the rotation currently has someone on call.
    #[serde(default = "default_true")]
    pub live: bool,
}

fn default_true() -> bool {
    true
}

/// Identifies a workload.
#[derive(
    Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadRef {
    pub kind: String,
    pub namespace: String,
    pub name: String,
}

/// Where an ownership attribution came from.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OwnershipSource {
    Label,
    DeployMetadata,
    Codeowners,
}

/// Resolved owner of a workload.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipEntry {
    pub workload: WorkloadRef,
    pub team: String,
    pub rotation: String,
    pub receiver: String,
    pub source: OwnershipSource,
}

/// A workload whose attribution cannot be trusted.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StaleOwnership {
    pub workload: WorkloadRef,
    pub team: String,
    pub reason: String,
}

/// One ownership change.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipChange {
    pub workload: WorkloadRef,
    pub previous_team: Option<String>,
    pub team: Option<String>,
    pub source: Option<OwnershipSource>,
    pub changed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceOwnershipRegistryStatus {
    #[serde(default)]
    pub entries: Vec<OwnershipEntry>,
    #[serde(default)]
    pub unowned: Vec<WorkloadRef>,
    #[serde(default)]
    pub stale: Vec<StaleOwnership>,
    /// Oldest first, bounded by `spec.historyLimit`.
    #[serde(default)]
    pub history: Vec<OwnershipChange>,
    #[serde(default)]
    pub owned_count: u32,
    #[serde(default)]
    pub unowned_count: u32,
    #[serde(default)]
    pub last_reconciled: Option<DateTime<Utc>>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
