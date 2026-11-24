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

//! `TenantQuota` — declarative node of the hierarchical quota tree (#1959).

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// CPU / memory quantities (Kubernetes quantity strings, e.g. `"500m"`, `"64Gi"`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceQuantities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// One node of the quota tree. Leaves own namespaces; interior nodes group children.
///
/// Semantics: `guaranteed` is protected from preemption; usage above it is elastic
/// (borrowed) up to `max` and reclaimable. An interior node's guarantee defaults to
/// the sum of its children's and must not be lower. A child's `max` must not exceed
/// its parent's. Unset `max` means "bounded only by the parent/cluster".
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "TenantQuota",
    shortname = "tq",
    status = "TenantQuotaStatus",
    printcolumn = r#"{"name":"Parent","type":"string","jsonPath":".spec.parent"}"#,
    printcolumn = r#"{"name":"Valid","type":"string","jsonPath":".status.valid"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct TenantQuotaSpec {
    /// Name of the parent `TenantQuota`; empty for a root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Namespaces governed by this node (leaves only; each namespace in exactly one node).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Relative weight when sharing elastic capacity with siblings. Must be > 0.
    #[serde(default = "default_weight")]
    pub weight: f64,
    /// Protected minimum share.
    #[serde(default)]
    pub guaranteed: ResourceQuantities,
    /// Elastic ceiling.
    #[serde(default)]
    pub max: ResourceQuantities,
}

fn default_weight() -> f64 {
    1.0
}

/// Observed state published by the controller.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TenantQuotaStatus {
    #[serde(default)]
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}
