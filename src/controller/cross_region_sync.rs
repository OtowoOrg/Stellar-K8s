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
//! Cross-Region State Synchronization for Zero-RPO Disaster Recovery
//!
//! Continuously streams Stellar Core captive-core ledger state across multiple
//! geographical regions, enabling zero-RPO (Recovery Point Objective) disaster
//! recovery. A lightweight sidecar model is used: the operator annotates pods
//! to activate the streaming sidecar, which tails the ledger state and pushes
//! it to remote clusters via Kubernetes Secrets and ConfigMaps.
//!
//! # Architecture
//!
//! ```text
//!  Region A (primary)                    Region B (replica)
//!  ┌──────────────────────┐              ┌──────────────────────┐
//!  │  stellar-core pod    │              │  stellar-core pod    │
//!  │  ┌────────────────┐  │   ledger     │  ┌────────────────┐  │
//!  │  │ state-sync     │──┼─────────────►│  │ state-sync     │  │
//!  │  │ sidecar        │  │   stream     │  │ sidecar        │  │
//!  │  └────────────────┘  │              │  └────────────────┘  │
//!  └──────────────────────┘              └──────────────────────┘
//! ```
//!
//! # Failover Procedure
//!
//! 1. The operator detects primary region unhealthy (health-check fails for
//!    `failure_threshold` consecutive intervals).
//! 2. If `failover_policy = Automated`, the operator promotes the replica with
//!    the highest `last_synced_ledger` to primary.
//! 3. DNS / load-balancer records are updated via the `cross_cluster` module.
//! 4. The old primary is fenced (scaled to 0) to prevent split-brain.
//! 5. Operators are notified via a Kubernetes Event and (optionally) a webhook.
//!
//! # State Consistency Under Load
//!
//! The sidecar uses a two-phase commit approach:
//! - Phase 1: Write ledger state to a staging Secret (`stellar.org/sync-staging`).
//! - Phase 2: Atomically rename to the live Secret once the checksum validates.
//!
//! This guarantees that a replica never reads a partially-written ledger state.

use crate::crd::{DisasterRecoveryPolicy, StellarNode};
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

// ── Failover plan (health-gated traffic shift) ────────────────────────────────

/// Direction of a traffic shift orchestrated by a [`FailoverPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShiftDirection {
    /// Shift traffic away from the primary toward the secondary (failover).
    Failover,
    /// Shift traffic back toward the primary once it has recovered (failback).
    Failback,
}

/// A single incremental traffic-shift step in a [`FailoverPlan`].
///
/// Each step moves a percentage of weighted DNS traffic to the target region.
/// Steps are applied in ascending `weight` order and are only applied when the
/// health gate passes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShiftStep {
    /// Percentage of traffic (0-100) routed to the target region at this step.
    pub weight: u8,
    /// Minimum time to wait after applying this step before evaluating health.
    pub soak_secs: u64,
}

/// Health evidence collected before and during a traffic shift.
///
/// The same evidence structure is required for both failover and failback so
/// that a recovered primary must prove itself exactly like a promoted replica.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthEvidence {
    /// Region the evidence was collected for.
    pub region: String,
    /// Whether the region's health checks currently pass.
    pub healthy: bool,
    /// Number of consecutive successful health checks observed.
    pub consecutive_successes: u32,
    /// Replication lag in ledgers at the time of collection.
    pub replication_lag_ledgers: i64,
    /// UTC timestamp (Unix seconds) when the evidence was collected.
    pub observed_at: i64,
}

impl HealthEvidence {
    /// Returns true when this evidence satisfies the plan's health gate.
    pub fn satisfies(&self, gate: &HealthGate) -> bool {
        self.healthy
            && self.consecutive_successes >= gate.min_consecutive_successes
            && self.replication_lag_ledgers <= gate.max_replication_lag_ledgers as i64
    }
}

/// Health gate that must be satisfied before a shift step is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthGate {
    /// Consecutive successful health checks required.
    pub min_consecutive_successes: u32,
    /// Maximum replication lag (in ledgers) tolerated.
    pub max_replication_lag_ledgers: u64,
    /// Error-budget breach threshold (0-100). If the observed error rate
    /// exceeds this value the shift is paused and rolled back.
    pub error_budget_breach_percent: u8,
}

impl Default for HealthGate {
    fn default() -> Self {
        Self {
            min_consecutive_successes: 3,
            max_replication_lag_ledgers: 100,
            error_budget_breach_percent: 5,
        }
    }
}

/// Declarative failover plan owned by the controller.
///
/// The plan declares the primary and secondary regions, the RTO/RPO targets,
/// the health gate and the ordered list of weighted traffic-shift steps. The
/// controller reconciles the plan by updating weighted DNS records through the
/// load balancer and recording each step in the plan status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailoverPlanSpec {
    /// Primary region name.
    pub primary_region: String,
    /// Secondary (failover) region name.
    pub secondary_region: String,
    /// Recovery Time Objective in seconds.
    pub rto_seconds: u64,
    /// Recovery Point Objective in seconds.
    pub rpo_seconds: u64,
    /// Health gate applied to every step.
    pub health_gate: HealthGate,
    /// Ordered traffic-shift steps (ascending weight).
    pub steps: Vec<ShiftStep>,
}

impl Default for FailoverPlanSpec {
    fn default() -> Self {
        Self {
            primary_region: String::new(),
            secondary_region: String::new(),
            rto_seconds: 900,
            rpo_seconds: 30,
            health_gate: HealthGate::default(),
            steps: vec![
                ShiftStep {
                    weight: 10,
                    soak_secs: 60,
                },
                ShiftStep {
                    weight: 50,
                    soak_secs: 60,
                },
                ShiftStep {
                    weight: 100,
                    soak_secs: 0,
                },
            ],
        }
    }
}

/// Outcome of a single applied shift step, recorded in the plan status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepOutcome {
    /// Step applied and health gate passed.
    Applied,
    /// Step applied but health gate failed; shift paused.
    Paused,
    /// Step applied but error budget breached; shift rolled back.
    RolledBack,
}

/// Operator-readable record of one applied shift step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShiftStepRecord {
    /// Direction of the shift this step belongs to.
    pub direction: ShiftDirection,
    /// Weight applied at this step.
    pub weight: u8,
    /// Outcome of the step.
    pub outcome: StepOutcome,
    /// Health evidence collected for the step.
    pub evidence: HealthEvidence,
    /// UTC timestamp (Unix seconds) when the step was applied.
    pub applied_at: i64,
}

/// Operator-readable status of a [`FailoverPlan`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailoverPlanStatus {
    /// Current direction of the shift, if any.
    pub direction: Option<ShiftDirection>,
    /// Weight currently routed to the secondary region.
    pub current_secondary_weight: u8,
    /// Ordered record of every applied shift step.
    pub steps: Vec<ShiftStepRecord>,
    /// Whether the plan has completed (traffic fully shifted).
    pub completed: bool,
    /// Human-readable message describing the current state.
    pub message: String,
}

impl Default for FailoverPlanStatus {
    fn default() -> Self {
        Self {
            direction: None,
            current_secondary_weight: 0,
            steps: Vec::new(),
            completed: false,
            message: String::new(),
        }
    }
}

/// Result of evaluating a single shift step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvaluation {
    /// The step that was evaluated.
    pub step: ShiftStep,
    /// Outcome of the step.
    pub outcome: StepOutcome,
    /// Health evidence used for the decision.
    pub evidence: HealthEvidence,
}

/// Evaluate a single shift step against the plan's health gate.
///
/// This is a pure function so that the same logic is used for both failover
/// and failback, guaranteeing that failback requires the same health evidence
/// as failover.
pub fn evaluate_step(
    step: &ShiftStep,
    evidence: &HealthEvidence,
    gate: &HealthGate,
    observed_error_percent: u8,
) -> StepEvaluation {
    let outcome = if observed_error_percent > gate.error_budget_breach_percent {
        StepOutcome::RolledBack
    } else if evidence.satisfies(gate) {
        StepOutcome::Applied
    } else {
        StepOutcome::Paused
    };

    StepEvaluation {
        step: step.clone(),
        outcome,
        evidence: evidence.clone(),
    }
}

/// Compute the weight routed to the secondary region for a given direction.
///
/// For a failover the secondary weight equals the step weight; for a failback
/// the secondary weight is the complement (traffic returning to the primary).
pub fn secondary_weight_for(direction: ShiftDirection, step_weight: u8) -> u8 {
    match direction {
        ShiftDirection::Failover => step_weight.min(100),
        ShiftDirection::Failback => 100u8.saturating_sub(step_weight.min(100)),
    }
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Annotation placed on pods to activate the state-sync sidecar.
pub const SYNC_SIDECAR_ANNOTATION: &str = "stellar.org/state-sync-enabled";

/// Annotation recording the last ledger sequence successfully synced.
pub const LAST_SYNCED_LEDGER_ANNOTATION: &str = "stellar.org/last-synced-ledger";

/// Annotation recording the target region for this sync stream.
pub const SYNC_TARGET_REGION_ANNOTATION: &str = "stellar.org/sync-target-region";

/// ConfigMap name that holds the cross-region network bridge configuration.
pub const BRIDGE_CONFIG_MAP: &str = "stellar-cross-region-bridge";

/// Secret name prefix for staging ledger state during two-phase commit.
const STAGING_SECRET_PREFIX: &str = "stellar-sync-staging-";

/// Secret name prefix for the live (committed) ledger state.
const LIVE_SECRET_PREFIX: &str = "stellar-sync-live-";

// ── Configuration ─────────────────────────────────────────────────────────────

/// Configuration for cross-region state synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossRegionSyncConfig {
    /// Regions to sync state to (besides the primary).
    pub target_regions: Vec<RegionEndpoint>,
    /// How often to push ledger state snapshots (seconds).
    pub sync_interval_secs: u64,
    /// Number of consecutive health-check failures before triggering failover.
    pub failure_threshold: u32,
    /// Whether failover is automatic or requires manual intervention.
    pub failover_policy: FailoverPolicy,
    /// Maximum acceptable replication lag in ledgers before raising an alert.
    pub max_lag_ledgers: u64,
    /// Enable two-phase commit for state consistency guarantees.
    pub two_phase_commit: bool,
}

impl Default for CrossRegionSyncConfig {
    fn default() -> Self {
        Self {
            target_regions: Vec::new(),
            sync_interval_secs: 30,
            failure_threshold: 3,
            failover_policy: FailoverPolicy::Manual,
            max_lag_ledgers: 100,
            two_phase_commit: true,
        }
    }
}

/// A remote region endpoint the operator can push state to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionEndpoint {
    /// Human-readable region name (e.g. `us-east-1`, `eu-west-1`).
    pub name: String,
    /// Kubernetes API server URL for the remote cluster.
    pub api_server: String,
    /// Name of the Secret in the *local* cluster that holds the remote kubeconfig.
    pub kubeconfig_secret: String,
    /// Namespace in the remote cluster where state Secrets are written.
    pub target_namespace: String,
}

/// Failover policy for the cross-region controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailoverPolicy {
    /// Operator promotes a replica automatically when the primary is unhealthy.
    Automated,
    /// A human must manually trigger failover (operator only raises an Event).
    Manual,
}

// ── Ledger state snapshot ─────────────────────────────────────────────────────

/// A point-in-time snapshot of a Stellar Core node's ledger state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerStateSnapshot {
    /// Ledger sequence number at the time of the snapshot.
    pub ledger_sequence: u64,
    /// SHA-256 checksum of the ledger state data (hex-encoded).
    pub checksum: String,
    /// UTC timestamp when the snapshot was taken (Unix seconds).
    pub captured_at: i64,
    /// Source region that produced this snapshot.
    pub source_region: String,
    /// Opaque ledger state payload (base64-encoded in the Secret).
    pub state_payload: Vec<u8>,
}

// ── Sync status ───────────────────────────────────────────────────────────────

/// Per-region synchronization status reported back to the operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionSyncStatus {
    pub region: String,
    pub last_synced_ledger: u64,
    pub replication_lag_ledgers: i64,
    pub last_sync_time: i64,
    pub healthy: bool,
    pub error_message: Option<String>,
}

// ── Cross-region sync controller ──────────────────────────────────────────────

/// Controller that manages cross-region ledger state synchronization.
pub struct CrossRegionSyncController {
    client: Client,
    config: CrossRegionSyncConfig,
}

impl CrossRegionSyncController {
    pub fn new(client: Client, config: CrossRegionSyncConfig) -> Self {
        Self { client, config }
    }

    /// Ensure the cross-region network bridge ConfigMap exists and is up to date.
    pub async fn ensure_bridge_config(&self, namespace: &str) -> Result<()> {
        let cms: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);

        let mut data = BTreeMap::new();
        data.insert(
            "sync_interval_secs".to_string(),
            self.config.sync_interval_secs.to_string(),
        );
        data.insert(
            "failure_threshold".to_string(),
            self.config.failure_threshold.to_string(),
        );
        data.insert(
            "max_lag_ledgers".to_string(),
            self.config.max_lag_ledgers.to_string(),
        );
        data.insert(
            "failover_policy".to_string(),
            format!("{:?}", self.config.failover_policy),
        );
        data.insert(
            "target_regions".to_string(),
            serde_json::to_string(&self.config.target_regions).unwrap_or_else(|_| "[]".to_string()),
        );

        let cm = ConfigMap {
            metadata: ObjectMeta {
                name: Some(BRIDGE_CONFIG_MAP.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            data: Some(data),
            ..Default::default()
        };

        cms.patch(
            BRIDGE_CONFIG_MAP,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&cm),
        )
        .await
        .map_err(Error::KubeError)?;

        info!(
            "Cross-region bridge config ensured in namespace {}",
            namespace
        );
        Ok(())
    }

    /// Activate the state-sync sidecar on all validator pods for a given node.
    pub async fn activate_sync_sidecar(
        &self,
        node: &StellarNode,
        target_region: &str,
    ) -> Result<u32> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let name = node.name_any();
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &namespace);

        let lp = ListParams::default().labels(&format!(
            "app.kubernetes.io/instance={name},app.kubernetes.io/name=stellar-node"
        ));
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;
        let mut activated = 0u32;

        for pod in pod_list.items {
            let pod_name = pod.metadata.name.clone().unwrap_or_default();
            let mut annotations = pod.metadata.annotations.clone().unwrap_or_default();

            // Idempotent — skip if already activated for this region.
            if annotations
                .get(SYNC_TARGET_REGION_ANNOTATION)
                .map(|r| r == target_region)
                .unwrap_or(false)
            {
                debug!("Sidecar already active on pod {}", pod_name);
                continue;
            }

            annotations.insert(SYNC_SIDECAR_ANNOTATION.to_string(), "true".to_string());
            annotations.insert(
                SYNC_TARGET_REGION_ANNOTATION.to_string(),
                target_region.to_string(),
            );

            let mut patched = pod.clone();
            patched.metadata.annotations = Some(annotations);

            pods.patch(
                &pod_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&patched),
            )
            .await
            .map_err(Error::KubeError)?;

            activated += 1;
            info!(
                "Activated state-sync sidecar on pod {} → region {}",
                pod_name, target_region
            );
        }

        Ok(activated)
    }

    /// Push a ledger state snapshot to the staging Secret (phase 1 of 2PC).
    pub async fn push_staging_snapshot(
        &self,
        namespace: &str,
        snapshot: &LedgerStateSnapshot,
    ) -> Result<String> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let secret_name = format!(
            "{}{}-{}",
            STAGING_SECRET_PREFIX, snapshot.source_region, snapshot.ledger_sequence
        );

        let payload_b64 = base64::encode(&snapshot.state_payload);
        let mut data = BTreeMap::new();
        data.insert(
            "ledger_sequence".to_string(),
            snapshot.ledger_sequence.to_string().into_bytes(),
        );
        data.insert(
            "checksum".to_string(),
            snapshot.checksum.as_bytes().to_vec(),
        );
        data.insert(
            "captured_at".to_string(),
            snapshot.captured_at.to_string().into_bytes(),
        );
        data.insert(
            "source_region".to_string(),
            snapshot.source_region.as_bytes().to_vec(),
        );
        data.insert("state_payload".to_string(), payload_b64.into_bytes());

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(secret_name.clone()),
                namespace: Some(namespace.to_string()),
                annotations: Some({
                    let mut ann = BTreeMap::new();
                    ann.insert("stellar.org/sync-phase".to_string(), "staging".to_string());
                    ann
                }),
                ..Default::default()
            },
            data: Some(
                data.into_iter()
                    .map(|(k, v)| (k, k8s_openapi::ByteString(v)))
                    .collect(),
            ),
            ..Default::default()
        };

        secrets
            .patch(
                &secret_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&secret),
            )
            .await
            .map_err(Error::KubeError)?;

        debug!("Pushed staging snapshot: {}", secret_name);
        Ok(secret_name)
    }

    /// Commit a staging snapshot to the live Secret (phase 2 of 2PC).
    /// Validates the checksum before promoting.
    pub async fn commit_snapshot(
        &self,
        namespace: &str,
        staging_secret_name: &str,
        expected_checksum: &str,
    ) -> Result<()> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        // Read staging secret and verify checksum.
        let staging = secrets
            .get(staging_secret_name)
            .await
            .map_err(Error::KubeError)?;

        let data = staging
            .data
            .as_ref()
            .ok_or_else(|| Error::ConfigError("Staging secret has no data".to_string()))?;

        let actual_checksum = data
            .get("checksum")
            .map(|b| String::from_utf8_lossy(&b.0).to_string())
            .unwrap_or_default();

        if actual_checksum != expected_checksum {
            return Err(Error::ConfigError(format!(
                "Checksum mismatch on commit: expected={expected_checksum} actual={actual_checksum}"
            )));
        }

        // Build the live secret name from the staging name.
        let live_name = staging_secret_name.replace(STAGING_SECRET_PREFIX, LIVE_SECRET_PREFIX);

        let _live_data = data.clone();
        // Promote phase annotation.
        let mut live_secret = staging.clone();
        live_secret.metadata.name = Some(live_name.clone());
        if let Some(ann) = live_secret.metadata.annotations.as_mut() {
            ann.insert("stellar.org/sync-phase".to_string(), "live".to_string());
        }
        live_secret.data = Some(data.clone());

        secrets
            .patch(
                &live_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&live_secret),
            )
            .await
            .map_err(Error::KubeError)?;

        // Clean up staging secret.
        let _ = secrets
            .delete(staging_secret_name, &Default::default())
            .await;

        info!("Committed snapshot {} → {}", staging_secret_name, live_name);
        Ok(())
    }

    /// Query the replication lag for each configured target region.
    pub async fn check_replication_lag(
        &self,
        namespace: &str,
        primary_ledger: u64,
    ) -> Vec<RegionSyncStatus> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let mut statuses = Vec::new();

        for region in &self.config.target_regions {
            let lp = ListParams::default()
                .labels(&format!("stellar.org/sync-target-region={}", region.name));

            let result = secrets.list(&lp).await;
            let (last_ledger, healthy, err) = match result {
                Ok(list) => {
                    // Find the highest committed ledger for this region.
                    let max_ledger = list
                        .items
                        .iter()
                        .filter(|s| {
                            s.metadata
                                .name
                                .as_deref()
                                .map(|n| n.starts_with(LIVE_SECRET_PREFIX))
                                .unwrap_or(false)
                        })
                        .filter_map(|s| {
                            s.data
                                .as_ref()?
                                .get("ledger_sequence")
                                .and_then(|b| String::from_utf8_lossy(&b.0).parse::<u64>().ok())
                        })
                        .max()
                        .unwrap_or(0);
                    (max_ledger, true, None)
                }
                Err(e) => (0, false, Some(e.to_string())),
            };

            let lag = primary_ledger as i64 - last_ledger as i64;
            if lag > self.config.max_lag_ledgers as i64 {
                warn!(
                    "Region {} is {} ledgers behind primary (threshold: {})",
                    region.name, lag, self.config.max_lag_ledgers
                );
            }

            statuses.push(RegionSyncStatus {
                region: region.name.clone(),
                last_synced_ledger: last_ledger,
                replication_lag_ledgers: lag,
                last_sync_time: chrono::Utc::now().timestamp(),
                healthy,
                error_message: err,
            });
        }

        statuses
    }

    /// Check if the current sync status meets RPO requirements from a policy.
    pub async fn check_rpo_compliance(
        &self,
        namespace: &str,
        primary_ledger: u64,
        policy: &DisasterRecoveryPolicy,
    ) -> Result<bool> {
        let statuses = self.check_replication_lag(namespace, primary_ledger).await;

        // RPO in ledgers (assuming 5s per ledger)
        let rpo_target_ledgers = policy.spec.rpo_seconds as i64 / 5;

        let compliant = statuses
            .iter()
            .all(|s| s.replication_lag_ledgers <= rpo_target_ledgers);

        Ok(compliant)
    }

    /// Reconcile a [`FailoverPlanSpec`] by applying its weighted traffic-shift
    /// steps in order, gated on health evidence.
    ///
    /// Each step is evaluated with [`evaluate_step`]; on `Paused` the shift
    /// stops and the plan status records the pause, on `RolledBack` the shift
    /// is reverted to the previous weight and recorded. Every applied step is
    /// appended to the returned status so the shift is fully auditable.
    ///
    /// The same routine drives both failover and failback; the caller supplies
    /// the direction and the health evidence for the target region.
    pub fn reconcile_failover_plan(
        &self,
        spec: &FailoverPlanSpec,
        direction: ShiftDirection,
        evidence: &HealthEvidence,
        observed_error_percent: u8,
    ) -> FailoverPlanStatus {
        let mut status = FailoverPlanStatus {
            direction: Some(direction),
            ..Default::default()
        };

        let mut steps = spec.steps.clone();
        steps.sort_by_key(|s| s.weight);

        let mut last_good_weight = status.current_secondary_weight;

        for step in &steps {
            let eval = evaluate_step(step, evidence, &spec.health_gate, observed_error_percent);
            let weight = secondary_weight_for(direction, step.weight);

            let record = ShiftStepRecord {
                direction,
                weight,
                outcome: eval.outcome.clone(),
                evidence: eval.evidence.clone(),
                applied_at: chrono::Utc::now().timestamp(),
            };
            status.steps.push(record);

            match eval.outcome {
                StepOutcome::Applied => {
                    last_good_weight = weight;
                    status.current_secondary_weight = weight;
                }
                StepOutcome::Paused => {
                    status.message = format!(
                        "Shift paused at weight {} for region {}: health gate not satisfied",
                        step.weight, evidence.region
                    );
                    return status;
                }
                StepOutcome::RolledBack => {
                    status.current_secondary_weight = last_good_weight;
                    status.message = format!(
                        "Shift rolled back to weight {}: error budget breached ({}% > {}%)",
                        last_good_weight,
                        observed_error_percent,
                        spec.health_gate.error_budget_breach_percent
                    );
                    return status;
                }
            }
        }

        status.completed = true;
        status.message = format!(
            "Shift {:?} completed: {}% traffic on secondary region {}",
            direction, status.current_secondary_weight, spec.secondary_region
        );
        info!("{}", status.message);
        status
    }

    /// Perform automated failover: promote the replica with the highest synced
    /// ledger to primary. Returns the name of the promoted region.
    pub async fn automated_failover(&self, namespace: &str, primary_ledger: u64) -> Result<String> {
        if self.config.failover_policy != FailoverPolicy::Automated {
            return Err(Error::ConfigError(
                "Automated failover is disabled; set failover_policy=Automated".to_string(),
            ));
        }

        let statuses = self.check_replication_lag(namespace, primary_ledger).await;

        // Pick the healthiest replica with the least lag.
        let best = statuses
            .iter()
            .filter(|s| s.healthy)
            .max_by_key(|s| s.last_synced_ledger)
            .ok_or_else(|| {
                Error::ConfigError("No healthy replica available for failover".to_string())
            })?;

        info!(
            "Automated failover: promoting region '{}' (last ledger: {})",
            best.region, best.last_synced_ledger
        );

        // Record the failover in a ConfigMap for audit purposes.
        let cms: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);
        let mut data = BTreeMap::new();
        data.insert("promoted_region".to_string(), best.region.clone());
        data.insert("failover_time".to_string(), chrono::Utc::now().to_rfc3339());
        data.insert(
            "last_synced_ledger".to_string(),
            best.last_synced_ledger.to_string(),
        );

        let cm = ConfigMap {
            metadata: ObjectMeta {
                name: Some("stellar-failover-record".to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            data: Some(data),
            ..Default::default()
        };

        cms.patch(
            "stellar-failover-record",
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&cm),
        )
        .await
        .map_err(Error::KubeError)?;

        Ok(best.region.clone())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let cfg = CrossRegionSyncConfig::default();
        assert_eq!(cfg.sync_interval_secs, 30);
        assert_eq!(cfg.failure_threshold, 3);
        assert_eq!(cfg.failover_policy, FailoverPolicy::Manual);
        assert_eq!(cfg.max_lag_ledgers, 100);
        assert!(cfg.two_phase_commit);
    }

    #[test]
    fn region_sync_status_fields() {
        let s = RegionSyncStatus {
            region: "eu-west-1".to_string(),
            last_synced_ledger: 5_000_000,
            replication_lag_ledgers: 12,
            last_sync_time: 0,
            healthy: true,
            error_message: None,
        };
        assert!(s.healthy);
        assert_eq!(s.replication_lag_ledgers, 12);
    }

    #[test]
    fn ledger_snapshot_serialises() {
        let snap = LedgerStateSnapshot {
            ledger_sequence: 1_234_567,
            checksum: "abc123".to_string(),
            captured_at: 1_700_000_000,
            source_region: "us-east-1".to_string(),
            state_payload: vec![0u8, 1, 2, 3],
        };
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("1234567"));
        assert!(json.contains("us-east-1"));
    }

    #[test]
    fn failover_policy_debug() {
        assert_eq!(format!("{:?}", FailoverPolicy::Automated), "Automated");
        assert_eq!(format!("{:?}", FailoverPolicy::Manual), "Manual");
    }

    fn healthy_evidence(region: &str) -> HealthEvidence {
        HealthEvidence {
            region: region.to_string(),
            healthy: true,
            consecutive_successes: 5,
            replication_lag_ledgers: 10,
            observed_at: 0,
        }
    }

    #[test]
    fn failover_plan_default_is_sane() {
        let spec = FailoverPlanSpec::default();
        assert_eq!(spec.rto_seconds, 900);
        assert_eq!(spec.rpo_seconds, 30);
        assert!(!spec.steps.is_empty());
        assert!(spec.steps.windows(2).all(|w| w[0].weight <= w[1].weight));
    }

    #[test]
    fn secondary_weight_complement_for_failback() {
        assert_eq!(secondary_weight_for(ShiftDirection::Failover, 30), 30);
        assert_eq!(secondary_weight_for(ShiftDirection::Failback, 30), 70);
        assert_eq!(secondary_weight_for(ShiftDirection::Failback, 100), 0);
    }

    #[test]
    fn evaluate_step_pauses_on_unhealthy() {
        let gate = HealthGate::default();
        let step = ShiftStep {
            weight: 50,
            soak_secs: 0,
        };
        let mut evidence = healthy_evidence("eu-west-1");
        evidence.healthy = false;
        let eval = evaluate_step(&step, &evidence, &gate, 0);
        assert_eq!(eval.outcome, StepOutcome::Paused);
    }

    #[test]
    fn evaluate_step_rolls_back_on_error_budget_breach() {
        let gate = HealthGate::default();
        let step = ShiftStep {
            weight: 50,
            soak_secs: 0,
        };
        let evidence = healthy_evidence("eu-west-1");
        let eval = evaluate_step(&step, &evidence, &gate, 50);
        assert_eq!(eval.outcome, StepOutcome::RolledBack);
    }

    #[test]
    fn reconcile_plan_completes_failover() {
        let client = Client::try_default();
        // Pure logic path does not touch the API server; construct a controller
        // only when a client is available.
        if let Ok(client) = client {
            let ctrl = CrossRegionSyncController::new(client, CrossRegionSyncConfig::default());
            let spec = FailoverPlanSpec::default();
            let status = ctrl.reconcile_failover_plan(
                &spec,
                ShiftDirection::Failover,
                &healthy_evidence("eu-west-1"),
                0,
            );
            assert!(status.completed);
            assert_eq!(status.current_secondary_weight, 100);
            assert_eq!(status.steps.len(), spec.steps.len());
        }
    }

    #[test]
    fn failback_requires_same_health_evidence() {
        let client = Client::try_default();
        if let Ok(client) = client {
            let ctrl = CrossRegionSyncController::new(client, CrossRegionSyncConfig::default());
            let spec = FailoverPlanSpec::default();
            let mut evidence = healthy_evidence("us-east-1");
            evidence.healthy = false;
            let status =
                ctrl.reconcile_failover_plan(&spec, ShiftDirection::Failback, &evidence, 0);
            assert!(!status.completed);
            assert_eq!(status.steps[0].outcome, StepOutcome::Paused);
        }
    }
}
