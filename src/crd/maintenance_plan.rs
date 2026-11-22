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
//! MaintenancePlan CRD — declarative planned-maintenance orchestration.
//!
//! Operators declare target nodes, drain/PDB/prewarm/SLO policy, and abort
//! behaviour. The controller coordinates PDB-aware draining, replacement
//! prewarming, service verification, and automatic recovery.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::crd::types::Condition;

/// Declarative control surface for planned node maintenance.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "MaintenancePlan",
    namespaced,
    status = "MaintenancePlanStatus",
    shortname = "mplan",
    shortname = "mtplan",
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Intent","type":"string","jsonPath":".spec.intent"}"#,
    printcolumn = r#"{"name":"Progress","type":"string","jsonPath":".status.progress"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePlanSpec {
    /// Nodes to maintain (names and/or label selector).
    pub target: MaintenanceTarget,
    /// Why the plan exists (patch, reboot, kernel, evacuate).
    #[serde(default)]
    pub intent: MaintenanceIntent,
    /// Eviction / drain knobs. Parallelism is PDB-capped, never serial-only.
    #[serde(default)]
    pub drain: DrainConfig,
    /// PodDisruptionBudget respect and stall recovery.
    #[serde(default)]
    pub pdb: PdbConfig,
    /// Prewarm replacements before evicting originals.
    #[serde(default)]
    pub prewarm: PrewarmConfig,
    /// Service SLO gates before success / on abort.
    #[serde(default)]
    pub slo: SloVerification,
    /// Abort + automatic recovery policy.
    #[serde(default)]
    pub abort: AbortPolicy,
}

/// Selects Kubernetes nodes for the plan.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTarget {
    /// Explicit node names.
    #[serde(default)]
    pub node_names: Vec<String>,
    /// Label selector (all listed labels must match).
    #[serde(default)]
    pub match_labels: std::collections::BTreeMap<String, String>,
}

/// Operator-declared maintenance intent.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum MaintenanceIntent {
    #[default]
    Patch,
    Reboot,
    KernelUpdate,
    Evacuate,
}

/// Drain / eviction configuration.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DrainConfig {
    /// Upper bound on concurrent evictions (still capped by each PDB).
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,
    /// Pod termination grace period.
    #[serde(default = "default_grace")]
    pub grace_period_seconds: u32,
    #[serde(default = "default_true")]
    pub ignore_daemon_sets: bool,
    #[serde(default = "default_true")]
    pub delete_empty_dir_data: bool,
}

fn default_max_parallel() -> u32 {
    4
}
fn default_grace() -> u32 {
    30
}
fn default_true() -> bool {
    true
}

impl Default for DrainConfig {
    fn default() -> Self {
        Self {
            max_parallel: default_max_parallel(),
            grace_period_seconds: default_grace(),
            ignore_daemon_sets: true,
            delete_empty_dir_data: true,
        }
    }
}

/// How the orchestrator treats PodDisruptionBudgets.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PdbConfig {
    /// Never evict when it would violate a PDB.
    #[serde(default = "default_true")]
    pub respect: bool,
    /// Time with zero eviction progress before stall recovery.
    #[serde(default = "default_stall_timeout")]
    pub stall_timeout_seconds: u32,
    /// Safe, explicit recovery after a stall timeout.
    #[serde(default)]
    pub stall_recovery: StallRecovery,
}

fn default_stall_timeout() -> u32 {
    120
}

impl Default for PdbConfig {
    fn default() -> Self {
        Self {
            respect: true,
            stall_timeout_seconds: default_stall_timeout(),
            stall_recovery: StallRecovery::default(),
        }
    }
}

/// Recovery path used after a PDB stall timeout.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum StallRecovery {
    /// Add a surge replica so the PDB allows a disruption, then evict.
    #[default]
    SurgeThenEvict,
    /// Skip the stuck node, uncordon it, continue the remaining set.
    SkipNode,
    /// Abort the plan and run recovery.
    Abort,
}

/// Replacement prewarming before destructive drain.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrewarmConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_prewarm_timeout")]
    pub readiness_timeout_seconds: u32,
    /// Extra ready replicas to create per workload before eviction.
    #[serde(default = "default_replica_buffer")]
    pub replica_buffer: u32,
}

fn default_prewarm_timeout() -> u32 {
    180
}
fn default_replica_buffer() -> u32 {
    1
}

impl Default for PrewarmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            readiness_timeout_seconds: default_prewarm_timeout(),
            replica_buffer: default_replica_buffer(),
        }
    }
}

/// SLO verification before marking a node / plan successful.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SloVerification {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum user-facing error rate (0.0–1.0). Breach aborts the plan.
    #[serde(default = "default_max_error_rate")]
    pub max_error_rate: f64,
    /// Minimum availability ratio (0.0–1.0).
    #[serde(default = "default_min_availability")]
    pub min_availability: f64,
    #[serde(default = "default_slo_window")]
    pub sample_window_seconds: u32,
}

fn default_max_error_rate() -> f64 {
    0.001
}
fn default_min_availability() -> f64 {
    0.999
}
fn default_slo_window() -> u32 {
    30
}

impl Default for SloVerification {
    fn default() -> Self {
        Self {
            enabled: true,
            max_error_rate: default_max_error_rate(),
            min_availability: default_min_availability(),
            sample_window_seconds: default_slo_window(),
        }
    }
}

/// Abort / automatic recovery policy.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AbortPolicy {
    #[serde(default = "default_true")]
    pub on_slo_breach: bool,
    #[serde(default = "default_true")]
    pub auto_recover: bool,
    #[serde(default = "default_true")]
    pub uncordon_on_abort: bool,
    /// Operator-requested abort (idempotent).
    #[serde(default)]
    pub requested: bool,
}

impl Default for AbortPolicy {
    fn default() -> Self {
        Self {
            on_slo_breach: true,
            auto_recover: true,
            uncordon_on_abort: true,
            requested: false,
        }
    }
}

/// Observed plan state. Persisted so reconcile is restart-safe.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePlanStatus {
    #[serde(default)]
    pub phase: MaintenancePhase,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Human-readable `completed/total`.
    #[serde(default)]
    pub progress: String,
    /// Resolved node list (stable order).
    #[serde(default)]
    pub resolved_nodes: Vec<String>,
    /// Index into `resolved_nodes` currently being processed.
    #[serde(default)]
    pub cursor: u32,
    /// Unix time post-drain verification began (0 = not started); bounds the verification wait.
    #[serde(default)]
    pub post_verify_started_unix: i64,
    /// Workloads that had pods on the target nodes when the plan started; readiness, SLO and
    /// post-drain checks are scoped to these so unrelated unhealthy workloads cannot block a plan.
    #[serde(default)]
    pub affected_workloads: Vec<String>,
    #[serde(default)]
    pub nodes_completed: Vec<String>,
    #[serde(default)]
    pub nodes_failed: Vec<String>,
    #[serde(default)]
    pub prewarmed_workloads: Vec<String>,
    /// Nodes already cordoned by this plan (never before verification).
    #[serde(default)]
    pub cordoned_nodes: Vec<String>,
    #[serde(default)]
    pub pdb_stalls: u32,
    #[serde(default)]
    pub last_progress_unix: i64,
    /// Unix time when the plan first resolved targets (cycle duration base).
    #[serde(default)]
    pub started_unix: i64,
    #[serde(default)]
    pub last_error_rate: f64,
    #[serde(default)]
    pub last_availability: f64,
    /// Measured milliseconds from eviction to replacement serving (this plan).
    #[serde(default)]
    pub cold_window_ms: u64,
    /// Wall time of the plan so far.
    #[serde(default)]
    pub patch_cycle_ms: u64,
    #[serde(default)]
    pub user_facing_errors: u64,
    #[serde(default)]
    pub observed_generation: i64,
    #[serde(default)]
    pub last_event: Option<String>,
    #[serde(default)]
    pub last_reconcile_time: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

/// Plan lifecycle phases (also emitted as Kubernetes events).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum MaintenancePhase {
    #[default]
    Planned,
    Prewarming,
    Draining,
    Stalled,
    Verifying,
    Aborted,
    Completed,
    Failed,
}

impl MaintenancePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planned => "Planned",
            Self::Prewarming => "Prewarming",
            Self::Draining => "Draining",
            Self::Stalled => "Stalled",
            Self::Verifying => "Verifying",
            Self::Aborted => "Aborted",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Aborted)
    }
}

impl MaintenancePlanSpec {
    /// Semantic validation used by the reconciler (schema is also on the CRD).
    pub fn validate(&self) -> Result<(), String> {
        if self.target.node_names.is_empty() && self.target.match_labels.is_empty() {
            return Err("target must set nodeNames and/or matchLabels".into());
        }
        if self.drain.max_parallel == 0 {
            return Err("drain.maxParallel must be >= 1".into());
        }
        if self.pdb.stall_timeout_seconds == 0 {
            return Err("pdb.stallTimeoutSeconds must be >= 1".into());
        }
        if !(0.0..=1.0).contains(&self.slo.max_error_rate) {
            return Err("slo.maxErrorRate must be in [0, 1]".into());
        }
        if !(0.0..=1.0).contains(&self.slo.min_availability) {
            return Err("slo.minAvailability must be in [0, 1]".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_is_safe() {
        let spec = MaintenancePlanSpec {
            target: MaintenanceTarget {
                node_names: vec!["n1".into()],
                match_labels: Default::default(),
            },
            ..Default::default()
        };
        assert!(spec.validate().is_ok());
        assert!(spec.pdb.respect);
        assert!(spec.prewarm.enabled);
        assert!(spec.abort.on_slo_breach);
        assert!(spec.drain.max_parallel > 1);
    }

    #[test]
    fn rejects_empty_target() {
        let spec = MaintenancePlanSpec {
            target: MaintenanceTarget::default(),
            ..Default::default()
        };
        assert!(spec.validate().is_err());
    }
}

impl Default for MaintenancePlanSpec {
    fn default() -> Self {
        Self {
            target: MaintenanceTarget::default(),
            intent: MaintenanceIntent::default(),
            drain: DrainConfig::default(),
            pdb: PdbConfig::default(),
            prewarm: PrewarmConfig::default(),
            slo: SloVerification::default(),
            abort: AbortPolicy::default(),
        }
    }
}
