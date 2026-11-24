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
//! Job/CronJob orphan detection with ownership reconciliation (epic #1503).
//!
//! Batch workloads leave a lot of debris behind: terminal Jobs whose CronJob was
//! deleted, Pods whose Job was removed out from under them, Jobs stuck in
//! `Failed`, and terminal Pods that never got garbage collected. This module
//! finds that debris and reclaims it, driven entirely by the namespace-scoped
//! [`JobRetentionPolicy`] CRD — no TTL is hardcoded anywhere.
//!
//! # Design
//!
//! The module is split in two halves:
//!
//! 1. **A pure core** — [`ClusterView`], [`classify_job`], [`classify_pod`] and
//!    [`plan_reclaim`] — which turns a snapshot of Jobs, Pods and CronJobs plus
//!    a policy into a deterministic [`ReclaimPlan`]. No clock and no cluster
//!    access: `now` is always a parameter, so every rule is unit-testable.
//! 2. **A thin Kubernetes path** — [`collect_view`] reads the live objects with
//!    `kube`, [`apply_plan`] issues the deletes and owner-reference patches, and
//!    [`reconcile_job_retention`] ties it together with status and metrics.
//!
//! # Orphan classes
//!
//! | Class | Meaning | Action |
//! |---|---|---|
//! | [`OrphanClass::DeletedCronJob`] | Job's owning CronJob is gone | delete |
//! | [`OrphanClass::BrokenOwnerReference`] | `ownerReference` UID no longer resolves | repair (or delete) |
//! | [`OrphanClass::StuckFailedJob`] | `Failed` Job past the retention window | delete |
//! | [`OrphanClass::CompletedPod`] | terminal Pod of a terminal Job, past the pod window | delete |
//! | [`OrphanClass::NamespaceMove`] | remnant of an artifact moved out of its namespace | delete |
//! | [`OrphanClass::DeletedOwnerRemnant`] | artifact with no controller owner at all | delete (in scope only) |
//!
//! # Safety properties
//!
//! - An `Active`/`Pending` Job or a `Pending`/`Running` Pod is **never**
//!   deleted, whatever the policy says.
//! - A Pod is only reclaimed once its owning Job is itself terminal, so a Job
//!   that is still running its other pods is left alone.
//! - Classification is a pure function of a single snapshot, so a CronJob spec
//!   change that lands mid-cycle cannot produce a half-applied decision: the
//!   change is picked up by the next sweep, which re-lists everything.
//! - Artifacts with no controller owner are only touched when they carry the
//!   configured workload label, so hand-written Jobs and Pods are never deleted.

use chrono::{DateTime, Utc};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams};
use kube::{Client, ResourceExt};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use crate::crd::job_retention::{
    JobRetentionPolicy, JobRetentionPolicySpec, JobRetentionPolicyStatus, ReclaimedByClass,
};
use crate::error::Result;

#[cfg(feature = "metrics")]
use super::metrics;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Kind of batch artifact under reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtifactKind {
    Job,
    Pod,
}

impl ArtifactKind {
    /// Metric/label friendly name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Job => "Job",
            Self::Pod => "Pod",
        }
    }

    /// Kubernetes kind string.
    pub fn kube_kind(self) -> &'static str {
        match self {
            Self::Job => "Job",
            Self::Pod => "Pod",
        }
    }
}

/// Why an artifact is considered an orphan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrphanClass {
    /// The owning CronJob no longer exists.
    DeletedCronJob,
    /// An `ownerReference` left dangling by a partial deletion.
    BrokenOwnerReference,
    /// A `Failed` Job that outlived the retention window.
    StuckFailedJob,
    /// A terminal Pod past the pod retention window.
    CompletedPod,
    /// A remnant of an artifact moved out of its original namespace.
    NamespaceMove,
    /// An artifact that carries no controller owner at all.
    DeletedOwnerRemnant,
}

impl OrphanClass {
    /// Stable label value used in metrics and status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeletedCronJob => "deleted_cron_job",
            Self::BrokenOwnerReference => "broken_owner_reference",
            Self::StuckFailedJob => "stuck_failed_job",
            Self::CompletedPod => "completed_pod",
            Self::NamespaceMove => "namespace_move",
            Self::DeletedOwnerRemnant => "deleted_owner_remnant",
        }
    }

    /// Every class, in declaration order.
    pub fn all() -> [Self; 6] {
        [
            Self::DeletedCronJob,
            Self::BrokenOwnerReference,
            Self::StuckFailedJob,
            Self::CompletedPod,
            Self::NamespaceMove,
            Self::DeletedOwnerRemnant,
        ]
    }

    fn tally(self, into: &mut ReclaimedByClass) {
        match self {
            Self::DeletedCronJob => into.deleted_cron_job += 1,
            Self::BrokenOwnerReference => into.broken_owner_reference += 1,
            Self::StuckFailedJob => into.stuck_failed_job += 1,
            Self::CompletedPod => into.completed_pod += 1,
            Self::NamespaceMove => into.namespace_move += 1,
            Self::DeletedOwnerRemnant => into.deleted_owner_remnant += 1,
        }
    }
}

/// What the reconciler should do with an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReclaimAction {
    /// Issue a `DELETE`.
    Delete,
    /// Re-point a broken `ownerReference` at the live owner UID.
    RepairOwnerRef,
    /// Do nothing this cycle.
    Retain,
}

/// Identity of an artifact: `Job` and `Pod` names are unique per namespace, and
/// the two kinds never share a key, so the pair is a stable sort key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ArtifactId {
    pub namespace: String,
    pub name: String,
}

impl ArtifactId {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

/// Snapshot of a single `ownerReference`, faithful enough to be re-emitted in a
/// patch when the UID is repaired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRefSnapshot {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub uid: String,
    pub controller: Option<bool>,
    pub block_owner_deletion: Option<bool>,
}

impl OwnerRefSnapshot {
    /// A CronJob owner reference.
    pub fn cron_job(name: impl Into<String>, uid: impl Into<String>) -> Self {
        Self {
            api_version: "batch/v1".to_string(),
            kind: "CronJob".to_string(),
            name: name.into(),
            uid: uid.into(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }
    }

    /// A Job owner reference.
    pub fn job(name: impl Into<String>, uid: impl Into<String>) -> Self {
        Self {
            api_version: "batch/v1".to_string(),
            kind: "Job".to_string(),
            name: name.into(),
            uid: uid.into(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }
    }

    /// Kubernetes JSON shape, used when patching repaired references.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "apiVersion": self.api_version,
            "kind": self.kind,
            "name": self.name,
            "uid": self.uid,
            "controller": self.controller,
            "blockOwnerDeletion": self.block_owner_deletion,
        })
    }
}

/// Derived phase of a Job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobPhase {
    /// Created but not yet picked up by a controller.
    Pending,
    /// Has at least one running pod.
    Active,
    /// At least one pod completed successfully and none are running.
    Succeeded,
    /// At least one pod failed and none are running.
    Failed,
    /// Explicitly suspended via `spec.suspend`.
    Suspended,
}

impl JobPhase {
    /// Has the Job stopped producing new pods?
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// Derived phase of a Pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

impl PodPhase {
    /// Has the Pod finished (successfully or not)?
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// A Job reduced to the fields the classifier needs.
#[derive(Debug, Clone)]
pub struct JobObservation {
    pub id: ArtifactId,
    pub uid: String,
    pub owner_refs: Vec<OwnerRefSnapshot>,
    pub phase: JobPhase,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub labels: BTreeMap<String, String>,
    /// Namespace the artifact was moved from, per the configured annotation.
    pub previous_namespace: Option<String>,
}

impl JobObservation {
    /// Timestamp the retention window is measured from: completion when known,
    /// creation otherwise.
    pub fn age_basis(&self) -> DateTime<Utc> {
        self.finished_at.unwrap_or(self.created_at)
    }
}

/// A Pod reduced to the fields the classifier needs.
#[derive(Debug, Clone)]
pub struct PodObservation {
    pub id: ArtifactId,
    pub uid: String,
    pub owner_refs: Vec<OwnerRefSnapshot>,
    pub phase: PodPhase,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub labels: BTreeMap<String, String>,
    /// Namespace the artifact was moved from, per the configured annotation.
    pub previous_namespace: Option<String>,
}

impl PodObservation {
    /// Timestamp the retention window is measured from.
    pub fn age_basis(&self) -> DateTime<Utc> {
        self.finished_at.unwrap_or(self.created_at)
    }
}

/// A point-in-time snapshot of one namespace's batch artifacts.
///
/// `plan_reclaim` is a pure function of this struct, so a sweep can never act
/// on a half-updated cluster view: the view is listed once, classified, and the
/// resulting plan is applied against whatever is live at that point. Deleting an
/// object that changed in the meantime fails with `NotFound`, which is ignored.
#[derive(Debug, Clone, Default)]
pub struct ClusterView {
    /// Live CronJobs keyed by `(namespace, name)` with their UID.
    pub cronjobs: BTreeMap<(String, String), String>,
    /// Jobs keyed by UID.
    pub jobs: BTreeMap<String, JobObservation>,
    /// Pods in the namespace.
    pub pods: Vec<PodObservation>,
}

impl ClusterView {
    /// New empty view.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a live CronJob.
    pub fn with_cronjob(mut self, namespace: &str, name: &str, uid: &str) -> Self {
        self.cronjobs
            .insert((namespace.to_string(), name.to_string()), uid.to_string());
        self
    }

    /// Register a Job.
    pub fn with_job(mut self, job: JobObservation) -> Self {
        self.jobs.insert(job.uid.clone(), job);
        self
    }

    /// Register a Pod.
    pub fn with_pod(mut self, pod: PodObservation) -> Self {
        self.pods.push(pod);
        self
    }

    /// Jobs in deterministic order.
    pub fn sorted_jobs(&self) -> Vec<&JobObservation> {
        let mut jobs: Vec<&JobObservation> = self.jobs.values().collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        jobs
    }

    /// Pods in deterministic order.
    pub fn sorted_pods(&self) -> Vec<&PodObservation> {
        let mut pods: Vec<&PodObservation> = self.pods.iter().collect();
        pods.sort_by(|a, b| a.id.cmp(&b.id));
        pods
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// The decision made for one artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub class: OrphanClass,
    pub action: ReclaimAction,
    /// Seconds since [`JobObservation::age_basis`] / [`PodObservation::age_basis`].
    pub age_seconds: i64,
    /// Human-readable justification, surfaced in status and logs.
    pub reason: String,
    /// Live UID to write back when `action` is [`ReclaimAction::RepairOwnerRef`].
    pub repair_uid: Option<String>,
}

impl Classification {
    fn retain(
        id: ArtifactId,
        kind: ArtifactKind,
        class: OrphanClass,
        age_seconds: i64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id,
            kind,
            class,
            action: ReclaimAction::Retain,
            age_seconds,
            reason: reason.into(),
            repair_uid: None,
        }
    }

    fn delete(
        id: ArtifactId,
        kind: ArtifactKind,
        class: OrphanClass,
        age_seconds: i64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id,
            kind,
            class,
            action: ReclaimAction::Delete,
            age_seconds,
            reason: reason.into(),
            repair_uid: None,
        }
    }

    fn repair(
        id: ArtifactId,
        kind: ArtifactKind,
        class: OrphanClass,
        age_seconds: i64,
        reason: impl Into<String>,
        repair_uid: String,
    ) -> Self {
        Self {
            id,
            kind,
            class,
            action: ReclaimAction::RepairOwnerRef,
            age_seconds,
            reason: reason.into(),
            repair_uid: Some(repair_uid),
        }
    }
}

/// Resolved state of a Job's controller owner.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OwnerState {
    /// The CronJob exists with exactly the referenced UID.
    Live,
    /// A CronJob of the same name exists under a different UID: the owner was
    /// deleted and recreated, leaving the Job's `ownerReference` broken.
    Recreated { live_uid: String },
    /// No CronJob of that name exists in the namespace.
    Missing,
    /// The Job carries no controller owner at all.
    Unowned,
}

fn resolve_job_owner(view: &ClusterView, job: &JobObservation) -> OwnerState {
    let Some(ref owner) = job.owner_refs.iter().find(|r| r.kind == "CronJob") else {
        return OwnerState::Unowned;
    };
    match view
        .cronjobs
        .get(&(job.id.namespace.clone(), owner.name.clone()))
    {
        None => OwnerState::Missing,
        Some(live_uid) if *live_uid == owner.uid => OwnerState::Live,
        Some(live_uid) => OwnerState::Recreated {
            live_uid: live_uid.clone(),
        },
    }
}

fn age_seconds(basis: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (now - basis).num_seconds().max(0)
}

fn was_moved(spec: &JobRetentionPolicySpec, previous: Option<&String>, current: &str) -> bool {
    spec.reclaim_after_namespace_move && previous.is_some_and(|p| p != current)
}

fn job_retention_for(spec: &JobRetentionPolicySpec, phase: JobPhase) -> Option<u32> {
    match phase {
        JobPhase::Succeeded => Some(spec.completed_job_retention_seconds),
        JobPhase::Failed => Some(spec.effective_failed_job_retention_seconds()),
        // Never reclaim a Job that may still make progress.
        JobPhase::Pending | JobPhase::Active | JobPhase::Suspended => None,
    }
}

/// Classify one Job against the cluster view and the retention policy.
///
/// Pure: the same `view`, `spec` and `now` always produce the same decision.
pub fn classify_job(
    view: &ClusterView,
    spec: &JobRetentionPolicySpec,
    job: &JobObservation,
    now: DateTime<Utc>,
) -> Classification {
    let age = age_seconds(job.age_basis(), now);
    let retention = job_retention_for(spec, job.phase);
    let past_retention = retention.is_some_and(|r| age >= i64::from(r));

    match resolve_job_owner(view, job) {
        OwnerState::Live => {
            let owner = job
                .owner_refs
                .iter()
                .find(|r| r.kind == "CronJob")
                .map(|r| r.name.clone())
                .unwrap_or_default();
            if job.phase == JobPhase::Failed && past_retention && spec.delete_orphaned_jobs {
                return Classification::delete(
                    job.id.clone(),
                    ArtifactKind::Job,
                    OrphanClass::StuckFailedJob,
                    age,
                    format!(
                        "Job has been Failed for {age}s, past the {retention:?}s stuck-job window"
                    ),
                );
            }
            Classification::retain(
                job.id.clone(),
                ArtifactKind::Job,
                OrphanClass::StuckFailedJob,
                age,
                format!("owned by live CronJob '{owner}'"),
            )
        }
        OwnerState::Recreated { live_uid } => {
            let owner = job
                .owner_refs
                .iter()
                .find(|r| r.kind == "CronJob")
                .map(|r| r.name.clone())
                .unwrap_or_default();
            if spec.reconcile_owner_references {
                return Classification::repair(
                    job.id.clone(),
                    ArtifactKind::Job,
                    OrphanClass::BrokenOwnerReference,
                    age,
                    format!(
                        "ownerReference to CronJob '{owner}' points at a stale UID; \
                         re-pointing at the live UID"
                    ),
                    live_uid,
                );
            }
            if job.phase == JobPhase::Failed && past_retention && spec.delete_orphaned_jobs {
                return Classification::delete(
                    job.id.clone(),
                    ArtifactKind::Job,
                    OrphanClass::StuckFailedJob,
                    age,
                    format!("Job has been Failed for {age}s and owner repair is disabled"),
                );
            }
            Classification::retain(
                job.id.clone(),
                ArtifactKind::Job,
                OrphanClass::BrokenOwnerReference,
                age,
                format!("ownerReference UID is stale but owner repair is disabled"),
            )
        }
        OwnerState::Missing => {
            let owner = job
                .owner_refs
                .iter()
                .find(|r| r.kind == "CronJob")
                .map(|r| r.name.clone())
                .unwrap_or_default();
            let moved = was_moved(spec, job.previous_namespace.as_ref(), &job.id.namespace);
            let class = if moved {
                OrphanClass::NamespaceMove
            } else {
                OrphanClass::DeletedCronJob
            };
            if !spec.delete_orphaned_jobs {
                return Classification::retain(
                    job.id.clone(),
                    ArtifactKind::Job,
                    class,
                    age,
                    format!("owning CronJob '{owner}' is gone but job deletion is disabled"),
                );
            }
            if past_retention {
                let why = if moved {
                    format!(
                        "moved out of namespace '{}'",
                        job.previous_namespace.clone().unwrap_or_default()
                    )
                } else {
                    format!("owning CronJob '{owner}' no longer exists")
                };
                return Classification::delete(
                    job.id.clone(),
                    ArtifactKind::Job,
                    class,
                    age,
                    format!("{why}; terminal for {age}s, past the {retention:?}s window"),
                );
            }
            Classification::retain(
                job.id.clone(),
                ArtifactKind::Job,
                class,
                age,
                format!("owning CronJob '{owner}' is gone; waiting out the {retention:?}s window"),
            )
        }
        OwnerState::Unowned => {
            if !spec.in_scope(&job.labels) {
                return Classification::retain(
                    job.id.clone(),
                    ArtifactKind::Job,
                    OrphanClass::DeletedOwnerRemnant,
                    age,
                    "no controller owner and outside the sweep scope".to_string(),
                );
            }
            if past_retention {
                return Classification::delete(
                    job.id.clone(),
                    ArtifactKind::Job,
                    OrphanClass::DeletedOwnerRemnant,
                    age,
                    format!("controller owner was removed; terminal for {age}s"),
                );
            }
            Classification::retain(
                job.id.clone(),
                ArtifactKind::Job,
                OrphanClass::DeletedOwnerRemnant,
                age,
                format!("controller owner was removed; waiting out the {retention:?}s window"),
            )
        }
    }
}

/// Classify one Pod against the cluster view and the retention policy.
///
/// A Pod is only reclaimed once it is terminal *and* its owning Job is itself
/// terminal (or gone), so a running Job never loses pods mid-flight.
pub fn classify_pod(
    view: &ClusterView,
    spec: &JobRetentionPolicySpec,
    pod: &PodObservation,
    now: DateTime<Utc>,
) -> Classification {
    let age = age_seconds(pod.age_basis(), now);
    let retention = i64::from(spec.pod_retention_seconds);
    let moved = was_moved(spec, pod.previous_namespace.as_ref(), &pod.id.namespace);
    let owner = pod.owner_refs.iter().find(|r| r.kind == "Job");

    let delete = |class: OrphanClass, reason: String| {
        Classification::delete(pod.id.clone(), ArtifactKind::Pod, class, age, reason)
    };
    let retain = |class: OrphanClass, reason: String| {
        Classification::retain(pod.id.clone(), ArtifactKind::Pod, class, age, reason)
    };

    let Some(owner) = owner else {
        if !spec.in_scope(&pod.labels) {
            return retain(
                OrphanClass::DeletedOwnerRemnant,
                "no controller owner and outside the sweep scope".to_string(),
            );
        }
        let class = if moved {
            OrphanClass::NamespaceMove
        } else {
            OrphanClass::DeletedOwnerRemnant
        };
        if pod.phase.is_terminal() && age >= retention && spec.delete_orphaned_pods {
            return delete(
                class,
                format!("controller owner was removed; Pod terminal for {age}s"),
            );
        }
        return retain(
            class,
            format!("controller owner was removed; waiting out the {retention}s pod window"),
        );
    };

    let Some(job) = view.jobs.get(&owner.uid) else {
        // Dangling Job ownerReference: either the Job was recreated under a new
        // UID, or it was deleted leaving this Pod behind.
        let recreated = view
            .jobs
            .values()
            .any(|j| j.id.namespace == pod.id.namespace && j.id.name == owner.name);
        let class = if recreated {
            OrphanClass::BrokenOwnerReference
        } else if moved {
            OrphanClass::NamespaceMove
        } else {
            OrphanClass::DeletedOwnerRemnant
        };
        if pod.phase.is_terminal() && age >= retention && spec.delete_orphaned_pods {
            return delete(
                class,
                format!(
                    "owning Job '{}' is gone; Pod terminal for {age}s",
                    owner.name
                ),
            );
        }
        return retain(
            class,
            format!(
                "owning Job '{}' is gone; waiting out the {retention}s pod window",
                owner.name
            ),
        );
    };

    if !pod.phase.is_terminal() {
        return retain(OrphanClass::CompletedPod, format!("Pod is {:?}", pod.phase));
    }
    if !job.phase.is_terminal() {
        return retain(
            OrphanClass::CompletedPod,
            format!("owning Job '{}' is still {:?}", job.id.name, job.phase),
        );
    }
    if age >= retention {
        if spec.delete_orphaned_pods {
            return delete(
                OrphanClass::CompletedPod,
                format!(
                    "Pod {:?} for terminal Job '{}' is {age}s old, past the {retention}s window",
                    pod.phase, job.id.name
                ),
            );
        }
        return retain(
            OrphanClass::CompletedPod,
            format!("Pod is {age}s old but pod deletion is disabled"),
        );
    }
    retain(
        OrphanClass::CompletedPod,
        format!("terminal Pod is {age}s old, inside the {retention}s window"),
    )
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// One planned operation, or one explicit decision to keep an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimTarget {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub class: OrphanClass,
    pub action: ReclaimAction,
    pub age_seconds: i64,
    pub reason: String,
    pub repair_uid: Option<String>,
}

impl From<Classification> for ReclaimTarget {
    fn from(c: Classification) -> Self {
        Self {
            id: c.id,
            kind: c.kind,
            class: c.class,
            action: c.action,
            age_seconds: c.age_seconds,
            reason: c.reason,
            repair_uid: c.repair_uid,
        }
    }
}

/// Aggregate counters for one sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanSummary {
    pub jobs_scanned: usize,
    pub pods_scanned: usize,
    pub jobs_deleted: usize,
    pub pods_deleted: usize,
    pub owner_refs_repaired: usize,
    pub retained: usize,
    pub by_class: ReclaimedByClass,
}

/// The complete, deterministic result of one sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReclaimPlan {
    /// Deletes and owner-reference repairs, sorted by `(kind, namespace, name)`.
    pub targets: Vec<ReclaimTarget>,
    /// Artifacts deliberately kept this cycle, same ordering.
    pub retained: Vec<ReclaimTarget>,
    pub summary: PlanSummary,
}

impl ReclaimPlan {
    /// Deletes only.
    pub fn deletes(&self) -> impl Iterator<Item = &ReclaimTarget> {
        self.targets
            .iter()
            .filter(|t| t.action == ReclaimAction::Delete)
    }

    /// Owner-reference repairs only.
    pub fn repairs(&self) -> impl Iterator<Item = &ReclaimTarget> {
        self.targets
            .iter()
            .filter(|t| t.action == ReclaimAction::RepairOwnerRef)
    }

    /// Is the sweep a no-op? (nothing to delete or repair)
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Terminal Pods still slated for deletion — must be empty once a sweep has
    /// converged (acceptance: zero orphaned job pods past the retention window).
    pub fn outstanding_orphan_pods(&self) -> Vec<&ReclaimTarget> {
        self.deletes()
            .filter(|t| t.kind == ArtifactKind::Pod)
            .collect()
    }
}

/// Classify every Job and Pod in `view` and produce the sweep plan.
///
/// Pure and deterministic: inputs are sorted, and `now` is explicit.
pub fn plan_reclaim(
    view: &ClusterView,
    spec: &JobRetentionPolicySpec,
    now: DateTime<Utc>,
) -> ReclaimPlan {
    let mut plan = ReclaimPlan {
        summary: PlanSummary {
            jobs_scanned: view.jobs.len(),
            pods_scanned: view.pods.len(),
            ..Default::default()
        },
        ..Default::default()
    };

    for job in view.sorted_jobs() {
        plan.push(classify_job(view, spec, job, now));
    }
    for pod in view.sorted_pods() {
        plan.push(classify_pod(view, spec, pod, now));
    }

    plan.targets
        .sort_by(|a, b| (a.kind, &a.id).cmp(&(b.kind, &b.id)));
    plan.retained
        .sort_by(|a, b| (a.kind, &a.id).cmp(&(b.kind, &b.id)));
    plan
}

impl ReclaimPlan {
    fn push(&mut self, c: Classification) {
        let target: ReclaimTarget = c.into();
        match target.action {
            ReclaimAction::Retain => {
                self.summary.retained += 1;
                self.retained.push(target);
            }
            ReclaimAction::Delete => {
                match target.kind {
                    ArtifactKind::Job => self.summary.jobs_deleted += 1,
                    ArtifactKind::Pod => self.summary.pods_deleted += 1,
                }
                target.class.tally(&mut self.summary.by_class);
                self.targets.push(target);
            }
            ReclaimAction::RepairOwnerRef => {
                self.summary.owner_refs_repaired += 1;
                target.class.tally(&mut self.summary.by_class);
                self.targets.push(target);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Kubernetes adapters
// ---------------------------------------------------------------------------

fn owner_refs_of(
    refs: &Option<Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference>>,
) -> Vec<OwnerRefSnapshot> {
    refs.as_deref()
        .unwrap_or_default()
        .iter()
        .map(|r| OwnerRefSnapshot {
            api_version: r.api_version.clone(),
            kind: r.kind.clone(),
            name: r.name.clone(),
            uid: r.uid.clone(),
            controller: r.controller,
            block_owner_deletion: r.block_owner_deletion,
        })
        .collect()
}

fn previous_namespace_of(
    annotations: &Option<BTreeMap<String, String>>,
    key: &str,
) -> Option<String> {
    annotations
        .as_ref()
        .and_then(|a| a.get(key))
        .filter(|v| !v.is_empty())
        .cloned()
}

/// Derive a Job's phase from its status.
///
/// An active pod wins over completions so a Job running several pods in parallel
/// is never mistaken for a finished one.
pub fn job_phase(job: &Job) -> JobPhase {
    if job.spec.as_ref().and_then(|s| s.suspend) == Some(true) {
        return JobPhase::Suspended;
    }
    let Some(status) = job.status.as_ref() else {
        return JobPhase::Pending;
    };
    if status.active.unwrap_or(0) > 0 {
        JobPhase::Active
    } else if status.succeeded.unwrap_or(0) > 0 {
        JobPhase::Succeeded
    } else if status.failed.unwrap_or(0) > 0 {
        JobPhase::Failed
    } else {
        JobPhase::Pending
    }
}

/// Reduce a live `Job` to a [`JobObservation`].
pub fn job_observation(
    job: &Job,
    spec: &JobRetentionPolicySpec,
    now: DateTime<Utc>,
) -> JobObservation {
    let annotations = job.metadata.annotations.clone();
    let uid = job.metadata.uid.clone().unwrap_or_default();
    JobObservation {
        id: ArtifactId::new(job.namespace().unwrap_or_default(), job.name_any()),
        uid,
        owner_refs: owner_refs_of(&job.metadata.owner_references),
        phase: job_phase(job),
        created_at: job
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0)
            .unwrap_or(now),
        finished_at: job
            .status
            .as_ref()
            .and_then(|s| s.completion_time.as_ref())
            .map(|t| t.0),
        labels: job.metadata.labels.clone().unwrap_or_default(),
        previous_namespace: previous_namespace_of(
            &annotations,
            &spec.previous_namespace_annotation,
        ),
    }
}

/// Derive a Pod's phase from `status.phase`.
pub fn pod_phase(pod: &Pod) -> PodPhase {
    match pod.status.as_ref().and_then(|s| s.phase.as_deref()) {
        Some("Pending") => PodPhase::Pending,
        Some("Running") => PodPhase::Running,
        Some("Succeeded") => PodPhase::Succeeded,
        Some("Failed") => PodPhase::Failed,
        _ => PodPhase::Unknown,
    }
}

fn pod_finished_at(pod: &Pod) -> Option<DateTime<Utc>> {
    pod.status
        .as_ref()?
        .container_statuses
        .as_ref()?
        .iter()
        .filter_map(|cs| cs.state.as_ref())
        .filter_map(|s| s.terminated.as_ref())
        .filter_map(|t| t.finished_at.as_ref().map(|t| t.0))
        .max()
}

/// Reduce a live `Pod` to a [`PodObservation`].
pub fn pod_observation(
    pod: &Pod,
    spec: &JobRetentionPolicySpec,
    now: DateTime<Utc>,
) -> PodObservation {
    let annotations = pod.metadata.annotations.clone();
    let uid = pod.metadata.uid.clone().unwrap_or_default();
    PodObservation {
        id: ArtifactId::new(pod.namespace().unwrap_or_default(), pod.name_any()),
        uid,
        owner_refs: owner_refs_of(&pod.metadata.owner_references),
        phase: pod_phase(pod),
        created_at: pod
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0)
            .unwrap_or(now),
        finished_at: pod_finished_at(pod),
        labels: pod.metadata.labels.clone().unwrap_or_default(),
        previous_namespace: previous_namespace_of(
            &annotations,
            &spec.previous_namespace_annotation,
        ),
    }
}

/// List the namespace's CronJobs, Jobs and Pods into a [`ClusterView`].
pub async fn collect_view(
    client: &Client,
    namespace: &str,
    spec: &JobRetentionPolicySpec,
) -> Result<ClusterView> {
    let now = Utc::now();
    let params = ListParams::default();

    let cronjobs: Api<CronJob> = Api::namespaced(client.clone(), namespace);
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    let (cronjob_list, job_list, pod_list) = tokio::try_join!(
        cronjobs.list(&params),
        jobs.list(&params),
        pods.list(&params),
    )?;

    let mut view = ClusterView::new();
    for cj in &cronjob_list.items {
        view.cronjobs.insert(
            (namespace.to_string(), cj.name_any()),
            cj.metadata.uid.clone().unwrap_or_default(),
        );
    }
    for job in &job_list.items {
        let obs = job_observation(job, spec, now);
        view.jobs.insert(obs.uid.clone(), obs);
    }
    for pod in &pod_list.items {
        let obs = pod_observation(pod, spec, now);
        view.pods.push(obs);
    }
    Ok(view)
}

// ---------------------------------------------------------------------------
// Reconciliation
// ---------------------------------------------------------------------------

/// What a sweep actually did against the cluster.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReclaimOutcome {
    pub jobs_deleted: u32,
    pub pods_deleted: u32,
    pub owner_refs_repaired: u32,
    /// Non-fatal per-artifact failures (RBAC, conflict, transient API errors).
    pub errors: Vec<String>,
}

impl ReclaimOutcome {
    /// Total artifacts reclaimed this sweep.
    pub fn total(&self) -> u32 {
        self.jobs_deleted + self.pods_deleted
    }
}

/// Build the `ownerReferences` array for a repaired artifact, replacing the UID
/// of the reference whose name matches `name` with `live_uid`.
pub fn repaired_owner_references(
    refs: &[OwnerRefSnapshot],
    name: &str,
    live_uid: &str,
) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|r| {
            if r.name == name && r.uid != live_uid {
                OwnerRefSnapshot {
                    uid: live_uid.to_string(),
                    ..r.clone()
                }
                .to_json()
            } else {
                r.to_json()
            }
        })
        .collect()
}

async fn delete_job(
    client: &Client,
    namespace: &str,
    name: &str,
    grace_period_seconds: i64,
) -> Result<()> {
    let api: Api<Job> = Api::namespaced(client.clone(), namespace);
    api.delete(
        name,
        &DeleteParams::default().grace_period(grace_period_seconds.max(0) as u32),
    )
    .await?;
    Ok(())
}

async fn delete_pod(
    client: &Client,
    namespace: &str,
    name: &str,
    grace_period_seconds: i64,
) -> Result<()> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    api.delete(
        name,
        &DeleteParams::default().grace_period(grace_period_seconds.max(0) as u32),
    )
    .await?;
    Ok(())
}

/// Is this a `NotFound` from the API server? Such a failure means the artifact
/// is already gone, which is the state the sweep wanted, so it is not an error.
fn is_not_found(e: &crate::error::Error) -> bool {
    matches!(
        e,
        crate::error::Error::KubeError(kube::Error::Api(a)) if a.code == 404
    )
}

async fn repair_job_owner_reference(
    client: &Client,
    view: &ClusterView,
    target: &ReclaimTarget,
) -> Result<()> {
    let live_uid = target.repair_uid.as_ref().ok_or_else(|| {
        crate::error::Error::ConfigError("repair target without a live UID".into())
    })?;
    let job = view
        .jobs
        .values()
        .find(|j| j.id == target.id)
        .ok_or_else(|| {
            crate::error::Error::ConfigError("repair target is not in the cluster view".into())
        })?;
    let owner_name = job
        .owner_refs
        .iter()
        .find(|r| r.kind == "CronJob")
        .map(|r| r.name.clone())
        .unwrap_or_default();
    let patch = serde_json::json!({
        "metadata": {
            "ownerReferences": repaired_owner_references(&job.owner_refs, &owner_name, live_uid)
        }
    });
    let api: Api<Job> = Api::namespaced(client.clone(), &target.id.namespace);
    api.patch(
        &target.id.name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;
    Ok(())
}

/// Execute a plan against the cluster.
///
/// In a dry run nothing is mutated and the planned counts are reported as if it
/// had succeeded, which keeps status and metrics comparable across modes.
pub async fn apply_plan(
    client: &Client,
    view: &ClusterView,
    spec: &JobRetentionPolicySpec,
    plan: &ReclaimPlan,
) -> ReclaimOutcome {
    let mut outcome = ReclaimOutcome::default();
    if spec.dry_run {
        debug!(
            targets = plan.targets.len(),
            "job orphan sweep is a dry run; not mutating the cluster"
        );
        outcome.jobs_deleted = plan.summary.jobs_deleted as u32;
        outcome.pods_deleted = plan.summary.pods_deleted as u32;
        outcome.owner_refs_repaired = plan.summary.owner_refs_repaired as u32;
        return outcome;
    }

    for target in &plan.targets {
        let result: Result<()> = match (target.action, target.kind) {
            (ReclaimAction::Delete, ArtifactKind::Job) => {
                delete_job(
                    client,
                    &target.id.namespace,
                    &target.id.name,
                    spec.grace_period_seconds,
                )
                .await
            }
            (ReclaimAction::Delete, ArtifactKind::Pod) => {
                delete_pod(
                    client,
                    &target.id.namespace,
                    &target.id.name,
                    spec.grace_period_seconds,
                )
                .await
            }
            (ReclaimAction::RepairOwnerRef, _) => {
                repair_job_owner_reference(client, view, target).await
            }
            (ReclaimAction::Retain, _) => Ok(()),
        };
        match result {
            Ok(()) => match target.action {
                ReclaimAction::Delete => match target.kind {
                    ArtifactKind::Job => outcome.jobs_deleted += 1,
                    ArtifactKind::Pod => outcome.pods_deleted += 1,
                },
                ReclaimAction::RepairOwnerRef => outcome.owner_refs_repaired += 1,
                ReclaimAction::Retain => {}
            },
            Err(e) if is_not_found(&e) => {
                debug!(
                    kind = target.kind.kube_kind(),
                    name = %target.id.name,
                    "artifact already gone; nothing to reclaim"
                );
            }
            Err(e) => {
                let msg = format!("{} {}: {e}", target.kind.kube_kind(), target.id.name);
                warn!(target = %msg, "failed to reclaim orphaned artifact");
                outcome.errors.push(msg);
            }
        }
    }
    outcome
}

/// Render a plan as a human-readable table.
pub fn format_plan_table(plan: &ReclaimPlan) -> String {
    let mut out = format!(
        "Job orphan sweep — jobs: {} scanned / {} deleted, pods: {} scanned / {} deleted, \
         owner refs repaired: {}, retained: {}\n",
        plan.summary.jobs_scanned,
        plan.summary.jobs_deleted,
        plan.summary.pods_scanned,
        plan.summary.pods_deleted,
        plan.summary.owner_refs_repaired,
        plan.summary.retained,
    );
    if plan.targets.is_empty() && plan.retained.is_empty() {
        out.push_str("No Job/CronJob artifacts found.\n");
        return out;
    }
    out.push_str(&format!(
        "\n{:<6} {:<40} {:<20} {:<26} {:>8}  {}\n",
        "KIND", "NAME", "NAMESPACE", "CLASS", "AGE(s)", "REASON"
    ));
    out.push_str(&"-".repeat(140));
    out.push('\n');
    for t in plan.targets.iter().chain(plan.retained.iter()) {
        let action = match t.action {
            ReclaimAction::Delete => "delete",
            ReclaimAction::RepairOwnerRef => "repair",
            ReclaimAction::Retain => "retain",
        };
        out.push_str(&format!(
            "{:<6} {:<40} {:<20} {:<26} {:>8}  [{action}] {}\n",
            t.kind.as_str(),
            t.id.name,
            t.id.namespace,
            t.class.as_str(),
            t.age_seconds,
            t.reason
        ));
    }
    out
}

/// Build the status to persist for a completed sweep.
pub fn sweep_status(
    spec: &JobRetentionPolicySpec,
    plan: &ReclaimPlan,
    outcome: &ReclaimOutcome,
    now: DateTime<Utc>,
) -> JobRetentionPolicyStatus {
    let reclaimed_jobs = if spec.dry_run {
        plan.summary.jobs_deleted as u32
    } else {
        outcome.jobs_deleted
    };
    let reclaimed_pods = if spec.dry_run {
        plan.summary.pods_deleted as u32
    } else {
        outcome.pods_deleted
    };
    let repaired = if spec.dry_run {
        plan.summary.owner_refs_repaired as u32
    } else {
        outcome.owner_refs_repaired
    };
    let mut by_class = ReclaimedByClass::default();
    for target in plan.targets.iter() {
        target.class.tally(&mut by_class);
    }
    let healthy = outcome.errors.is_empty();
    JobRetentionPolicyStatus {
        reclaimed_jobs,
        reclaimed_pods,
        repaired_owner_references: repaired,
        reclaimed_by_class: by_class,
        retained: plan.summary.retained as u32,
        dry_run: spec.dry_run,
        last_reconciled: Some(now),
        conditions: vec![crate::crd::types::Condition::ready(
            healthy,
            if healthy { "Swept" } else { "PartiallySwept" },
            &format!(
                "reclaimed {reclaimed_jobs} job(s) and {reclaimed_pods} pod(s), \
                 repaired {repaired} owner reference(s), {} error(s)",
                outcome.errors.len()
            ),
        )],
    }
}

/// Record sweep results as Prometheus metrics, labelled per namespace.
pub fn record_metrics(namespace: &str, plan: &ReclaimPlan, outcome: &ReclaimOutcome) {
    #[cfg(feature = "metrics")]
    {
        for target in plan.targets.iter() {
            let n = match target.action {
                ReclaimAction::Delete => 1,
                ReclaimAction::RepairOwnerRef => 1,
                ReclaimAction::Retain => 0,
            };
            if n == 0 {
                continue;
            }
            metrics::inc_job_orphan_reclaimed(
                namespace,
                target.kind.as_str(),
                target.class.as_str(),
                n,
            );
        }
        metrics::set_job_orphans_outstanding(
            namespace,
            plan.outstanding_orphan_pods().len() as i64,
        );
        let _ = outcome;
    }
    #[cfg(not(feature = "metrics"))]
    {
        let _ = (namespace, plan, outcome);
    }
}

/// Reconcile one `JobRetentionPolicy`.
///
/// Lists the namespace's batch artifacts, plans a sweep, applies it (unless
/// `spec.dryRun`), records metrics and patches the status subresource.
pub async fn reconcile_job_retention(
    client: &Client,
    policy: &JobRetentionPolicy,
) -> Result<JobRetentionPolicyStatus> {
    let namespace = policy.namespace().unwrap_or_else(|| "default".to_string());
    let spec = policy.spec.clone();
    let now = Utc::now();

    let view = collect_view(client, &namespace, &spec).await?;
    let plan = plan_reclaim(&view, &spec, now);
    info!(
        namespace = %namespace,
        policy = %policy.name_any(),
        jobs = plan.summary.jobs_scanned,
        pods = plan.summary.pods_scanned,
        deletes = plan.targets.iter().filter(|t| t.action == ReclaimAction::Delete).count(),
        "planned Job/CronJob orphan sweep\n{}",
        format_plan_table(&plan)
    );

    let outcome = apply_plan(client, &view, &spec, &plan).await;
    record_metrics(&namespace, &plan, &outcome);
    let status = sweep_status(&spec, &plan, &outcome, now);

    let api: Api<JobRetentionPolicy> = Api::all(client.clone());
    api.patch_status(
        &policy.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(serde_json::json!({ "status": status })),
    )
    .await?;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "stellar";

    /// Fixed "current" time (2026-01-01T12:00:00Z) so every test is deterministic.
    fn now() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_767_225_600, 0).expect("valid timestamp")
    }

    fn at(secs_ago: i64) -> DateTime<Utc> {
        now() - chrono::Duration::seconds(secs_ago)
    }

    fn spec() -> JobRetentionPolicySpec {
        JobRetentionPolicySpec {
            completed_job_retention_seconds: 600,
            failed_job_retention_seconds: 60,
            stuck_job_grace_seconds: 3_600,
            pod_retention_seconds: 300,
            ..Default::default()
        }
    }

    fn managed_labels() -> BTreeMap<String, String> {
        BTreeMap::from([(
            "app.kubernetes.io/managed-by".to_string(),
            "stellar-operator".to_string(),
        )])
    }

    fn job(
        name: &str,
        owner: Option<OwnerRefSnapshot>,
        phase: JobPhase,
        created_ago: i64,
        finished_ago: Option<i64>,
    ) -> JobObservation {
        JobObservation {
            id: ArtifactId::new(NS, name),
            uid: format!("uid-{name}"),
            owner_refs: owner.into_iter().collect(),
            phase,
            created_at: at(created_ago),
            finished_at: finished_ago.map(at),
            labels: managed_labels(),
            previous_namespace: None,
        }
    }

    fn pod(
        name: &str,
        owner: Option<OwnerRefSnapshot>,
        phase: PodPhase,
        created_ago: i64,
        finished_ago: Option<i64>,
    ) -> PodObservation {
        PodObservation {
            id: ArtifactId::new(NS, name),
            uid: format!("uid-{name}"),
            owner_refs: owner.into_iter().collect(),
            phase,
            created_at: at(created_ago),
            finished_at: finished_ago.map(at),
            labels: managed_labels(),
            previous_namespace: None,
        }
    }

    fn base_view() -> ClusterView {
        ClusterView::new()
            .with_cronjob(NS, "ledger-sync", "cron-uid-1")
            .with_cronjob(NS, "snapshot", "cron-uid-2")
    }

    // -- Orphan class 1: deleted CronJob ----------------------------------

    #[test]
    fn class1_deleted_cron_job_is_reclaimed_after_retention() {
        let s = spec();
        let view = base_view();
        let orphan = job(
            "ledger-sync-2999999",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "gone-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &orphan, now());
        assert_eq!(c.class, OrphanClass::DeletedCronJob);
        assert_eq!(c.action, ReclaimAction::Delete);
        assert_eq!(c.age_seconds, 7_000);
        assert!(c.reason.contains("no longer exists"), "{}", c.reason);
    }

    #[test]
    fn class1_deleted_cron_job_inside_retention_is_retained() {
        let s = spec();
        let view = base_view();
        let fresh = job(
            "ledger-sync-3000000",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "gone-uid")),
            JobPhase::Succeeded,
            120,
            Some(60),
        );
        let c = classify_job(&view, &s, &fresh, now());
        assert_eq!(c.action, ReclaimAction::Retain);
        assert!(c.reason.contains("waiting out"), "{}", c.reason);
    }

    #[test]
    fn class1_deleted_cron_job_respects_disabled_job_deletion() {
        let mut s = spec();
        s.delete_orphaned_jobs = false;
        let view = base_view();
        let orphan = job(
            "ledger-sync-1",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "gone-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &orphan, now());
        assert_eq!(c.action, ReclaimAction::Retain);
        assert!(c.reason.contains("deletion is disabled"), "{}", c.reason);
    }

    // -- Orphan class 2: broken ownerReference ----------------------------

    #[test]
    fn class2_recreated_cron_job_triggers_owner_reference_repair() {
        let s = spec();
        let view = base_view();
        let stale = job(
            "ledger-sync-2800000",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "stale-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &stale, now());
        assert_eq!(c.class, OrphanClass::BrokenOwnerReference);
        assert_eq!(c.action, ReclaimAction::RepairOwnerRef);
        assert_eq!(c.repair_uid.as_deref(), Some("cron-uid-1"));
        assert!(c.reason.contains("stale UID"), "{}", c.reason);
    }

    #[test]
    fn class2_owner_repair_can_be_disabled() {
        let mut s = spec();
        s.reconcile_owner_references = false;
        let view = base_view();
        let stale = job(
            "ledger-sync-2800000",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "stale-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &stale, now());
        assert_eq!(c.action, ReclaimAction::Retain);
        assert!(c.reason.contains("repair is disabled"), "{}", c.reason);
    }

    #[test]
    fn class2_stuck_failed_job_with_disabled_repair_is_deleted() {
        let mut s = spec();
        s.reconcile_owner_references = false;
        let view = base_view();
        let stale = job(
            "ledger-sync-2800001",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "stale-uid")),
            JobPhase::Failed,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &stale, now());
        assert_eq!(c.class, OrphanClass::StuckFailedJob);
        assert_eq!(c.action, ReclaimAction::Delete);
    }

    #[test]
    fn repaired_owner_references_rewrites_only_the_stale_uid() {
        let refs = vec![
            OwnerRefSnapshot::cron_job("ledger-sync", "stale-uid"),
            OwnerRefSnapshot::job("other", "keep-me"),
        ];
        let out = repaired_owner_references(&refs, "ledger-sync", "live-uid");
        assert_eq!(out[0]["uid"], "live-uid");
        assert_eq!(out[0]["kind"], "CronJob");
        assert_eq!(out[0]["name"], "ledger-sync");
        assert_eq!(out[1]["uid"], "keep-me");
    }

    #[test]
    fn a_pod_whose_job_was_recreated_is_reclaimed() {
        let s = spec();
        let view = base_view().with_job(job(
            "ledger-sync-2900000",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Active,
            60,
            None,
        ));
        let p = pod(
            "ledger-sync-2900000-x1",
            Some(OwnerRefSnapshot::job("ledger-sync-2900000", "dead-job-uid")),
            PodPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_pod(&view, &s, &p, now());
        assert_eq!(c.class, OrphanClass::BrokenOwnerReference);
        assert_eq!(c.action, ReclaimAction::Delete);
    }

    // -- Orphan class 3: stuck failed job ---------------------------------

    #[test]
    fn class3_stuck_failed_job_of_a_live_cron_job_is_reclaimed() {
        let s = spec();
        let view = base_view();
        let stuck = job(
            "snapshot-123",
            Some(OwnerRefSnapshot::cron_job("snapshot", "cron-uid-2")),
            JobPhase::Failed,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &stuck, now());
        assert_eq!(c.class, OrphanClass::StuckFailedJob);
        assert_eq!(c.action, ReclaimAction::Delete);
        assert!(c.reason.contains("stuck-job window"), "{}", c.reason);
    }

    #[test]
    fn class3_recently_failed_job_respects_the_stuck_grace_floor() {
        let s = spec();
        assert_eq!(s.effective_failed_job_retention_seconds(), 3_600);
        let view = base_view();
        let recent = job(
            "snapshot-124",
            Some(OwnerRefSnapshot::cron_job("snapshot", "cron-uid-2")),
            JobPhase::Failed,
            300,
            Some(240),
        );
        let c = classify_job(&view, &s, &recent, now());
        assert_eq!(c.action, ReclaimAction::Retain);
    }

    #[test]
    fn class3_failed_job_uses_finished_at_not_creation_time() {
        let s = spec();
        let view = base_view();
        let j = job(
            "snapshot-125",
            Some(OwnerRefSnapshot::cron_job("snapshot", "cron-uid-2")),
            JobPhase::Failed,
            86_400,
            Some(120),
        );
        let c = classify_job(&view, &s, &j, now());
        assert_eq!(c.age_seconds, 120);
        assert_eq!(c.action, ReclaimAction::Retain);
    }

    #[test]
    fn a_failed_job_of_a_deleted_cron_job_is_classified_as_a_deleted_cron_job() {
        let s = spec();
        let view = base_view();
        let j = job(
            "gone-1",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "gone-uid")),
            JobPhase::Failed,
            7_200,
            Some(7_000),
        );
        let c = classify_job(&view, &s, &j, now());
        assert_eq!(c.class, OrphanClass::DeletedCronJob);
        assert_eq!(c.action, ReclaimAction::Delete);
    }

    // -- Orphan class 4: completed pods -----------------------------------

    #[test]
    fn class4_completed_pod_past_retention_is_reclaimed() {
        let s = spec();
        let view = base_view().with_job(job(
            "ledger-sync-2700000",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        let p = pod(
            "ledger-sync-2700000-k2q",
            Some(OwnerRefSnapshot::job(
                "ledger-sync-2700000",
                "uid-ledger-sync-2700000",
            )),
            PodPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_pod(&view, &s, &p, now());
        assert_eq!(c.class, OrphanClass::CompletedPod);
        assert_eq!(c.action, ReclaimAction::Delete);
        assert!(c.reason.contains("terminal Job"), "{}", c.reason);
    }

    #[test]
    fn class4_completed_pod_inside_retention_is_retained() {
        let s = spec();
        let view = base_view().with_job(job(
            "ledger-sync-2700001",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Succeeded,
            120,
            Some(60),
        ));
        let p = pod(
            "ledger-sync-2700001-k2q",
            Some(OwnerRefSnapshot::job(
                "ledger-sync-2700001",
                "uid-ledger-sync-2700001",
            )),
            PodPhase::Succeeded,
            120,
            Some(60),
        );
        let c = classify_pod(&view, &s, &p, now());
        assert_eq!(c.action, ReclaimAction::Retain);
        assert!(c.reason.contains("inside the"), "{}", c.reason);
    }

    // -- Orphan class 5: orphans after a namespace move --------------------

    #[test]
    fn class5_namespace_move_remnant_is_reclaimed() {
        let s = spec();
        let view = base_view();
        let mut j = job(
            "ledger-sync-1",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "old-cron-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        j.previous_namespace = Some("stellar-old".to_string());
        let c = classify_job(&view, &s, &j, now());
        assert_eq!(c.class, OrphanClass::NamespaceMove);
        assert_eq!(c.action, ReclaimAction::Delete);
        assert!(c.reason.contains("stellar-old"), "{}", c.reason);
    }

    #[test]
    fn class5_namespace_move_can_be_excluded() {
        let mut s = spec();
        s.reclaim_after_namespace_move = false;
        let view = base_view();
        let mut j = job(
            "ledger-sync-1",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "old-cron-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        j.previous_namespace = Some("stellar-old".to_string());
        let c = classify_job(&view, &s, &j, now());
        assert_eq!(c.class, OrphanClass::DeletedCronJob);
        assert_eq!(c.action, ReclaimAction::Delete);
    }

    #[test]
    fn class5_namespace_move_pod_with_a_live_job_is_still_classified() {
        let s = spec();
        let view = base_view().with_job(job(
            "ledger-sync-1",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "old-cron-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        let mut p = pod(
            "ledger-sync-1-abc",
            Some(OwnerRefSnapshot::job("ledger-sync-1", "uid-ledger-sync-1")),
            PodPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        p.previous_namespace = Some("stellar-old".to_string());
        // The owning Job UID resolves, so the Pod is treated as a completed pod
        // of a terminal Job and reclaimed — no premature deletion of active work.
        let c = classify_pod(&view, &s, &p, now());
        assert_eq!(c.class, OrphanClass::CompletedPod);
        assert_eq!(c.action, ReclaimAction::Delete);
    }

    // -- Safety: no premature deletion ------------------------------------

    #[test]
    fn active_and_pending_jobs_are_never_deleted() {
        let s = spec();
        let view = base_view();
        for phase in [JobPhase::Active, JobPhase::Pending, JobPhase::Suspended] {
            let mut j = job(
                "live-1",
                Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
                phase,
                999_999,
                None,
            );
            j.id = ArtifactId::new(NS, format!("live-{phase:?}"));
            let c = classify_job(&view, &s, &j, now());
            assert_eq!(c.action, ReclaimAction::Retain, "{phase:?} was deleted");
        }
    }

    #[test]
    fn active_and_pending_pods_are_never_deleted() {
        let s = spec();
        let view = base_view().with_job(job(
            "running-job",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Active,
            7_200,
            None,
        ));
        for phase in [PodPhase::Pending, PodPhase::Running, PodPhase::Unknown] {
            let mut p = pod(
                "running-pod",
                Some(OwnerRefSnapshot::job("running-job", "uid-running-job")),
                phase,
                999_999,
                None,
            );
            p.id = ArtifactId::new(NS, format!("running-{phase:?}"));
            let c = classify_pod(&view, &s, &p, now());
            assert_eq!(c.action, ReclaimAction::Retain, "{phase:?} was deleted");
        }
    }

    #[test]
    fn a_terminal_pod_of_a_running_job_is_retained() {
        let s = spec();
        let view = base_view().with_job(job(
            "parallel-job",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Active,
            7_200,
            None,
        ));
        let p = pod(
            "parallel-pod-done",
            Some(OwnerRefSnapshot::job("parallel-job", "uid-parallel-job")),
            PodPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        let c = classify_pod(&view, &s, &p, now());
        assert_eq!(c.action, ReclaimAction::Retain);
        assert!(c.reason.contains("still Active"), "{}", c.reason);
    }

    #[test]
    fn unowned_artifacts_outside_the_sweep_scope_are_never_touched() {
        let s = spec();
        let view = base_view();
        let mut j = job("byo", None, JobPhase::Succeeded, 7_200, Some(7_000));
        j.labels = BTreeMap::new();
        let c = classify_job(&view, &s, &j, now());
        assert_eq!(c.action, ReclaimAction::Retain);
        assert!(c.reason.contains("outside the sweep scope"), "{}", c.reason);

        let mut p = pod("byo-pod", None, PodPhase::Succeeded, 7_200, Some(7_000));
        p.labels = BTreeMap::new();
        let c = classify_pod(&view, &s, &p, now());
        assert_eq!(c.action, ReclaimAction::Retain);
    }

    #[test]
    fn unowned_managed_artifacts_are_reclaimed() {
        let s = spec();
        let view = base_view();
        let j = job("no-owner", None, JobPhase::Succeeded, 7_200, Some(7_000));
        assert_eq!(
            classify_job(&view, &s, &j, now()).class,
            OrphanClass::DeletedOwnerRemnant
        );
        let p = pod("no-owner-pod", None, PodPhase::Failed, 7_200, Some(7_000));
        assert_eq!(
            classify_pod(&view, &s, &p, now()).class,
            OrphanClass::DeletedOwnerRemnant
        );
    }

    #[test]
    fn pod_deletion_can_be_disabled() {
        let mut s = spec();
        s.delete_orphaned_pods = false;
        let view = base_view().with_job(job(
            "j1",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        let p = pod(
            "j1-pod",
            Some(OwnerRefSnapshot::job("j1", "uid-j1")),
            PodPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        assert_eq!(
            classify_pod(&view, &s, &p, now()).action,
            ReclaimAction::Retain
        );
    }

    // -- CronJob spec changes mid-cycle -----------------------------------

    #[test]
    fn cron_job_spec_changes_mid_cycle_do_not_produce_partial_decisions() {
        let s = spec();
        // Cycle 1: the CronJob is live, so the Job is retained.
        let cycle1 = base_view().with_job(job(
            "ledger-sync-1",
            Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        let plan1 = plan_reclaim(&cycle1, &s, now());
        assert!(plan1.is_empty());
        assert_eq!(plan1.summary.retained, 1);

        // Cycle 2: the CronJob spec changed and the owner was recreated. The
        // *old* Job is now a broken ownerReference and is repaired, not deleted,
        // and the new Job (fresh UID) is untouched.
        let cycle2 = base_view()
            .with_cronjob(NS, "ledger-sync", "cron-uid-v2")
            .with_job(job(
                "ledger-sync-1",
                Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_job(job(
                "ledger-sync-2",
                Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-v2")),
                JobPhase::Active,
                5,
                None,
            ));
        let plan2 = plan_reclaim(&cycle2, &s, now());
        assert_eq!(plan2.repairs().count(), 1);
        assert_eq!(plan2.repairs().next().unwrap().id.name, "ledger-sync-1");
        assert_eq!(plan2.deletes().count(), 0);
        assert!(plan2
            .retained
            .iter()
            .any(|t| t.id.name == "ledger-sync-2" && t.action == ReclaimAction::Retain));
    }

    #[test]
    fn suspended_cron_job_does_not_cause_job_reclamation() {
        let s = spec();
        let view = base_view().with_job(job(
            "snapshot-1",
            Some(OwnerRefSnapshot::cron_job("snapshot", "cron-uid-2")),
            JobPhase::Suspended,
            86_400,
            None,
        ));
        let plan = plan_reclaim(&view, &s, now());
        assert!(plan.is_empty());
    }

    // -- Planning ----------------------------------------------------------

    #[test]
    fn plan_is_deterministic_and_sorted() {
        let s = spec();
        let view = base_view()
            .with_job(job(
                "z-job",
                Some(OwnerRefSnapshot::cron_job("retired-sync", "gone")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_job(job(
                "a-job",
                Some(OwnerRefSnapshot::cron_job("retired-sync", "gone")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_pod(pod("m-pod", None, PodPhase::Succeeded, 7_200, Some(7_000)));
        let first = plan_reclaim(&view, &s, now());
        let second = plan_reclaim(&view, &s, now());
        assert_eq!(first, second, "planning must be deterministic");
        let names: Vec<_> = first.targets.iter().map(|t| t.id.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["a-job", "z-job", "m-pod"],
            "jobs sort before pods, then by name"
        );
        assert_eq!(first.summary.jobs_scanned, 2);
        assert_eq!(first.summary.pods_scanned, 1);
        assert_eq!(first.summary.jobs_deleted, 2);
        assert_eq!(first.summary.pods_deleted, 1);
    }

    #[test]
    fn plan_clears_every_orphan_class_in_one_sweep() {
        let s = spec();
        let live = base_view()
            .with_job(job(
                "live-job",
                Some(OwnerRefSnapshot::cron_job("ledger-sync", "cron-uid-1")),
                JobPhase::Active,
                30,
                None,
            ))
            .with_pod(pod(
                "live-pod",
                Some(OwnerRefSnapshot::job("live-job", "uid-live-job")),
                PodPhase::Running,
                30,
                None,
            ));

        // 1. deleted CronJob
        let view = live.clone().with_job(job(
            "orphan-cron",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "dead")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        // 2. broken ownerReference (recreated CronJob)
        let view = view
            .with_job(job(
                "broken-ref",
                Some(OwnerRefSnapshot::cron_job("ledger-sync", "stale-uid")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_cronjob(NS, "recreated", "cron-uid-9")
            .with_job(job(
                "broken-ref-2",
                Some(OwnerRefSnapshot::cron_job("recreated", "stale-uid")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ));
        // 3. stuck failed job
        let view = view.with_job(job(
            "stuck",
            Some(OwnerRefSnapshot::cron_job("snapshot", "cron-uid-2")),
            JobPhase::Failed,
            7_200,
            Some(7_000),
        ));
        // 4. completed pod
        let view = view.with_pod(pod(
            "done-pod",
            Some(OwnerRefSnapshot::job("orphan-cron", "uid-orphan-cron")),
            PodPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        // 5. namespace-move remnant
        let mut moved = job(
            "moved",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "old-uid")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        );
        moved.previous_namespace = Some("stellar-old".to_string());
        let view = view.with_job(moved);

        let plan = plan_reclaim(&view, &s, now());
        let seen: Vec<(OrphanClass, ReclaimAction)> =
            plan.targets.iter().map(|t| (t.class, t.action)).collect();
        for class in [
            OrphanClass::DeletedCronJob,
            OrphanClass::BrokenOwnerReference,
            OrphanClass::StuckFailedJob,
            OrphanClass::CompletedPod,
            OrphanClass::NamespaceMove,
        ] {
            assert!(
                seen.contains(&(class, ReclaimAction::Delete))
                    || seen.contains(&(class, ReclaimAction::RepairOwnerRef)),
                "class {class:?} not detected in {seen:?}"
            );
        }

        // Active work is untouched.
        assert!(plan
            .retained
            .iter()
            .any(|t| t.id.name == "live-job" && t.action == ReclaimAction::Retain));
        assert!(plan
            .retained
            .iter()
            .any(|t| t.id.name == "live-pod" && t.action == ReclaimAction::Retain));
    }

    #[test]
    fn converged_sweep_leaves_no_orphan_pods_past_retention() {
        let s = spec();
        let view = base_view()
            .with_job(job(
                "old-job",
                Some(OwnerRefSnapshot::cron_job("retired-sync", "gone")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_pod(pod(
                "old-pod",
                Some(OwnerRefSnapshot::job("old-job", "uid-old-job")),
                PodPhase::Succeeded,
                7_200,
                Some(7_000),
            ));
        let plan = plan_reclaim(&view, &s, now());
        assert_eq!(plan.outstanding_orphan_pods().len(), 1);

        // Apply the plan: the job and its pod are gone, so a re-plan is empty.
        let after = ClusterView::new().with_cronjob(NS, "ledger-sync", "cron-uid-1");
        let replan = plan_reclaim(&after, &s, now());
        assert!(replan.is_empty());
        assert!(replan.outstanding_orphan_pods().is_empty());
    }

    #[test]
    fn empty_view_produces_an_empty_plan() {
        let plan = plan_reclaim(&ClusterView::new(), &spec(), now());
        assert!(plan.is_empty());
        assert!(plan.retained.is_empty());
        assert_eq!(plan.summary, PlanSummary::default());
    }

    // -- Status and metrics -------------------------------------------------

    #[test]
    fn status_reports_reclaimed_counts_per_class() {
        let s = spec();
        let view = base_view()
            .with_job(job(
                "a",
                Some(OwnerRefSnapshot::cron_job("retired-sync", "gone")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_job(job(
                "b",
                Some(OwnerRefSnapshot::cron_job("snapshot", "cron-uid-2")),
                JobPhase::Failed,
                7_200,
                Some(7_000),
            ))
            .with_pod(pod(
                "b-pod",
                Some(OwnerRefSnapshot::job("b", "uid-b")),
                PodPhase::Failed,
                7_200,
                Some(7_000),
            ));
        let plan = plan_reclaim(&view, &s, now());
        let status = sweep_status(&s, &plan, &ReclaimOutcome::default(), now());
        assert_eq!(status.reclaimed_jobs, 2);
        assert_eq!(status.reclaimed_pods, 1);
        assert_eq!(status.reclaimed_by_class.deleted_cron_job, 1);
        assert_eq!(status.reclaimed_by_class.stuck_failed_job, 1);
        assert_eq!(status.reclaimed_by_class.completed_pod, 1);
        assert!(!status.dry_run);
        assert_eq!(status.conditions[0].status, "True");
        assert_eq!(status.conditions[0].reason, "Swept");
    }

    #[test]
    fn status_marks_partial_failures() {
        let s = spec();
        let view = base_view();
        let plan = plan_reclaim(&view, &s, now());
        let outcome = ReclaimOutcome {
            errors: vec!["Job stellar/x: boom".to_string()],
            ..Default::default()
        };
        let status = sweep_status(&s, &plan, &outcome, now());
        assert_eq!(status.conditions[0].status, "False");
        assert_eq!(status.conditions[0].reason, "PartiallySwept");
    }

    #[test]
    fn dry_run_status_uses_the_planned_counts() {
        let mut s = spec();
        s.dry_run = true;
        let view = base_view().with_job(job(
            "a",
            Some(OwnerRefSnapshot::cron_job("retired-sync", "gone")),
            JobPhase::Succeeded,
            7_200,
            Some(7_000),
        ));
        let plan = plan_reclaim(&view, &s, now());
        let status = sweep_status(&s, &plan, &ReclaimOutcome::default(), now());
        assert!(status.dry_run);
        assert_eq!(status.reclaimed_jobs, 1);
    }

    #[test]
    fn record_metrics_runs_for_any_plan() {
        let plan = plan_reclaim(&ClusterView::new(), &spec(), now());
        record_metrics(NS, &plan, &ReclaimOutcome::default());
    }

    #[test]
    fn orphan_class_labels_are_stable() {
        for class in OrphanClass::all() {
            assert!(!class.as_str().is_empty());
            assert!(class
                .as_str()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'));
        }
        assert_eq!(ArtifactKind::Job.kube_kind(), "Job");
        assert_eq!(ArtifactKind::Pod.kube_kind(), "Pod");
    }

    // -- Kubernetes adapters ------------------------------------------------

    #[test]
    fn job_phase_prefers_active_over_completions() {
        let mut job = k8s_openapi::api::batch::v1::Job {
            metadata: Default::default(),
            spec: Some(k8s_openapi::api::batch::v1::JobSpec {
                ..Default::default()
            }),
            status: Some(k8s_openapi::api::batch::v1::JobStatus {
                active: Some(1),
                succeeded: Some(2),
                failed: Some(0),
                ..Default::default()
            }),
        };
        assert_eq!(job_phase(&job), JobPhase::Active);
        job.status.as_mut().unwrap().active = Some(0);
        assert_eq!(job_phase(&job), JobPhase::Succeeded);
        job.status.as_mut().unwrap().succeeded = Some(0);
        job.status.as_mut().unwrap().failed = Some(3);
        assert_eq!(job_phase(&job), JobPhase::Failed);
        job.spec.as_mut().unwrap().suspend = Some(true);
        assert_eq!(job_phase(&job), JobPhase::Suspended);
        job.status = None;
        job.spec = None;
        assert_eq!(job_phase(&job), JobPhase::Pending);
    }

    #[test]
    fn pod_phase_maps_status_phase_strings() {
        use k8s_openapi::api::core::v1::PodStatus;
        let pod_with = |phase: Option<&str>| k8s_openapi::api::core::v1::Pod {
            metadata: Default::default(),
            status: Some(PodStatus {
                phase: phase.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(pod_phase(&pod_with(Some("Pending"))), PodPhase::Pending);
        assert_eq!(pod_phase(&pod_with(Some("Running"))), PodPhase::Running);
        assert_eq!(pod_phase(&pod_with(Some("Succeeded"))), PodPhase::Succeeded);
        assert_eq!(pod_phase(&pod_with(Some("Failed"))), PodPhase::Failed);
        assert_eq!(pod_phase(&pod_with(Some("Weird"))), PodPhase::Unknown);
        assert_eq!(pod_phase(&pod_with(None)), PodPhase::Unknown);
    }

    #[test]
    fn observations_carry_owner_refs_labels_and_namespace_history() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference, Time};
        let meta = ObjectMeta {
            name: Some("j1".into()),
            namespace: Some(NS.into()),
            uid: Some("u1".into()),
            creation_timestamp: Some(Time(now())),
            labels: Some(managed_labels()),
            annotations: Some(BTreeMap::from([(
                "stellar.org/previous-namespace".to_string(),
                "stellar-old".to_string(),
            )])),
            owner_references: Some(vec![OwnerReference {
                api_version: "batch/v1".into(),
                kind: "CronJob".into(),
                name: "ledger-sync".into(),
                uid: "c1".into(),
                controller: Some(true),
                block_owner_deletion: Some(true),
            }]),
            ..Default::default()
        };
        let job = k8s_openapi::api::batch::v1::Job {
            metadata: meta.clone(),
            spec: None,
            status: Some(k8s_openapi::api::batch::v1::JobStatus {
                succeeded: Some(1),
                completion_time: Some(Time(at(60))),
                ..Default::default()
            }),
        };
        let obs = job_observation(&job, &spec(), now());
        assert_eq!(obs.id, ArtifactId::new(NS, "j1"));
        assert_eq!(obs.uid, "u1");
        assert_eq!(obs.phase, JobPhase::Succeeded);
        assert_eq!(obs.created_at, now());
        assert_eq!(obs.finished_at, Some(at(60)));
        assert_eq!(obs.age_basis(), at(60));
        assert_eq!(obs.owner_refs.len(), 1);
        assert_eq!(obs.owner_refs[0].name, "ledger-sync");
        assert_eq!(obs.previous_namespace.as_deref(), Some("stellar-old"));
        assert!(spec().in_scope(&obs.labels));

        let pod = k8s_openapi::api::core::v1::Pod {
            metadata,
            status: Some(k8s_openapi::api::core::v1::PodStatus {
                phase: Some("Succeeded".into()),
                ..Default::default()
            }),
        };
        let pobs = pod_observation(&pod, &spec(), now());
        assert_eq!(pobs.phase, PodPhase::Succeeded);
        assert!(pobs.finished_at.is_none());
        assert_eq!(pobs.age_basis(), now());
    }

    #[test]
    fn plan_table_lists_actions() {
        let s = spec();
        let view = base_view()
            .with_job(job(
                "orphan",
                Some(OwnerRefSnapshot::cron_job("retired-sync", "gone")),
                JobPhase::Succeeded,
                7_200,
                Some(7_000),
            ))
            .with_pod(pod("running", None, PodPhase::Running, 10, None));
        let table = format_plan_table(&plan_reclaim(&view, &s, now()));
        assert!(table.contains("KIND"), "{table}");
        assert!(table.contains("[delete] orphan"), "{table}");
        assert!(table.contains("[retain]"), "{table}");
    }

    #[test]
    fn empty_plan_table_says_so() {
        let table = format_plan_table(&plan_reclaim(&ClusterView::new(), &spec(), now()));
        assert!(table.contains("No Job/CronJob artifacts found."), "{table}");
    }
}
