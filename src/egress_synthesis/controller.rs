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
//! Batch controller that writes synthesized egress policies.
//!
//! The controller writes every candidate into a shadow namespace first. Only
//! a candidate that passed the staged rollout is promoted into the namespace
//! of the workload it protects.

use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, Patch, PatchParams};
use kube::Client;
use tracing::info;

use super::candidate::CandidatePolicy;
use crate::error::Result;

/// Prefix of the namespace the candidates are written to.
pub const SHADOW_NAMESPACE_PREFIX: &str = "stellar-egress-shadow";

/// The shadow namespace a workload namespace is staged in.
pub fn shadow_namespace_for(workload_namespace: &str) -> String {
    format!("{SHADOW_NAMESPACE_PREFIX}-{workload_namespace}")
}

/// Write a candidate policy into the shadow namespace.
///
/// The candidate is written with the server side apply field manager of the
/// synthesizer so repeated runs converge instead of fighting.
pub async fn apply_candidate(
    client: &Client,
    candidate: &CandidatePolicy,
    shadow_namespace: &str,
    dry_run: bool,
) -> Result<()> {
    let policy = candidate.to_network_policy();
    apply_policy(client, &policy, shadow_namespace, dry_run).await
}

/// Promote a candidate policy into the namespace of the workload.
pub async fn promote_candidate(
    client: &Client,
    candidate: &CandidatePolicy,
    target_namespace: &str,
    dry_run: bool,
) -> Result<()> {
    let policy = candidate.to_network_policy();
    apply_policy(client, &policy, target_namespace, dry_run).await
}

async fn apply_policy(
    client: &Client,
    policy: &NetworkPolicy,
    namespace: &str,
    dry_run: bool,
) -> Result<()> {
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), namespace);
    let name = policy
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "egress-synthesized".to_string());

    let patch_params = PatchParams {
        field_manager: Some("stellar-k8s-egress-synthesizer".to_string()),
        dry_run,
        ..Default::default()
    };

    api.patch(&name, &patch_params, &Patch::Apply(policy))
        .await?;

    info!(
        "synthesized egress policy {}/{} applied (dry_run={dry_run})",
        namespace, name
    );
    Ok(())
}
