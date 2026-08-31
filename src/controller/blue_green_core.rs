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
//! Blue/green rollout for **Stellar Core / Validator** StatefulSets.
//!
//! Separate from [`super::blue_green`] (Horizon/Soroban RPC Deployments).
//!
//! Safety invariants:
//! - independent PVCs (never share a live Core data volume)
//! - green warms with `NODE_IS_VALIDATOR=false`
//! - cutover serializes identity: blue fully down before green publishes
//! - Service selector switches only after green is publishing, has observed the
//!   current publish-rollout token, and is healthy (Ready + Synced + lag)
//! - rollback requires blue Ready + synced before Service switch
//! - never delete rollback-protected PVCs
//! - failed rollouts stay failed until explicit `stellar.org/bg-retry=true`

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{
    ConfigMap, PersistentVolumeClaim, Pod, Service, TypedLocalObjectReference,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::discovery::ApiResource;
use kube::{Client, ResourceExt};
use serde::Deserialize;
use tracing::{info, instrument, warn};

use crate::controller::kms_secret;
use crate::controller::resource_meta::merge_resource_meta;
use crate::controller::resources::{
    build_config_map, build_pvc, build_service, build_statefulset, owner_reference, resource_name,
    standard_labels,
};
use crate::controller::sync_state_monitor::{parse_sync_state, CoreInfoSnapshot};
use crate::crd::types::{BlueGreenStrategyConfig, NodeType, RolloutStrategyType};
use crate::crd::{CoreSyncState, StellarNode};
use crate::error::{Error, Result};

pub const COLOR_LABEL: &str = "stellar.org/deployment-color";
pub const COLOR_BLUE: &str = "blue";
pub const COLOR_GREEN: &str = "green";
pub const ROLE_LABEL: &str = "stellar.org/bg-role";
pub const ROLE_ACTIVE: &str = "active";
pub const ROLE_STANDBY: &str = "standby";

pub const ANN_PHASE: &str = "stellar.org/bg-phase";
pub const ANN_ACTIVE_COLOR: &str = "stellar.org/bg-active-color";
pub const ANN_TARGET_VERSION: &str = "stellar.org/bg-target-version";
pub const ANN_SNAPSHOT: &str = "stellar.org/bg-snapshot";
pub const ANN_CUTOVER_AT: &str = "stellar.org/bg-cutover-at";
pub const ANN_STARTED_AT: &str = "stellar.org/bg-started-at";
pub const ANN_BLUE_VERSION: &str = "stellar.org/bg-blue-version";
pub const ANN_RETRY: &str = "stellar.org/bg-retry";
pub const ANN_CUTOVER_STEP: &str = "stellar.org/bg-cutover-step";
pub const ANN_ROLLBACK_STEP: &str = "stellar.org/bg-rollback-step";
/// Pod-template annotation used to force a StatefulSet rollout after publish config changes.
pub const ANN_PUBLISH_ROLLOUT: &str = "stellar.org/bg-publish-rollout";

/// Persisted rollout phase for Validator blue/green.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreBlueGreenPhase {
    BlueActive,
    PreparingGreen,
    WaitingForGreen,
    /// Serialized cutover in progress (see [`CutoverStep`]).
    CuttingOver,
    GreenActive,
    /// Serialized rollback in progress (see [`RollbackStep`]).
    RollingBack,
    /// Stable failure; no destructive prep until `stellar.org/bg-retry=true`.
    Failed,
    /// Green is active and a further version bump was requested; deferred by design.
    UpgradeDeferred,
}

impl CoreBlueGreenPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BlueActive => "BlueActive",
            Self::PreparingGreen => "PreparingGreen",
            Self::WaitingForGreen => "WaitingForGreen",
            Self::CuttingOver => "CuttingOver",
            Self::GreenActive => "GreenActive",
            Self::RollingBack => "RollingBack",
            Self::Failed => "Failed",
            Self::UpgradeDeferred => "UpgradeDeferred",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "PreparingGreen" => Self::PreparingGreen,
            "WaitingForGreen" => Self::WaitingForGreen,
            "CuttingOver" | "Cutover" | "GreenReady" => Self::CuttingOver,
            "GreenActive" => Self::GreenActive,
            "RollingBack" => Self::RollingBack,
            "Failed" => Self::Failed,
            "UpgradeDeferred" => Self::UpgradeDeferred,
            _ => Self::BlueActive,
        }
    }
}

/// Sub-steps of a serialized cutover (persisted in `stellar.org/bg-cutover-step`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CutoverStep {
    /// Scale blue to 0.
    ScaleBlueDown,
    /// Wait until blue pods are gone / STS fully scaled down.
    WaitBlueDown,
    /// Set green `NODE_IS_VALIDATOR=true` and force STS rollout.
    EnableGreenPublish,
    /// Wait for restarted green Ready + Synced + lag gate + observed publish-rollout.
    WaitGreenHealthy,
    /// Switch Service selector to green.
    SwitchService,
    Complete,
}

impl CutoverStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ScaleBlueDown => "ScaleBlueDown",
            Self::WaitBlueDown => "WaitBlueDown",
            Self::EnableGreenPublish => "EnableGreenPublish",
            Self::WaitGreenHealthy => "WaitGreenHealthy",
            Self::SwitchService => "SwitchService",
            Self::Complete => "Complete",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "WaitBlueDown" => Self::WaitBlueDown,
            "EnableGreenPublish" => Self::EnableGreenPublish,
            "WaitGreenHealthy" => Self::WaitGreenHealthy,
            "SwitchService" => Self::SwitchService,
            "Complete" => Self::Complete,
            _ => Self::ScaleBlueDown,
        }
    }
}

/// Commands emitted by the pure cutover planner (for tests + reconcile).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CutoverCommand {
    ScaleBlueToZero,
    Wait,
    EnableGreenPublishingAndRestart,
    SwitchServiceToGreenAndFinish,
}

/// Advance cutover only when safety predicates hold.
///
/// Ordering guaranteed:
/// ScaleBlueDown -> WaitBlueDown -> EnableGreenPublish -> WaitGreenHealthy -> SwitchService
pub fn plan_cutover_advance(
    step: CutoverStep,
    blue_fully_down: bool,
    green_eligible: bool,
) -> (CutoverStep, CutoverCommand) {
    match step {
        CutoverStep::ScaleBlueDown => (CutoverStep::WaitBlueDown, CutoverCommand::ScaleBlueToZero),
        CutoverStep::WaitBlueDown => {
            if blue_fully_down {
                (
                    CutoverStep::EnableGreenPublish,
                    CutoverCommand::EnableGreenPublishingAndRestart,
                )
            } else {
                (CutoverStep::WaitBlueDown, CutoverCommand::Wait)
            }
        }
        CutoverStep::EnableGreenPublish => {
            // Command already applied; move to wait. Caller persists step after acting.
            (CutoverStep::WaitGreenHealthy, CutoverCommand::Wait)
        }
        CutoverStep::WaitGreenHealthy => {
            if green_eligible {
                (
                    CutoverStep::SwitchService,
                    CutoverCommand::SwitchServiceToGreenAndFinish,
                )
            } else {
                (CutoverStep::WaitGreenHealthy, CutoverCommand::Wait)
            }
        }
        CutoverStep::SwitchService | CutoverStep::Complete => {
            (CutoverStep::Complete, CutoverCommand::Wait)
        }
    }
}

/// True iff Service switch is allowed: blue down AND green eligible after publish.
pub fn may_switch_service_to_green(blue_fully_down: bool, green_eligible: bool) -> bool {
    blue_fully_down && green_eligible
}

/// True iff green must remain non-publishing while blue is still active.
pub fn green_must_stay_standby(blue_fully_down: bool) -> bool {
    !blue_fully_down
}

/// True when the running green workload reflects the expected publish-rollout token.
///
/// An empty/missing expected token means publish+rollout has not been recorded yet,
/// so observation has not succeeded (fail-closed).
pub fn green_publish_rollout_observed(
    expected_token: Option<&str>,
    observed_pod_token: Option<&str>,
) -> bool {
    match expected_token {
        Some(expected) if !expected.is_empty() => observed_pod_token == Some(expected),
        _ => false,
    }
}

/// Combined eligibility for advancing to Service switch.
pub fn green_ready_for_service_switch(
    blue_fully_down: bool,
    health_eligible: bool,
    expected_rollout: Option<&str>,
    observed_pod_rollout: Option<&str>,
) -> bool {
    may_switch_service_to_green(
        blue_fully_down,
        health_eligible && green_publish_rollout_observed(expected_rollout, observed_pod_rollout),
    )
}

/// Executor safety clamp used by [`run_cutover_steps`] (and unit-tested).
///
/// Returns the step after enforcing "green stays standby while blue is up", and
/// whether green is eligible for Service switch (health + observed publish rollout).
pub fn enforce_cutover_safety(
    step: CutoverStep,
    blue_fully_down: bool,
    health_eligible: bool,
    expected_rollout: Option<&str>,
    observed_pod_rollout: Option<&str>,
) -> (CutoverStep, bool) {
    let step = if green_must_stay_standby(blue_fully_down)
        && matches!(
            step,
            CutoverStep::EnableGreenPublish
                | CutoverStep::WaitGreenHealthy
                | CutoverStep::SwitchService
        ) {
        CutoverStep::WaitBlueDown
    } else {
        step
    };

    let green_eligible =
        health_eligible && green_publish_rollout_observed(expected_rollout, observed_pod_rollout);
    (step, green_eligible)
}

/// Read the publish-rollout token from a Pod (inherited from STS pod template).
pub fn pod_publish_rollout_token(pod: &Pod) -> Option<String> {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(ANN_PUBLISH_ROLLOUT).cloned())
        .filter(|s| !s.is_empty())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RollbackStep {
    StopGreen,
    ScaleBlueUp,
    WaitBlueHealthy,
    SwitchService,
    Complete,
}

impl RollbackStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StopGreen => "StopGreen",
            Self::ScaleBlueUp => "ScaleBlueUp",
            Self::WaitBlueHealthy => "WaitBlueHealthy",
            Self::SwitchService => "SwitchService",
            Self::Complete => "Complete",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "ScaleBlueUp" => Self::ScaleBlueUp,
            "WaitBlueHealthy" => Self::WaitBlueHealthy,
            "SwitchService" => Self::SwitchService,
            "Complete" => Self::Complete,
            _ => Self::StopGreen,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RollbackCommand {
    ScaleGreenToZero,
    ScaleBlueToOne,
    Wait,
    SwitchServiceToBlueAndFinish,
}

pub fn plan_rollback_advance(
    step: RollbackStep,
    blue_eligible: bool,
) -> (RollbackStep, RollbackCommand) {
    match step {
        RollbackStep::StopGreen => (RollbackStep::ScaleBlueUp, RollbackCommand::ScaleGreenToZero),
        RollbackStep::ScaleBlueUp => (
            RollbackStep::WaitBlueHealthy,
            RollbackCommand::ScaleBlueToOne,
        ),
        RollbackStep::WaitBlueHealthy => {
            if blue_eligible {
                (
                    RollbackStep::SwitchService,
                    RollbackCommand::SwitchServiceToBlueAndFinish,
                )
            } else {
                (RollbackStep::WaitBlueHealthy, RollbackCommand::Wait)
            }
        }
        RollbackStep::SwitchService | RollbackStep::Complete => {
            (RollbackStep::Complete, RollbackCommand::Wait)
        }
    }
}

/// Gate for promoting a color (cutover green or rollback blue).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CutoverGateResult {
    NotReady {
        reason: String,
    },
    CatchingUp {
        sync_state: CoreSyncState,
        green_ledger: Option<u64>,
        blue_ledger: Option<u64>,
    },
    Unhealthy {
        reason: String,
    },
    Eligible {
        green_ledger: u64,
        blue_ledger: u64,
        lag: u64,
    },
}

impl CutoverGateResult {
    pub fn is_eligible(&self) -> bool {
        matches!(self, Self::Eligible { .. })
    }

    pub fn reason(&self) -> String {
        match self {
            Self::NotReady { reason } => reason.clone(),
            Self::CatchingUp { sync_state, .. } => format!("catching up ({sync_state})"),
            Self::Unhealthy { reason } => reason.clone(),
            Self::Eligible {
                green_ledger,
                blue_ledger,
                lag,
            } => format!("eligible (ledger {green_ledger} vs ref {blue_ledger}, lag {lag})"),
        }
    }
}

/// Evaluate whether `candidate` may become the active publisher.
///
/// `reference` is blue during cutover (lag compare) or the candidate itself during
/// rollback when the other color is down (`reference_optional` false path uses self).
pub fn evaluate_cutover_gate(
    candidate: &CoreInfoSnapshot,
    reference: &CoreInfoSnapshot,
    max_ledger_lag: u64,
) -> CutoverGateResult {
    if !candidate.reachable {
        return CutoverGateResult::NotReady {
            reason: "candidate Core /info unreachable".into(),
        };
    }

    match candidate.sync_state {
        CoreSyncState::CatchingUp => {
            return CutoverGateResult::CatchingUp {
                sync_state: CoreSyncState::CatchingUp,
                green_ledger: candidate.ledger,
                blue_ledger: reference.ledger,
            };
        }
        CoreSyncState::Unknown => {
            return CutoverGateResult::NotReady {
                reason: format!(
                    "candidate sync state unknown (raw={})",
                    candidate.raw_state.as_deref().unwrap_or("<none>")
                ),
            };
        }
        CoreSyncState::Synced => {}
    }

    if !candidate.pod_ready {
        return CutoverGateResult::Unhealthy {
            reason: "candidate is Synced but Kubernetes Ready is false".into(),
        };
    }

    let Some(cand_ledger) = candidate.ledger else {
        return CutoverGateResult::Unhealthy {
            reason: "candidate Synced but ledger sequence missing from /info".into(),
        };
    };

    // If reference has a ledger, enforce lag. If not (rollback with peer gone), require Synced only.
    if let Some(ref_ledger) = reference.ledger {
        let lag = ref_ledger.saturating_sub(cand_ledger);
        if lag > max_ledger_lag {
            return CutoverGateResult::CatchingUp {
                sync_state: CoreSyncState::Synced,
                green_ledger: Some(cand_ledger),
                blue_ledger: Some(ref_ledger),
            };
        }
        return CutoverGateResult::Eligible {
            green_ledger: cand_ledger,
            blue_ledger: ref_ledger,
            lag,
        };
    }

    CutoverGateResult::Eligible {
        green_ledger: cand_ledger,
        blue_ledger: cand_ledger,
        lag: 0,
    }
}

/// Rollback success requires Ready + Synced (+ ledger present). Lag vs a dead green is N/A.
pub fn evaluate_rollback_gate(blue: &CoreInfoSnapshot) -> CutoverGateResult {
    evaluate_cutover_gate(blue, blue, u64::MAX)
}

pub fn blue_sts_name(node: &StellarNode) -> String {
    node.name_any()
}
pub fn green_sts_name(node: &StellarNode) -> String {
    format!("{}-green", node.name_any())
}
pub fn blue_pvc_name(node: &StellarNode) -> String {
    resource_name(node, "data")
}
pub fn green_pvc_name(node: &StellarNode) -> String {
    resource_name(node, "green-data")
}
pub fn green_config_name(node: &StellarNode) -> String {
    resource_name(node, "green-config")
}
pub fn green_headless_name(node: &StellarNode) -> String {
    format!("{}-green-headless", node.name_any())
}
pub fn blue_headless_name(node: &StellarNode) -> String {
    format!("{}-headless", node.name_any())
}

fn annotation(node: &StellarNode, key: &str) -> Option<String> {
    node.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(key).cloned())
}

pub fn read_phase(node: &StellarNode) -> CoreBlueGreenPhase {
    node.status
        .as_ref()
        .and_then(|s| s.blue_green_phase.as_deref())
        .map(CoreBlueGreenPhase::parse)
        .or_else(|| annotation(node, ANN_PHASE).map(|s| CoreBlueGreenPhase::parse(&s)))
        .unwrap_or(CoreBlueGreenPhase::BlueActive)
}

pub fn read_active_color(node: &StellarNode) -> String {
    node.status
        .as_ref()
        .and_then(|s| s.blue_green_active_color.clone())
        .or_else(|| annotation(node, ANN_ACTIVE_COLOR))
        .unwrap_or_else(|| COLOR_BLUE.to_string())
}

pub fn read_cutover_step(node: &StellarNode) -> CutoverStep {
    annotation(node, ANN_CUTOVER_STEP)
        .map(|s| CutoverStep::parse(&s))
        .unwrap_or(CutoverStep::ScaleBlueDown)
}

pub fn read_rollback_step(node: &StellarNode) -> RollbackStep {
    annotation(node, ANN_ROLLBACK_STEP)
        .map(|s| RollbackStep::parse(&s))
        .unwrap_or(RollbackStep::StopGreen)
}

pub fn retry_requested(node: &StellarNode) -> bool {
    annotation(node, ANN_RETRY)
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

fn color_labels(node: &StellarNode, color: &str, role: &str) -> BTreeMap<String, String> {
    let mut labels = standard_labels(node);
    labels.insert(COLOR_LABEL.to_string(), color.to_string());
    labels.insert(ROLE_LABEL.to_string(), role.to_string());
    labels
}

pub fn apply_standby_core_config(cfg: &str) -> String {
    let mut out = cfg.to_string();
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str(
        "\n# Stellar-K8s Validator blue/green standby (do not publish while blue is active)\n",
    );
    out.push_str("NODE_IS_VALIDATOR=false\n");
    out
}

pub fn apply_publishing_core_config(cfg: &str) -> String {
    let mut lines: Vec<&str> = cfg
        .lines()
        .filter(|l| {
            let t = l.trim();
            !(t.starts_with("NODE_IS_VALIDATOR=")
                || t.contains("blue/green standby")
                || t.contains("do not publish while blue"))
        })
        .collect();
    lines.push("NODE_IS_VALIDATOR=true");
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Whether a publish-config change should bump the STS rollout annotation.
pub fn publish_rollout_annotation_value(
    publishing: bool,
    previous: Option<&str>,
) -> Option<String> {
    if publishing {
        Some(
            previous
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Utc::now().to_rfc3339()),
        )
    } else {
        None
    }
}

fn volume_snapshot_api_resource() -> ApiResource {
    ApiResource {
        group: "snapshot.storage.k8s.io".to_string(),
        version: "v1".to_string(),
        api_version: "snapshot.storage.k8s.io/v1".to_string(),
        kind: "VolumeSnapshot".to_string(),
        plural: "volumesnapshots".to_string(),
    }
}

fn image_version_tag(image: &str) -> String {
    if let Some(digest) = image.split('@').nth(1) {
        return digest.to_string();
    }
    image.rsplit(':').next().unwrap_or(image).to_string()
}

async fn sts_image_version(client: &Client, namespace: &str, name: &str) -> Result<Option<String>> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    match api.get(name).await {
        Ok(sts) => Ok(sts
            .spec
            .as_ref()
            .and_then(|s| s.template.spec.as_ref())
            .and_then(|ts| ts.containers.first())
            .and_then(|c| c.image.as_ref())
            .map(|img| image_version_tag(img))),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(None),
        Err(e) => Err(Error::KubeError(e)),
    }
}

/// STS is fully down when desired replicas are 0 and no ready pods remain.
pub fn sts_status_fully_down(
    replicas: Option<i32>,
    ready: Option<i32>,
    current: Option<i32>,
) -> bool {
    replicas.unwrap_or(0) == 0 && ready.unwrap_or(0) == 0 && current.unwrap_or(0) == 0
}

async fn sts_fully_down(client: &Client, namespace: &str, name: &str) -> Result<bool> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    match api.get(name).await {
        Ok(sts) => {
            let desired = sts.spec.as_ref().and_then(|s| s.replicas);
            let ready = sts.status.as_ref().and_then(|s| s.ready_replicas);
            let current = sts.status.as_ref().map(|s| s.replicas);
            Ok(sts_status_fully_down(desired, ready, current))
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(true),
        Err(e) => Err(Error::KubeError(e)),
    }
}

async fn no_ready_pods_for_color(client: &Client, node: &StellarNode, color: &str) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let selector = format!(
        "app.kubernetes.io/instance={},{}={}",
        node.name_any(),
        COLOR_LABEL,
        color
    );
    let pods = api
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(Error::KubeError)?;
    Ok(!pods.items.iter().any(pod_ready))
}

async fn blue_fully_down(client: &Client, node: &StellarNode) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let sts_down = sts_fully_down(client, &namespace, &blue_sts_name(node)).await?;
    let pods_down = no_ready_pods_for_color(client, node, COLOR_BLUE).await?;
    Ok(sts_down && pods_down)
}

async fn patch_node_progress(
    client: &Client,
    node: &StellarNode,
    phase: &CoreBlueGreenPhase,
    active_color: &str,
    message: &str,
    target_version: Option<&str>,
    snapshot: Option<&str>,
    cutover_step: Option<&str>,
    rollback_step: Option<&str>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let mut ann = serde_json::Map::new();
    ann.insert(ANN_PHASE.to_string(), serde_json::json!(phase.as_str()));
    ann.insert(
        ANN_ACTIVE_COLOR.to_string(),
        serde_json::json!(active_color),
    );
    if let Some(v) = target_version {
        ann.insert(ANN_TARGET_VERSION.to_string(), serde_json::json!(v));
    }
    if let Some(s) = snapshot {
        ann.insert(ANN_SNAPSHOT.to_string(), serde_json::json!(s));
    }
    if let Some(s) = cutover_step {
        ann.insert(ANN_CUTOVER_STEP.to_string(), serde_json::json!(s));
    }
    if let Some(s) = rollback_step {
        ann.insert(ANN_ROLLBACK_STEP.to_string(), serde_json::json!(s));
    }

    let status = serde_json::json!({
        "blueGreenPhase": phase.as_str(),
        "blueGreenActiveColor": active_color,
        "blueGreenMessage": message,
        "blueGreenTargetVersion": target_version,
        "blueGreenSnapshotName": snapshot,
    });

    let patch = serde_json::json!({
        "metadata": { "annotations": ann },
        "status": status,
    });

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator-bg").force(),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    let _ = api
        .patch_status(
            &name,
            &PatchParams::apply("stellar-operator-bg").force(),
            &Patch::Merge(&serde_json::json!({ "status": status })),
        )
        .await;
    Ok(())
}

async fn clear_retry_annotation(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let patch = serde_json::json!({
        "metadata": { "annotations": { ANN_RETRY: null } }
    });
    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator-bg").force(),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;
    Ok(())
}

pub async fn ensure_active_service_selector(
    client: &Client,
    node: &StellarNode,
    active_color: &str,
    enable_mtls: bool,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let mut svc = build_service(node, enable_mtls);
    if let Some(spec) = svc.spec.as_mut() {
        let mut selector = standard_labels(node);
        selector.insert(COLOR_LABEL.to_string(), active_color.to_string());
        selector.insert(ROLE_LABEL.to_string(), ROLE_ACTIVE.to_string());
        spec.selector = Some(selector);
    }

    let params = if dry_run {
        PatchParams::apply("stellar-operator").force().dry_run()
    } else {
        PatchParams::apply("stellar-operator").force()
    };
    api.patch(&name, &params, &Patch::Apply(&svc))
        .await
        .map_err(Error::KubeError)?;
    Ok(())
}

async fn ensure_headless_service(
    client: &Client,
    node: &StellarNode,
    name: &str,
    color: &str,
    role: &str,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let labels = color_labels(node, color, role);

    let svc = Service {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.to_string()),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(k8s_openapi::api::core::v1::ServiceSpec {
            cluster_ip: Some("None".to_string()),
            selector: Some(labels),
            ports: Some(vec![
                k8s_openapi::api::core::v1::ServicePort {
                    name: Some("peer".to_string()),
                    port: 11625,
                    ..Default::default()
                },
                k8s_openapi::api::core::v1::ServicePort {
                    name: Some("http".to_string()),
                    port: 11626,
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        status: None,
    };

    let params = if dry_run {
        PatchParams::apply("stellar-operator").force().dry_run()
    } else {
        PatchParams::apply("stellar-operator").force()
    };
    api.patch(name, &params, &Patch::Apply(&svc))
        .await
        .map_err(Error::KubeError)?;
    Ok(())
}

fn scale_sts(sts: &mut StatefulSet, replicas: i32) {
    if let Some(spec) = sts.spec.as_mut() {
        spec.replicas = Some(replicas);
    }
}

fn set_sts_identity(
    sts: &mut StatefulSet,
    name: &str,
    labels: BTreeMap<String, String>,
    pvc_name: &str,
    config_name: &str,
    headless: &str,
    publish_rollout: Option<&str>,
) {
    sts.metadata.name = Some(name.to_string());
    sts.metadata.labels = Some(labels.clone());
    if let Some(spec) = sts.spec.as_mut() {
        spec.service_name = headless.to_string();
        spec.selector.match_labels = Some(labels.clone());
        if let Some(tmpl) = spec.template.metadata.as_mut() {
            tmpl.labels = Some(labels);
            if let Some(token) = publish_rollout {
                let mut ann = tmpl.annotations.clone().unwrap_or_default();
                ann.insert(ANN_PUBLISH_ROLLOUT.to_string(), token.to_string());
                tmpl.annotations = Some(ann);
            }
        } else if let Some(token) = publish_rollout {
            let mut ann = BTreeMap::new();
            ann.insert(ANN_PUBLISH_ROLLOUT.to_string(), token.to_string());
            spec.template.metadata = Some(ObjectMeta {
                labels: Some(labels),
                annotations: Some(ann),
                ..Default::default()
            });
        }
        if let Some(pod_spec) = spec.template.spec.as_mut() {
            if let Some(volumes) = pod_spec.volumes.as_mut() {
                for vol in volumes {
                    if vol.name == "data" {
                        if let Some(pvc) = vol.persistent_volume_claim.as_mut() {
                            pvc.claim_name = pvc_name.to_string();
                        }
                    }
                    if vol.name == "config" {
                        if let Some(cm) = vol.config_map.as_mut() {
                            cm.name = Some(config_name.to_string());
                        }
                    }
                }
            }
        }
    }
}

/// Build STS with optional publish-rollout annotation (forces pod restart).
pub fn build_colored_statefulset_for_test(
    node: &StellarNode,
    color: &str,
    role: &str,
    sts_name: &str,
    pvc_name: &str,
    config_name: &str,
    headless: &str,
    replicas: i32,
    publish_rollout: Option<&str>,
) -> StatefulSet {
    let labels = color_labels(node, color, role);
    let mut sts = build_statefulset(node, false, None);
    set_sts_identity(
        &mut sts,
        sts_name,
        labels,
        pvc_name,
        config_name,
        headless,
        publish_rollout,
    );
    scale_sts(&mut sts, replicas);
    sts
}

pub fn sts_has_publish_rollout_annotation(sts: &StatefulSet) -> bool {
    sts.spec
        .as_ref()
        .and_then(|s| s.template.metadata.as_ref())
        .and_then(|m| m.annotations.as_ref())
        .map(|a| a.contains_key(ANN_PUBLISH_ROLLOUT))
        .unwrap_or(false)
}

async fn ensure_colored_statefulset(
    client: &Client,
    node: &StellarNode,
    color: &str,
    role: &str,
    sts_name: &str,
    pvc_name: &str,
    config_name: &str,
    headless: &str,
    replicas: i32,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
    dry_run: bool,
    publish_rollout: Option<&str>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let labels = color_labels(node, color, role);
    let mut sts = build_statefulset(node, enable_mtls, seed_injection);
    set_sts_identity(
        &mut sts,
        sts_name,
        labels,
        pvc_name,
        config_name,
        headless,
        publish_rollout,
    );
    scale_sts(&mut sts, replicas);

    let params = if dry_run {
        PatchParams::apply("stellar-operator").force().dry_run()
    } else {
        PatchParams::apply("stellar-operator").force()
    };
    api.patch(sts_name, &params, &Patch::Apply(&sts))
        .await
        .map_err(Error::KubeError)?;
    Ok(())
}

fn build_green_pvc(
    node: &StellarNode,
    storage_class: String,
    snapshot_name: Option<&str>,
) -> PersistentVolumeClaim {
    let mut pvc = build_pvc(node, storage_class);
    pvc.metadata.name = Some(green_pvc_name(node));
    let mut labels = color_labels(node, COLOR_GREEN, ROLE_STANDBY);
    labels.insert("stellar.org/bg-storage".to_string(), "green".to_string());
    pvc.metadata.labels = Some(labels);

    if let Some(spec) = pvc.spec.as_mut() {
        if let Some(snap) = snapshot_name {
            spec.data_source = Some(TypedLocalObjectReference {
                api_group: Some("snapshot.storage.k8s.io".to_string()),
                kind: "VolumeSnapshot".to_string(),
                name: snap.to_string(),
            });
        } else {
            spec.data_source = None;
        }
    }
    pvc
}

async fn ensure_green_pvc(
    client: &Client,
    node: &StellarNode,
    snapshot_name: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let name = green_pvc_name(node);
    let pvc = build_green_pvc(node, node.spec.storage.storage_class.clone(), snapshot_name);

    match api.get(&name).await {
        Ok(_) => {
            info!("Green PVC {} already exists (immutable dataSource)", name);
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            if !dry_run {
                api.create(&PostParams::default(), &pvc)
                    .await
                    .map_err(Error::KubeError)?;
            }
        }
        Err(e) => return Err(Error::KubeError(e)),
    }
    Ok(())
}

async fn ensure_green_config(
    client: &Client,
    node: &StellarNode,
    publishing: bool,
    enable_mtls: bool,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = green_config_name(node);
    let mut cm = build_config_map(node, None, enable_mtls);
    cm.metadata.name = Some(name.clone());
    cm.metadata.labels = Some(color_labels(
        node,
        COLOR_GREEN,
        if publishing {
            ROLE_ACTIVE
        } else {
            ROLE_STANDBY
        },
    ));

    if let Some(data) = cm.data.as_mut() {
        let raw = data.get("stellar-core.cfg").cloned().unwrap_or_default();
        let updated = if publishing {
            apply_publishing_core_config(&raw)
        } else {
            apply_standby_core_config(&raw)
        };
        data.insert("stellar-core.cfg".to_string(), updated);
    }

    let params = if dry_run {
        PatchParams::apply("stellar-operator").force().dry_run()
    } else {
        PatchParams::apply("stellar-operator").force()
    };
    api.patch(&name, &params, &Patch::Apply(&cm))
        .await
        .map_err(Error::KubeError)?;
    Ok(())
}

async fn create_cutover_snapshot_from_pvc(
    client: &Client,
    node: &StellarNode,
    config: &BlueGreenStrategyConfig,
    pvc_name: &str,
    source_label: &str,
) -> Result<String> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let snapshot_name = format!(
        "{}-bg-{}-{}",
        node.name_any(),
        source_label,
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let api_resource = volume_snapshot_api_resource();
    let api: Api<kube::api::DynamicObject> =
        Api::namespaced_with(client.clone(), &namespace, &api_resource);

    let mut labels = standard_labels(node);
    labels.insert("stellar.org/snapshot-of".to_string(), node.name_any());
    labels.insert("stellar.org/bg-snapshot".to_string(), "true".to_string());

    let snapshot = kube::api::DynamicObject {
        types: Some(kube::core::TypeMeta {
            api_version: api_resource.api_version.clone(),
            kind: api_resource.kind.clone(),
        }),
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(snapshot_name.clone()),
                namespace: Some(namespace),
                labels: Some(labels),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        data: serde_json::json!({
            "spec": {
                "source": { "persistentVolumeClaimName": pvc_name },
                "volumeSnapshotClassName": config.volume_snapshot_class_name,
            }
        }),
    };

    match api.get(&snapshot_name).await {
        Ok(_) => Ok(snapshot_name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            api.create(&PostParams::default(), &snapshot)
                .await
                .map_err(Error::KubeError)?;
            Ok(snapshot_name)
        }
        Err(e) => Err(Error::KubeError(e)),
    }
}

async fn snapshot_ready(client: &Client, namespace: &str, name: &str) -> Result<bool> {
    let api_resource = volume_snapshot_api_resource();
    let api: Api<kube::api::DynamicObject> =
        Api::namespaced_with(client.clone(), namespace, &api_resource);
    match api.get(name).await {
        Ok(obj) => Ok(obj
            .data
            .get("status")
            .and_then(|s| s.get("readyToUse"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
        Err(e) => Err(Error::KubeError(e)),
    }
}

#[derive(Debug, Deserialize)]
struct CoreInfoResponse {
    info: CoreInfoBody,
}
#[derive(Debug, Deserialize)]
struct CoreInfoBody {
    state: String,
    #[serde(default)]
    ledger: Option<CoreLedger>,
}
#[derive(Debug, Deserialize)]
struct CoreLedger {
    num: u64,
}

async fn query_pod_core_info(pod_ip: &str) -> Result<(CoreSyncState, Option<u64>, String)> {
    let url = format!("http://{pod_ip}:11626/info");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::ConfigError(format!("HTTP client: {e}")))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::ConfigError(format!("/info unreachable: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::ConfigError(format!("/info HTTP {}", resp.status())));
    }
    let body: CoreInfoResponse = resp
        .json()
        .await
        .map_err(|e| Error::ConfigError(format!("parse /info: {e}")))?;
    Ok((
        parse_sync_state(&body.info.state),
        body.info.ledger.map(|l| l.num),
        body.info.state,
    ))
}

fn pod_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|cs| cs.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
        .unwrap_or(false)
}

async fn list_pods_for_color(client: &Client, node: &StellarNode, color: &str) -> Result<Vec<Pod>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let selector = format!(
        "app.kubernetes.io/instance={},{}={}",
        node.name_any(),
        COLOR_LABEL,
        color
    );
    let pods = api
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(Error::KubeError)?;
    Ok(pods.items)
}

/// Observed publish-rollout token on the current green pod (prefer Ready).
async fn green_observed_publish_rollout(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<String>> {
    let pods = list_pods_for_color(client, node, COLOR_GREEN).await?;
    let pod = pods.iter().find(|p| pod_ready(p)).or_else(|| pods.first());
    Ok(pod.and_then(pod_publish_rollout_token))
}

async fn snapshot_for_color(
    client: &Client,
    node: &StellarNode,
    color: &str,
) -> Result<CoreInfoSnapshot> {
    let pods = list_pods_for_color(client, node, color).await?;

    let Some(pod) = pods.first() else {
        return Ok(CoreInfoSnapshot {
            sync_state: CoreSyncState::Unknown,
            ledger: None,
            pod_ready: false,
            reachable: false,
            raw_state: None,
        });
    };

    let ready = pod_ready(pod);
    let Some(ip) = pod.status.as_ref().and_then(|s| s.pod_ip.clone()) else {
        return Ok(CoreInfoSnapshot {
            sync_state: CoreSyncState::Unknown,
            ledger: None,
            pod_ready: ready,
            reachable: false,
            raw_state: None,
        });
    };

    match query_pod_core_info(&ip).await {
        Ok((sync_state, ledger, raw)) => Ok(CoreInfoSnapshot {
            sync_state,
            ledger,
            pod_ready: ready,
            reachable: true,
            raw_state: Some(raw),
        }),
        Err(e) => {
            warn!("Core /info query failed for {} {}: {}", color, ip, e);
            Ok(CoreInfoSnapshot {
                sync_state: CoreSyncState::Unknown,
                ledger: None,
                pod_ready: ready,
                reachable: false,
                raw_state: None,
            })
        }
    }
}

async fn delete_optional(client: &Client, namespace: &str, kind: &str, name: &str) -> Result<()> {
    match kind {
        "sts" => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            match api.delete(name, &DeleteParams::default()).await {
                Ok(_) => Ok(()),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
        "cm" => {
            let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
            match api.delete(name, &DeleteParams::default()).await {
                Ok(_) => Ok(()),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
        "svc" => {
            let api: Api<Service> = Api::namespaced(client.clone(), namespace);
            match api.delete(name, &DeleteParams::default()).await {
                Ok(_) => Ok(()),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
        // Never delete PVCs via this helper - rollback storage must be retained.
        "pvc" => Ok(()),
        _ => Ok(()),
    }
}

pub async fn cleanup_green_resources(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    delete_optional(client, &namespace, "sts", &green_sts_name(node)).await?;
    delete_optional(client, &namespace, "cm", &green_config_name(node)).await?;
    delete_optional(client, &namespace, "svc", &green_headless_name(node)).await?;
    Ok(())
}

fn rollout_timed_out(node: &StellarNode, config: &BlueGreenStrategyConfig) -> bool {
    let started = annotation(node, ANN_STARTED_AT)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Utc));
    let Some(started) = started else {
        return false;
    };
    Utc::now().signed_duration_since(started).num_seconds() > config.ready_timeout_seconds as i64
}

/// Idempotent Validator blue/green reconciler.
#[instrument(skip(client, node, seed_injection), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn reconcile_validator_blue_green(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
    dry_run: bool,
) -> Result<()> {
    if node.spec.node_type != NodeType::Validator {
        return Ok(());
    }
    if node.spec.strategy.strategy_type != RolloutStrategyType::BlueGreen {
        return Ok(());
    }

    let config = node.spec.strategy.blue_green_or_default();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let desired_version = node.spec.version.clone();
    let mut phase = read_phase(node);
    let active_color = read_active_color(node);

    // Explicit retry from Failed.
    if phase == CoreBlueGreenPhase::Failed && retry_requested(node) {
        clear_retry_annotation(client, node).await?;
        phase = CoreBlueGreenPhase::BlueActive;
        patch_node_progress(
            client,
            node,
            &CoreBlueGreenPhase::BlueActive,
            COLOR_BLUE,
            "retry requested; re-entering blue/green from BlueActive",
            None,
            None,
            None,
            None,
        )
        .await?;
    }

    ensure_headless_service(
        client,
        node,
        &blue_headless_name(node),
        COLOR_BLUE,
        if active_color == COLOR_BLUE {
            ROLE_ACTIVE
        } else {
            ROLE_STANDBY
        },
        dry_run,
    )
    .await?;

    let blue_version = sts_image_version(client, &namespace, &blue_sts_name(node))
        .await?
        .or_else(|| annotation(node, ANN_BLUE_VERSION));

    // Green already active: further upgrades are deferred (no flip-flop / no PVC delete).
    if matches!(
        phase,
        CoreBlueGreenPhase::GreenActive
            | CoreBlueGreenPhase::UpgradeDeferred
            | CoreBlueGreenPhase::RollingBack
    ) || active_color == COLOR_GREEN
    {
        if phase != CoreBlueGreenPhase::RollingBack {
            let green_ver = sts_image_version(client, &namespace, &green_sts_name(node)).await?;
            if green_ver.is_some() && green_ver.as_ref() != Some(&desired_version) {
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::UpgradeDeferred,
                    COLOR_GREEN,
                    "green is active; further blue/green upgrades are deferred until a safe storage lifecycle exists. Do not delete PVCs. Consolidate manually or use rollingUpdate after returning to a single active STS.",
                    Some(&desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    None,
                )
                .await?;
                ensure_green_config(client, node, true, enable_mtls, dry_run).await?;
                let token = annotation(node, ANN_PUBLISH_ROLLOUT);
                ensure_colored_statefulset(
                    client,
                    node,
                    COLOR_GREEN,
                    ROLE_ACTIVE,
                    &green_sts_name(node),
                    &green_pvc_name(node),
                    &green_config_name(node),
                    &green_headless_name(node),
                    if node.spec.suspended { 0 } else { 1 },
                    enable_mtls,
                    seed_injection,
                    dry_run,
                    token.as_deref(),
                )
                .await?;
                ensure_active_service_selector(client, node, COLOR_GREEN, enable_mtls, dry_run)
                    .await?;
                let mut blue_node = node.clone();
                if let Some(v) = annotation(node, ANN_BLUE_VERSION) {
                    if !v.is_empty() {
                        blue_node.spec.version = v;
                    }
                }
                ensure_colored_statefulset(
                    client,
                    &blue_node,
                    COLOR_BLUE,
                    ROLE_STANDBY,
                    &blue_sts_name(node),
                    &blue_pvc_name(node),
                    &resource_name(node, "config"),
                    &blue_headless_name(node),
                    0,
                    enable_mtls,
                    seed_injection,
                    dry_run,
                    None,
                )
                .await?;
                return Ok(());
            }
            if phase == CoreBlueGreenPhase::UpgradeDeferred {
                return Ok(());
            }
            // Matched desired version (or unknown): maintain green below.
            phase = CoreBlueGreenPhase::GreenActive;
        }
    }

    let needs_upgrade = match &blue_version {
        Some(v) => v != &desired_version && active_color == COLOR_BLUE,
        None => false,
    };

    if phase == CoreBlueGreenPhase::Failed {
        // Stable failure: keep blue active; do not recreate snapshots/PVCs.
        ensure_colored_statefulset(
            client,
            node,
            COLOR_BLUE,
            ROLE_ACTIVE,
            &blue_sts_name(node),
            &blue_pvc_name(node),
            &resource_name(node, "config"),
            &blue_headless_name(node),
            1,
            enable_mtls,
            seed_injection,
            dry_run,
            None,
        )
        .await?;
        ensure_active_service_selector(client, node, COLOR_BLUE, enable_mtls, dry_run).await?;
        return Ok(());
    }

    if phase == CoreBlueGreenPhase::UpgradeDeferred {
        return Ok(());
    }

    if phase == CoreBlueGreenPhase::BlueActive && !needs_upgrade {
        ensure_colored_statefulset(
            client,
            node,
            COLOR_BLUE,
            ROLE_ACTIVE,
            &blue_sts_name(node),
            &blue_pvc_name(node),
            &resource_name(node, "config"),
            &blue_headless_name(node),
            if node.spec.suspended { 0 } else { 1 },
            enable_mtls,
            seed_injection,
            dry_run,
            None,
        )
        .await?;
        ensure_active_service_selector(client, node, COLOR_BLUE, enable_mtls, dry_run).await?;
        patch_node_progress(
            client,
            node,
            &CoreBlueGreenPhase::BlueActive,
            COLOR_BLUE,
            "blue active; no blue/green upgrade in progress",
            None,
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    if phase == CoreBlueGreenPhase::BlueActive && needs_upgrade {
        phase = CoreBlueGreenPhase::PreparingGreen;
        let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
        let _ = api
            .patch(
                &node.name_any(),
                &PatchParams::apply("stellar-operator-bg").force(),
                &Patch::Merge(&serde_json::json!({
                    "metadata": {
                        "annotations": {
                            ANN_STARTED_AT: Utc::now().to_rfc3339(),
                            ANN_BLUE_VERSION: blue_version.clone().unwrap_or_default(),
                            ANN_TARGET_VERSION: desired_version.clone(),
                            ANN_PHASE: CoreBlueGreenPhase::PreparingGreen.as_str(),
                            ANN_CUTOVER_STEP: null,
                            ANN_ROLLBACK_STEP: null,
                        }
                    }
                })),
            )
            .await;
    }

    // Keep blue publishing while preparing/waiting (not during CuttingOver).
    if matches!(
        phase,
        CoreBlueGreenPhase::PreparingGreen | CoreBlueGreenPhase::WaitingForGreen
    ) {
        let mut blue_node = node.clone();
        if let Some(v) = annotation(node, ANN_BLUE_VERSION).or(blue_version.clone()) {
            if !v.is_empty() {
                blue_node.spec.version = v;
            }
        }
        ensure_colored_statefulset(
            client,
            &blue_node,
            COLOR_BLUE,
            ROLE_ACTIVE,
            &blue_sts_name(node),
            &blue_pvc_name(node),
            &resource_name(node, "config"),
            &blue_headless_name(node),
            1,
            enable_mtls,
            seed_injection,
            dry_run,
            None,
        )
        .await?;
        ensure_active_service_selector(client, node, COLOR_BLUE, enable_mtls, dry_run).await?;
    }

    match phase {
        CoreBlueGreenPhase::PreparingGreen => {
            let snapshot_name = if config.require_volume_snapshot {
                let existing = annotation(node, ANN_SNAPSHOT);
                let snap = match existing {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        create_cutover_snapshot_from_pvc(
                            client,
                            node,
                            &config,
                            &blue_pvc_name(node),
                            "blue",
                        )
                        .await?
                    }
                };
                if !snapshot_ready(client, &namespace, &snap).await? {
                    patch_node_progress(
                        client,
                        node,
                        &CoreBlueGreenPhase::PreparingGreen,
                        COLOR_BLUE,
                        &format!("waiting for VolumeSnapshot {snap} to become ready"),
                        Some(&desired_version),
                        Some(&snap),
                        None,
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                Some(snap)
            } else {
                None
            };

            ensure_green_pvc(client, node, snapshot_name.as_deref(), dry_run).await?;
            ensure_green_config(client, node, false, enable_mtls, dry_run).await?;
            ensure_headless_service(
                client,
                node,
                &green_headless_name(node),
                COLOR_GREEN,
                ROLE_STANDBY,
                dry_run,
            )
            .await?;
            ensure_colored_statefulset(
                client,
                node,
                COLOR_GREEN,
                ROLE_STANDBY,
                &green_sts_name(node),
                &green_pvc_name(node),
                &green_config_name(node),
                &green_headless_name(node),
                1,
                enable_mtls,
                seed_injection,
                dry_run,
                None,
            )
            .await?;

            patch_node_progress(
                client,
                node,
                &CoreBlueGreenPhase::WaitingForGreen,
                COLOR_BLUE,
                "green standby created (NODE_IS_VALIDATOR=false); waiting for sync/ledger gate",
                Some(&desired_version),
                snapshot_name.as_deref(),
                None,
                None,
            )
            .await?;
        }

        CoreBlueGreenPhase::WaitingForGreen => {
            if rollout_timed_out(node, &config) {
                let _ = cleanup_green_resources(client, node).await;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::Failed,
                    COLOR_BLUE,
                    "green failed readyTimeoutSeconds; blue remains active. Set annotation stellar.org/bg-retry=true to retry.",
                    Some(&desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    None,
                )
                .await?;
                return Ok(());
            }

            let green = snapshot_for_color(client, node, COLOR_GREEN).await?;
            let blue = snapshot_for_color(client, node, COLOR_BLUE).await?;
            let gate = evaluate_cutover_gate(&green, &blue, config.max_ledger_lag);
            if !gate.is_eligible() {
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::WaitingForGreen,
                    COLOR_BLUE,
                    &format!("green standby not eligible: {}", gate.reason()),
                    Some(&desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    None,
                )
                .await?;
                return Ok(());
            }

            // Enter serialized cutover; Service still on blue.
            patch_node_progress(
                client,
                node,
                &CoreBlueGreenPhase::CuttingOver,
                COLOR_BLUE,
                "green eligible; starting serialized cutover (Service still on blue)",
                Some(&desired_version),
                annotation(node, ANN_SNAPSHOT).as_deref(),
                Some(CutoverStep::ScaleBlueDown.as_str()),
                None,
            )
            .await?;
            run_cutover_steps(
                client,
                node,
                &config,
                enable_mtls,
                seed_injection,
                dry_run,
                &desired_version,
            )
            .await?;
        }

        CoreBlueGreenPhase::CuttingOver => {
            run_cutover_steps(
                client,
                node,
                &config,
                enable_mtls,
                seed_injection,
                dry_run,
                &desired_version,
            )
            .await?;
        }

        CoreBlueGreenPhase::GreenActive => {
            maintain_green_active(client, node, &config, enable_mtls, seed_injection, dry_run)
                .await?;
        }

        CoreBlueGreenPhase::RollingBack => {
            run_rollback_steps(client, node, &config, enable_mtls, seed_injection, dry_run).await?;
        }

        CoreBlueGreenPhase::BlueActive
        | CoreBlueGreenPhase::Failed
        | CoreBlueGreenPhase::UpgradeDeferred => {}
    }

    Ok(())
}

async fn run_cutover_steps(
    client: &Client,
    node: &StellarNode,
    config: &BlueGreenStrategyConfig,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
    dry_run: bool,
    desired_version: &str,
) -> Result<()> {
    let mut step = read_cutover_step(node);
    // Track expected publish token locally: node object may lag behind our own patches.
    let mut expected_rollout = annotation(node, ANN_PUBLISH_ROLLOUT);
    // Cap work per reconcile to keep idempotent progress.
    for _ in 0..4 {
        let blue_down = blue_fully_down(client, node).await?;
        let green = snapshot_for_color(client, node, COLOR_GREEN).await?;
        // During WaitGreenHealthy, blue is down - lag vs blue ledger may be absent; gate still requires Synced.
        let blue_ref = snapshot_for_color(client, node, COLOR_BLUE).await?;
        let health_eligible =
            evaluate_cutover_gate(&green, &blue_ref, config.max_ledger_lag).is_eligible();
        let observed_rollout = green_observed_publish_rollout(client, node).await?;

        // Hard safety: never enable publish or switch service while blue is up;
        // Service switch also requires observed publish-rollout on the green pod.
        let (clamped_step, green_eligible) = enforce_cutover_safety(
            step.clone(),
            blue_down,
            health_eligible,
            expected_rollout.as_deref(),
            observed_rollout.as_deref(),
        );
        step = clamped_step;

        let (next, cmd) = plan_cutover_advance(step.clone(), blue_down, green_eligible);

        match cmd {
            CutoverCommand::ScaleBlueToZero => {
                let mut blue_node = node.clone();
                if let Some(v) = annotation(node, ANN_BLUE_VERSION) {
                    if !v.is_empty() {
                        blue_node.spec.version = v;
                    }
                }
                ensure_colored_statefulset(
                    client,
                    &blue_node,
                    COLOR_BLUE,
                    ROLE_STANDBY,
                    &blue_sts_name(node),
                    &blue_pvc_name(node),
                    &resource_name(node, "config"),
                    &blue_headless_name(node),
                    0,
                    enable_mtls,
                    seed_injection,
                    dry_run,
                    None,
                )
                .await?;
                // Keep Service on blue until green is healthy (may have empty endpoints briefly).
                ensure_active_service_selector(client, node, COLOR_BLUE, enable_mtls, dry_run)
                    .await?;
                // Green must remain standby/non-publishing while blue was active.
                ensure_green_config(client, node, false, enable_mtls, dry_run).await?;
                step = next;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::CuttingOver,
                    COLOR_BLUE,
                    "scaled blue to 0; waiting for blue pods to terminate before green publishes",
                    Some(desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    Some(step.as_str()),
                    None,
                )
                .await?;
            }
            CutoverCommand::EnableGreenPublishingAndRestart => {
                if !blue_down {
                    step = CutoverStep::WaitBlueDown;
                    continue;
                }
                let rollout_token = Utc::now().to_rfc3339();
                ensure_green_config(client, node, true, enable_mtls, dry_run).await?;
                ensure_colored_statefulset(
                    client,
                    node,
                    COLOR_GREEN,
                    ROLE_ACTIVE,
                    &green_sts_name(node),
                    &green_pvc_name(node),
                    &green_config_name(node),
                    &green_headless_name(node),
                    1,
                    enable_mtls,
                    seed_injection,
                    dry_run,
                    Some(&rollout_token),
                )
                .await?;
                // Persist publish token on the node for maintenance / observation checks.
                let api: Api<StellarNode> = Api::namespaced(
                    client.clone(),
                    &node.namespace().unwrap_or_else(|| "default".to_string()),
                );
                let _ = api
                    .patch(
                        &node.name_any(),
                        &PatchParams::apply("stellar-operator-bg").force(),
                        &Patch::Merge(&serde_json::json!({
                            "metadata": { "annotations": { ANN_PUBLISH_ROLLOUT: rollout_token } }
                        })),
                    )
                    .await;
                expected_rollout = Some(rollout_token);
                step = CutoverStep::WaitGreenHealthy;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::CuttingOver,
                    COLOR_BLUE,
                    "green publishing enabled with forced STS rollout; waiting for Ready+Synced and observed publish-rollout before Service switch",
                    Some(desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    Some(step.as_str()),
                    None,
                )
                .await?;
            }
            CutoverCommand::SwitchServiceToGreenAndFinish => {
                if !green_ready_for_service_switch(
                    blue_down,
                    health_eligible,
                    expected_rollout.as_deref(),
                    observed_rollout.as_deref(),
                ) {
                    step = CutoverStep::WaitGreenHealthy;
                    continue;
                }
                ensure_active_service_selector(client, node, COLOR_GREEN, enable_mtls, dry_run)
                    .await?;
                let api: Api<StellarNode> = Api::namespaced(
                    client.clone(),
                    &node.namespace().unwrap_or_else(|| "default".to_string()),
                );
                let _ = api
                    .patch(
                        &node.name_any(),
                        &PatchParams::apply("stellar-operator-bg").force(),
                        &Patch::Merge(&serde_json::json!({
                            "metadata": {
                                "annotations": {
                                    ANN_CUTOVER_AT: Utc::now().to_rfc3339(),
                                    ANN_CUTOVER_STEP: CutoverStep::Complete.as_str(),
                                }
                            }
                        })),
                    )
                    .await;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::GreenActive,
                    COLOR_GREEN,
                    "cutover complete; Service on green. Blue retained at replicas=0 for rollback (PVC preserved).",
                    Some(desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    Some(CutoverStep::Complete.as_str()),
                    None,
                )
                .await?;
                return Ok(());
            }
            CutoverCommand::Wait => {
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::CuttingOver,
                    COLOR_BLUE,
                    &format!(
                        "cutover step {} (blue_down={blue_down}, health_eligible={health_eligible}, rollout_observed={}, green_eligible={green_eligible})",
                        step.as_str(),
                        green_publish_rollout_observed(
                            expected_rollout.as_deref(),
                            observed_rollout.as_deref(),
                        ),
                    ),
                    Some(desired_version),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    Some(step.as_str()),
                    None,
                )
                .await?;
                return Ok(());
            }
        }

        if matches!(step, CutoverStep::Complete) {
            return Ok(());
        }
        // Continue loop only when we made progress within the same reconcile.
        if matches!(cmd, CutoverCommand::Wait) {
            return Ok(());
        }
    }
    Ok(())
}

async fn maintain_green_active(
    client: &Client,
    node: &StellarNode,
    config: &BlueGreenStrategyConfig,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
    dry_run: bool,
) -> Result<()> {
    let token = annotation(node, ANN_PUBLISH_ROLLOUT);
    ensure_green_config(client, node, true, enable_mtls, dry_run).await?;
    ensure_colored_statefulset(
        client,
        node,
        COLOR_GREEN,
        ROLE_ACTIVE,
        &green_sts_name(node),
        &green_pvc_name(node),
        &green_config_name(node),
        &green_headless_name(node),
        if node.spec.suspended { 0 } else { 1 },
        enable_mtls,
        seed_injection,
        dry_run,
        token.as_deref(),
    )
    .await?;
    ensure_active_service_selector(client, node, COLOR_GREEN, enable_mtls, dry_run).await?;

    // Retain blue scaled down; never delete PVC.
    let mut blue_node = node.clone();
    if let Some(v) = annotation(node, ANN_BLUE_VERSION) {
        if !v.is_empty() {
            blue_node.spec.version = v;
        }
    }
    ensure_colored_statefulset(
        client,
        &blue_node,
        COLOR_BLUE,
        ROLE_STANDBY,
        &blue_sts_name(node),
        &blue_pvc_name(node),
        &resource_name(node, "config"),
        &blue_headless_name(node),
        0,
        enable_mtls,
        seed_injection,
        dry_run,
        None,
    )
    .await?;

    let green = snapshot_for_color(client, node, COLOR_GREEN).await?;
    if !evaluate_rollback_gate(&green).is_eligible() {
        warn!(
            "Post-cutover green unhealthy for {}/{}; starting serialized rollback",
            node.namespace().unwrap_or_default(),
            node.name_any()
        );
        patch_node_progress(
            client,
            node,
            &CoreBlueGreenPhase::RollingBack,
            COLOR_GREEN,
            "post-cutover green health failed; rolling back (Service still on green until blue Ready+Synced)",
            Some(&node.spec.version),
            annotation(node, ANN_SNAPSHOT).as_deref(),
            None,
            Some(RollbackStep::StopGreen.as_str()),
        )
        .await?;
        return run_rollback_steps(client, node, config, enable_mtls, seed_injection, dry_run)
            .await;
    }

    patch_node_progress(
        client,
        node,
        &CoreBlueGreenPhase::GreenActive,
        COLOR_GREEN,
        "green active and healthy; blue retained at 0 for rollback",
        Some(&node.spec.version),
        annotation(node, ANN_SNAPSHOT).as_deref(),
        Some(CutoverStep::Complete.as_str()),
        None,
    )
    .await?;
    Ok(())
}

async fn run_rollback_steps(
    client: &Client,
    node: &StellarNode,
    config: &BlueGreenStrategyConfig,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
    dry_run: bool,
) -> Result<()> {
    let mut step = read_rollback_step(node);
    for _ in 0..4 {
        let blue = snapshot_for_color(client, node, COLOR_BLUE).await?;
        let blue_eligible =
            evaluate_cutover_gate(&blue, &blue, config.max_ledger_lag).is_eligible();
        let (next, cmd) = plan_rollback_advance(step.clone(), blue_eligible);

        match cmd {
            RollbackCommand::ScaleGreenToZero => {
                ensure_green_config(client, node, false, enable_mtls, dry_run).await?;
                ensure_colored_statefulset(
                    client,
                    node,
                    COLOR_GREEN,
                    ROLE_STANDBY,
                    &green_sts_name(node),
                    &green_pvc_name(node),
                    &green_config_name(node),
                    &green_headless_name(node),
                    0,
                    enable_mtls,
                    seed_injection,
                    dry_run,
                    None,
                )
                .await?;
                step = next;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::RollingBack,
                    COLOR_GREEN,
                    "rollback: green scaled to 0 / non-publishing; scaling blue up (Service still on green)",
                    annotation(node, ANN_TARGET_VERSION).as_deref(),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    Some(step.as_str()),
                )
                .await?;
            }
            RollbackCommand::ScaleBlueToOne => {
                let mut blue_node = node.clone();
                if let Some(v) = annotation(node, ANN_BLUE_VERSION) {
                    if !v.is_empty() {
                        blue_node.spec.version = v;
                    }
                }
                ensure_colored_statefulset(
                    client,
                    &blue_node,
                    COLOR_BLUE,
                    ROLE_ACTIVE,
                    &blue_sts_name(node),
                    &blue_pvc_name(node),
                    &resource_name(node, "config"),
                    &blue_headless_name(node),
                    1,
                    enable_mtls,
                    seed_injection,
                    dry_run,
                    None,
                )
                .await?;
                step = next;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::RollingBack,
                    COLOR_GREEN,
                    "rollback: blue scaled to 1; waiting for Ready+Synced before Service switch",
                    annotation(node, ANN_TARGET_VERSION).as_deref(),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    Some(step.as_str()),
                )
                .await?;
            }
            RollbackCommand::SwitchServiceToBlueAndFinish => {
                if !blue_eligible {
                    step = RollbackStep::WaitBlueHealthy;
                    continue;
                }
                ensure_active_service_selector(client, node, COLOR_BLUE, enable_mtls, dry_run)
                    .await?;
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::BlueActive,
                    COLOR_BLUE,
                    "rollback complete; Service on blue after Ready+Synced. Green retained scaled to 0 (PVC preserved).",
                    annotation(node, ANN_TARGET_VERSION).as_deref(),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    Some(RollbackStep::Complete.as_str()),
                )
                .await?;
                return Ok(());
            }
            RollbackCommand::Wait => {
                patch_node_progress(
                    client,
                    node,
                    &CoreBlueGreenPhase::RollingBack,
                    COLOR_GREEN,
                    &format!(
                        "rollback step {} waiting for blue Ready+Synced (eligible={blue_eligible})",
                        step.as_str()
                    ),
                    annotation(node, ANN_TARGET_VERSION).as_deref(),
                    annotation(node, ANN_SNAPSHOT).as_deref(),
                    None,
                    Some(step.as_str()),
                )
                .await?;
                return Ok(());
            }
        }
    }
    Ok(())
}

pub fn should_take_over_validator_workload(node: &StellarNode) -> bool {
    node.spec.node_type == NodeType::Validator
        && node.spec.strategy.strategy_type == RolloutStrategyType::BlueGreen
}

pub fn storage_identities(node: &StellarNode) -> (String, String) {
    (blue_pvc_name(node), green_pvc_name(node))
}

/// PVC names that must never be deleted by blue/green automation.
pub fn protected_pvc_names(node: &StellarNode) -> Vec<String> {
    vec![blue_pvc_name(node), green_pvc_name(node)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{StellarNetwork, StellarNodeSpec, ValidatorConfig};
    use kube::core::ObjectMeta;

    fn sample_node() -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("validator-1".to_string()),
                namespace: Some("stellar".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.1.0".to_string(),
                validator_config: Some(ValidatorConfig {
                    seed_secret_ref: "seed".to_string(),
                    ..Default::default()
                }),
                strategy: crate::crd::types::RolloutStrategy {
                    strategy_type: RolloutStrategyType::BlueGreen,
                    canary: None,
                    blue_green: Some(BlueGreenStrategyConfig::default()),
                },
                ..Default::default()
            },
            status: None,
        }
    }

    fn snap(
        sync: CoreSyncState,
        ledger: Option<u64>,
        ready: bool,
        reachable: bool,
    ) -> CoreInfoSnapshot {
        let raw_state = Some(format!("{sync:?}"));
        CoreInfoSnapshot {
            sync_state: sync,
            ledger,
            pod_ready: ready,
            reachable,
            raw_state,
        }
    }

    #[test]
    fn storage_identities_are_distinct() {
        let node = sample_node();
        let (blue, green) = storage_identities(&node);
        assert_ne!(blue, green);
        assert!(protected_pvc_names(&node).contains(&blue));
        assert!(protected_pvc_names(&node).contains(&green));
    }

    #[test]
    fn standby_config_disables_validator() {
        let cfg = apply_standby_core_config("NETWORK_PASSPHRASE=\"Test\"\n");
        assert!(cfg.contains("NODE_IS_VALIDATOR=false"));
        let pub_cfg = apply_publishing_core_config(&cfg);
        assert!(pub_cfg.contains("NODE_IS_VALIDATOR=true"));
        assert!(!pub_cfg.contains("NODE_IS_VALIDATOR=false"));
    }

    #[test]
    fn green_cannot_publish_while_blue_active() {
        assert!(green_must_stay_standby(false));
        assert!(!green_must_stay_standby(true));
        assert!(!may_switch_service_to_green(false, true));
        assert!(!may_switch_service_to_green(true, false));
        assert!(may_switch_service_to_green(true, true));
    }

    /// A: while blue is still active, executor must block publish/switch steps.
    #[test]
    fn executor_blocks_publish_and_switch_while_blue_still_active() {
        let token = "rollout-2026";
        for bad_step in [
            CutoverStep::EnableGreenPublish,
            CutoverStep::WaitGreenHealthy,
            CutoverStep::SwitchService,
        ] {
            let (step, eligible) = enforce_cutover_safety(
                bad_step,
                false, // blue NOT fully down
                true,  // health would otherwise pass
                Some(token),
                Some(token),
            );
            assert_eq!(
                step,
                CutoverStep::WaitBlueDown,
                "publish/switch steps must clamp to WaitBlueDown while blue is up"
            );
            // Even with matching rollout, eligibility for switch is false when blue is up
            // once composed with may_switch / plan.
            let (_, cmd) = plan_cutover_advance(step, false, eligible);
            assert_eq!(cmd, CutoverCommand::Wait);
            assert!(!green_ready_for_service_switch(
                false,
                true,
                Some(token),
                Some(token)
            ));
        }
    }

    /// B: once blue is fully down, publish/restart may proceed.
    #[test]
    fn executor_allows_publish_restart_once_blue_fully_down() {
        let (step, eligible) =
            enforce_cutover_safety(CutoverStep::WaitBlueDown, true, false, None, None);
        assert_eq!(step, CutoverStep::WaitBlueDown);
        let (next, cmd) = plan_cutover_advance(step, true, eligible);
        assert_eq!(cmd, CutoverCommand::EnableGreenPublishingAndRestart);
        assert_eq!(next, CutoverStep::EnableGreenPublish);
        // Guard must NOT clamp EnableGreenPublish when blue is down.
        let (clamped, _) =
            enforce_cutover_safety(CutoverStep::EnableGreenPublish, true, false, None, None);
        assert_eq!(clamped, CutoverStep::EnableGreenPublish);
    }

    /// C: Ready+Synced with a stale/missing rollout token must not allow Service switch.
    #[test]
    fn service_switch_rejected_for_healthy_but_old_green_rollout() {
        let expected = "rollout-new";
        let old = "rollout-old";
        assert!(!green_publish_rollout_observed(Some(expected), Some(old)));
        assert!(!green_publish_rollout_observed(Some(expected), None));
        assert!(!green_publish_rollout_observed(None, Some(expected)));

        let (step, eligible) = enforce_cutover_safety(
            CutoverStep::WaitGreenHealthy,
            true,
            true, // health OK
            Some(expected),
            Some(old),
        );
        assert_eq!(step, CutoverStep::WaitGreenHealthy);
        assert!(!eligible);
        let (_, cmd) = plan_cutover_advance(step, true, eligible);
        assert_eq!(cmd, CutoverCommand::Wait);
        assert_ne!(cmd, CutoverCommand::SwitchServiceToGreenAndFinish);
        assert!(!green_ready_for_service_switch(
            true,
            true,
            Some(expected),
            Some(old)
        ));
    }

    /// D: Ready+Synced with the current rollout token may proceed to Service switch.
    #[test]
    fn service_switch_allowed_for_healthy_green_with_current_rollout() {
        let token = "rollout-current";
        assert!(green_publish_rollout_observed(Some(token), Some(token)));

        let (step, eligible) = enforce_cutover_safety(
            CutoverStep::WaitGreenHealthy,
            true,
            true,
            Some(token),
            Some(token),
        );
        assert_eq!(step, CutoverStep::WaitGreenHealthy);
        assert!(eligible);
        let (next, cmd) = plan_cutover_advance(step, true, eligible);
        assert_eq!(cmd, CutoverCommand::SwitchServiceToGreenAndFinish);
        assert_eq!(next, CutoverStep::SwitchService);
        assert!(green_ready_for_service_switch(
            true,
            true,
            Some(token),
            Some(token)
        ));
    }

    #[test]
    fn pod_publish_rollout_token_reads_annotation() {
        let mut pod = Pod::default();
        assert!(pod_publish_rollout_token(&pod).is_none());
        pod.metadata.annotations = Some(BTreeMap::from([(
            ANN_PUBLISH_ROLLOUT.to_string(),
            "tok-1".to_string(),
        )]));
        assert_eq!(pod_publish_rollout_token(&pod).as_deref(), Some("tok-1"));
    }

    #[test]
    fn cutover_ordering_requires_blue_down_before_publish_and_service() {
        // Start
        let (s1, c1) = plan_cutover_advance(CutoverStep::ScaleBlueDown, false, false);
        assert_eq!(c1, CutoverCommand::ScaleBlueToZero);
        assert_eq!(s1, CutoverStep::WaitBlueDown);

        // Still waiting for blue down - must not publish or switch
        let (s2, c2) = plan_cutover_advance(CutoverStep::WaitBlueDown, false, true);
        assert_eq!(c2, CutoverCommand::Wait);
        assert_eq!(s2, CutoverStep::WaitBlueDown);

        // Blue down -> enable publish+restart
        let (s3, c3) = plan_cutover_advance(CutoverStep::WaitBlueDown, true, false);
        assert_eq!(c3, CutoverCommand::EnableGreenPublishingAndRestart);
        assert_eq!(s3, CutoverStep::EnableGreenPublish);

        // After publish command applied, wait for healthy
        let (s4, c4) = plan_cutover_advance(CutoverStep::EnableGreenPublish, true, false);
        assert_eq!(c4, CutoverCommand::Wait);
        assert_eq!(s4, CutoverStep::WaitGreenHealthy);

        // Healthy but - Service switch only when eligible
        let (s5, c5) = plan_cutover_advance(CutoverStep::WaitGreenHealthy, true, false);
        assert_eq!(c5, CutoverCommand::Wait);
        let (s6, c6) = plan_cutover_advance(CutoverStep::WaitGreenHealthy, true, true);
        assert_eq!(c6, CutoverCommand::SwitchServiceToGreenAndFinish);
        assert_eq!(s6, CutoverStep::SwitchService);
    }

    #[test]
    fn service_does_not_switch_before_green_gate() {
        let (_, cmd) = plan_cutover_advance(CutoverStep::WaitGreenHealthy, true, false);
        assert_ne!(cmd, CutoverCommand::SwitchServiceToGreenAndFinish);
        assert!(!may_switch_service_to_green(true, false));
    }

    #[test]
    fn rollback_requires_blue_healthy_before_service_switch() {
        let (s1, c1) = plan_rollback_advance(RollbackStep::StopGreen, false);
        assert_eq!(c1, RollbackCommand::ScaleGreenToZero);
        assert_eq!(s1, RollbackStep::ScaleBlueUp);

        let (s2, c2) = plan_rollback_advance(RollbackStep::ScaleBlueUp, false);
        assert_eq!(c2, RollbackCommand::ScaleBlueToOne);
        assert_eq!(s2, RollbackStep::WaitBlueHealthy);

        let (s3, c3) = plan_rollback_advance(RollbackStep::WaitBlueHealthy, false);
        assert_eq!(c3, RollbackCommand::Wait);

        let (s4, c4) = plan_rollback_advance(RollbackStep::WaitBlueHealthy, true);
        assert_eq!(c4, RollbackCommand::SwitchServiceToBlueAndFinish);
        assert_eq!(s4, RollbackStep::SwitchService);
    }

    #[test]
    fn protected_pvcs_never_listed_for_deletion_helper() {
        // delete_optional explicitly no-ops "pvc" - covered by code review + this naming guard.
        let node = sample_node();
        assert_eq!(blue_pvc_name(&node), "validator-1-data");
        assert_eq!(green_pvc_name(&node), "validator-1-green-data");
    }

    #[test]
    fn failed_phase_is_stable_without_retry_annotation() {
        let mut node = sample_node();
        node.metadata.annotations = Some(BTreeMap::from([(
            ANN_PHASE.to_string(),
            CoreBlueGreenPhase::Failed.as_str().to_string(),
        )]));
        assert_eq!(read_phase(&node), CoreBlueGreenPhase::Failed);
        assert!(!retry_requested(&node));
        node.metadata
            .annotations
            .as_mut()
            .unwrap()
            .insert(ANN_RETRY.to_string(), "true".to_string());
        assert!(retry_requested(&node));
    }

    #[test]
    fn enabling_publishing_adds_rollout_annotation_to_sts() {
        let node = sample_node();
        let sts = build_colored_statefulset_for_test(
            &node,
            COLOR_GREEN,
            ROLE_ACTIVE,
            &green_sts_name(&node),
            &green_pvc_name(&node),
            &green_config_name(&node),
            &green_headless_name(&node),
            1,
            Some("2026-01-01T00:00:00Z"),
        );
        assert!(sts_has_publish_rollout_annotation(&sts));
        let standby = build_colored_statefulset_for_test(
            &node,
            COLOR_GREEN,
            ROLE_STANDBY,
            &green_sts_name(&node),
            &green_pvc_name(&node),
            &green_config_name(&node),
            &green_headless_name(&node),
            1,
            None,
        );
        assert!(!sts_has_publish_rollout_annotation(&standby));
    }

    #[test]
    fn gate_rejects_catching_up_and_readiness_alone() {
        let blue = snap(CoreSyncState::Synced, Some(110), true, true);
        assert!(!evaluate_cutover_gate(
            &snap(CoreSyncState::CatchingUp, Some(100), true, true),
            &blue,
            5
        )
        .is_eligible());
        assert!(!evaluate_cutover_gate(
            &snap(CoreSyncState::Unknown, Some(110), true, true),
            &blue,
            5
        )
        .is_eligible());
        assert!(evaluate_cutover_gate(
            &snap(CoreSyncState::Synced, Some(108), true, true),
            &blue,
            5
        )
        .is_eligible());
    }

    #[test]
    fn rollback_gate_requires_synced_ready() {
        assert!(
            !evaluate_rollback_gate(&snap(CoreSyncState::CatchingUp, Some(1), true, true))
                .is_eligible()
        );
        assert!(
            evaluate_rollback_gate(&snap(CoreSyncState::Synced, Some(1), true, true)).is_eligible()
        );
    }

    #[test]
    fn sts_fully_down_helper() {
        assert!(sts_status_fully_down(Some(0), Some(0), Some(0)));
        assert!(!sts_status_fully_down(Some(0), Some(1), Some(1)));
        assert!(!sts_status_fully_down(Some(1), Some(0), Some(0)));
    }

    #[test]
    fn default_max_ledger_lag_is_five() {
        assert_eq!(BlueGreenStrategyConfig::default().max_ledger_lag, 5);
    }
}
