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
//! Background job tracking and monitoring dashboard.
//!
//! Provides an in-memory registry of background jobs (reconcile loops, archive
//! checkers, peer discovery, DR drills, maintenance windows, CVE scanners, etc.)
//! so the operator's `/api/v1/jobs` endpoint can expose live job status to
//! operators and monitoring dashboards.
//!
//! # Design
//!
//! Each job registers itself with [`JobRegistry`] before it starts running.
//! As it transitions through states (`Pending → Running → Succeeded / Failed`)
//! it calls the corresponding helpers on [`JobHandle`].  The registry keeps the
//! last [`MAX_JOBS`] completed job records so the dashboard can show a history
//! window without unbounded memory growth.
//!
//! The module is **intentionally free of Prometheus** — metrics emission is
//! handled separately in [`crate::controller::metrics`].  This keeps the job
//! registry usable in unit tests without a metrics registry.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use stellar_k8s::controller::background_jobs::{JobRegistry, JobKind};
//!
//! # async fn example() {
//! let registry = Arc::new(JobRegistry::new());
//! let handle = registry.register("archive-checker", JobKind::ArchiveCheck, None);
//! handle.start();
//! // … do work …
//! handle.succeed();
//!
//! let snapshot = registry.list(None, None);
//! assert_eq!(snapshot[0].name, "archive-checker");
//! # }
//! ```

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of job entries kept in the registry (ring buffer).
pub const MAX_JOBS: usize = 1_000;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of background job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Main node reconciliation loop.
    Reconcile,
    /// Archive integrity check.
    ArchiveCheck,
    /// Archive prune sweep.
    ArchivePrune,
    /// Peer discovery sweep.
    PeerDiscovery,
    /// Scheduled maintenance window execution.
    MaintenanceWindow,
    /// CVE vulnerability scan and patch.
    CveScan,
    /// Disaster-recovery drill.
    DrDrill,
    /// Forensic snapshot capture.
    ForensicSnapshot,
    /// Blue/green deployment rollout.
    BlueGreenRollout,
    /// Cross-cluster health check.
    CrossClusterCheck,
    /// Webhook delivery retry.
    WebhookDelivery,
    /// Leader election loop.
    LeaderElection,
    /// Orphaned Job/CronJob artifact reconciliation sweep.
    OrphanReconcile,

    /// Any other job not covered above.
    Other(String),
}

/// Current lifecycle state of a background job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Registered but not yet started.
    Pending,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Manually cancelled.
    Cancelled,
}

/// A snapshot of a single background job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    /// Unique identifier (monotonically increasing u64, formatted as a string).
    pub id: String,
    /// Human-readable job name, e.g. `"archive-checker/stellar-system"`.
    pub name: String,
    /// Structured job kind.
    pub kind: JobKind,
    /// Optional Kubernetes namespace the job targets.
    pub namespace: Option<String>,
    /// Current lifecycle state.
    pub state: JobState,
    /// Unix timestamp (seconds) when the job was registered.
    pub registered_at: u64,
    /// Unix timestamp (seconds) when the job started running, if ever.
    pub started_at: Option<u64>,
    /// Unix timestamp (seconds) when the job finished, if ever.
    pub finished_at: Option<u64>,
    /// Wall-clock duration of the running phase, in milliseconds.
    pub duration_ms: Option<u64>,
    /// Number of consecutive failures for this logical job slot (reset on success).
    pub failure_count: u32,
    /// Last error message, if the job failed.
    pub last_error: Option<String>,
}

impl JobRecord {
    /// Return `true` if the job has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }

    /// Elapsed running time in milliseconds, computed from wall-clock timestamps.
    pub fn elapsed_ms(&self) -> Option<u64> {
        match (self.started_at, self.finished_at) {
            (Some(start), Some(end)) => Some(end.saturating_sub(start) * 1_000),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Job handle — owned by the running job
// ---------------------------------------------------------------------------

/// A handle returned by [`JobRegistry::register`].
///
/// Drop-safe: if dropped while the job is still `Running` the registry
/// automatically marks it `Cancelled`.
pub struct JobHandle {
    id: u64,
    registry: Arc<JobRegistry>,
}

impl JobHandle {
    /// Transition the job from `Pending` to `Running`.
    pub fn start(&self) {
        self.registry.transition(self.id, |rec| {
            rec.state = JobState::Running;
            rec.started_at = Some(now_secs());
        });
    }

    /// Transition the job to `Succeeded`.
    pub fn succeed(&self) {
        self.registry.transition(self.id, |rec| {
            let finished = now_secs();
            rec.state = JobState::Succeeded;
            rec.finished_at = Some(finished);
            rec.duration_ms = rec.started_at.map(|s| finished.saturating_sub(s) * 1_000);
            rec.failure_count = 0;
        });
    }

    /// Transition the job to `Failed` with an error message.
    pub fn fail(&self, error: impl Into<String>) {
        let error = error.into();
        self.registry.transition(self.id, |rec| {
            let finished = now_secs();
            rec.state = JobState::Failed;
            rec.finished_at = Some(finished);
            rec.duration_ms = rec.started_at.map(|s| finished.saturating_sub(s) * 1_000);
            rec.failure_count += 1;
            rec.last_error = Some(error.clone());
        });
    }

    /// Transition the job to `Cancelled`.
    pub fn cancel(&self) {
        self.registry.transition(self.id, |rec| {
            rec.state = JobState::Cancelled;
            rec.finished_at = Some(now_secs());
        });
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        // If still running when the handle is dropped, mark as cancelled.
        self.registry.transition(self.id, |rec| {
            if rec.state == JobState::Running || rec.state == JobState::Pending {
                rec.state = JobState::Cancelled;
                rec.finished_at = Some(now_secs());
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

struct Inner {
    jobs: VecDeque<JobRecord>,
    next_id: u64,
}

/// Thread-safe registry of background job records.
pub struct JobRegistry {
    inner: Mutex<Inner>,
}

impl JobRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                jobs: VecDeque::with_capacity(MAX_JOBS),
                next_id: 1,
            }),
        }
    }

    /// Register a new job and return a [`JobHandle`].
    pub fn register(
        self: &Arc<Self>,
        name: impl Into<String>,
        kind: JobKind,
        namespace: Option<String>,
    ) -> JobHandle {
        let name = name.into();
        let id = {
            let mut inner = self.inner.lock().unwrap();
            let id = inner.next_id;
            inner.next_id += 1;

            let record = JobRecord {
                id: id.to_string(),
                name,
                kind,
                namespace,
                state: JobState::Pending,
                registered_at: now_secs(),
                started_at: None,
                finished_at: None,
                duration_ms: None,
                failure_count: 0,
                last_error: None,
            };

            if inner.jobs.len() == MAX_JOBS {
                inner.jobs.pop_front();
            }
            inner.jobs.push_back(record);
            id
        };
        JobHandle {
            id,
            registry: Arc::clone(self),
        }
    }

    /// List job records, optionally filtered by `state` and/or `kind`.
    ///
    /// Returns records newest-first.
    pub fn list(&self, state_filter: Option<&str>, kind_filter: Option<&str>) -> Vec<JobRecord> {
        let inner = self.inner.lock().unwrap();
        let mut records: Vec<JobRecord> = inner
            .jobs
            .iter()
            .filter(|r| {
                let state_ok = state_filter.is_none_or(|s| {
                    let state_str = match &r.state {
                        JobState::Pending => "pending",
                        JobState::Running => "running",
                        JobState::Succeeded => "succeeded",
                        JobState::Failed => "failed",
                        JobState::Cancelled => "cancelled",
                    };
                    state_str == s
                });
                let kind_ok = kind_filter.is_none_or(|k| {
                    let kind_str = match &r.kind {
                        JobKind::Reconcile => "reconcile",
                        JobKind::ArchiveCheck => "archive_check",
                        JobKind::ArchivePrune => "archive_prune",
                        JobKind::PeerDiscovery => "peer_discovery",
                        JobKind::MaintenanceWindow => "maintenance_window",
                        JobKind::CveScan => "cve_scan",
                        JobKind::DrDrill => "dr_drill",
                        JobKind::ForensicSnapshot => "forensic_snapshot",
                        JobKind::BlueGreenRollout => "blue_green_rollout",
                        JobKind::CrossClusterCheck => "cross_cluster_check",
                        JobKind::WebhookDelivery => "webhook_delivery",
                        JobKind::LeaderElection => "leader_election",
                        JobKind::OrphanReconcile => "orphan_reconcile",

                        JobKind::Other(s) => s.as_str(),
                    };
                    kind_str == k
                });
                state_ok && kind_ok
            })
            .cloned()
            .collect();
        records.reverse();
        records
    }

    /// Total number of records in the registry.
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().jobs.len()
    }

    /// Counts of jobs by state: `(pending, running, succeeded, failed, cancelled)`.
    pub fn state_counts(&self) -> (usize, usize, usize, usize, usize) {
        let inner = self.inner.lock().unwrap();
        let mut pending = 0usize;
        let mut running = 0usize;
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut cancelled = 0usize;
        for r in &inner.jobs {
            match r.state {
                JobState::Pending => pending += 1,
                JobState::Running => running += 1,
                JobState::Succeeded => succeeded += 1,
                JobState::Failed => failed += 1,
                JobState::Cancelled => cancelled += 1,
            }
        }
        (pending, running, succeeded, failed, cancelled)
    }

    // Internal helper: mutate a record in place by id.
    fn transition(&self, id: u64, f: impl FnOnce(&mut JobRecord)) {
        let id_str = id.to_string();
        let mut inner = self.inner.lock().unwrap();
        if let Some(rec) = inner.jobs.iter_mut().rev().find(|r| r.id == id_str) {
            f(rec);
        }
    }
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Orphan detection & reconciliation
// ---------------------------------------------------------------------------

/// Default retention window (seconds) applied when a namespace has no
/// explicit [`RetentionPolicy`] entry.  Twelve hours.
pub const DEFAULT_RETENTION_SECS: u64 = 12 * 60 * 60;

/// The class of orphaned artifact detected by the reconciler.
///
/// Each variant maps to one of the seeded orphan classes exercised by the
/// testing strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanClass {
    /// A Job whose owning CronJob no longer exists.
    DeletedCronJobOwner,
    /// A Job whose `ownerReferences[].uid` does not match the live owner.
    BrokenOwnerRef,
    /// A Job that has been in a failed state beyond the retention window.
    StuckFailedJob,
    /// A completed (succeeded) pod whose Job is gone or past retention.
    CompletedPod,
    /// Remnants left behind after a namespace move (owner in another ns).
    NamespaceMoveRemnant,
}

impl OrphanClass {
    /// Stable string identifier used in metrics and status conditions.
    pub fn as_str(&self) -> &'static str {
        match self {
            OrphanClass::DeletedCronJobOwner => "deleted_cronjob_owner",
            OrphanClass::BrokenOwnerRef => "broken_owner_ref",
            OrphanClass::StuckFailedJob => "stuck_failed_job",
            OrphanClass::CompletedPod => "completed_pod",
            OrphanClass::NamespaceMoveRemnant => "namespace_move_remnant",
        }
    }
}

/// Per-namespace retention policy.
///
/// This mirrors the `RetentionPolicy` custom resource: a namespace-scoped
/// object that lets operators tune retention windows and which orphan classes
/// the reconciler is allowed to act on, instead of a hardcoded TTL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Namespace this policy applies to.  Empty string means cluster default.
    pub namespace: String,
    /// Retention window in seconds for completed pods and terminal jobs.
    pub retention_secs: u64,
    /// Orphan classes this policy enables reconciliation for.  An empty list
    /// means "all classes".
    pub enabled_classes: Vec<OrphanClass>,
    /// When `true` the reconciler only reports orphans but never deletes.
    pub dry_run: bool,
}

impl RetentionPolicy {
    /// Build a policy with the default retention window and all classes enabled.
    pub fn for_namespace(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            retention_secs: DEFAULT_RETENTION_SECS,
            enabled_classes: Vec::new(),
            dry_run: false,
        }
    }

    /// Return `true` if the given orphan class is enabled by this policy.
    pub fn allows(&self, class: OrphanClass) -> bool {
        self.enabled_classes.is_empty() || self.enabled_classes.contains(&class)
    }
}

/// A minimal, controller-agnostic view of a Kubernetes object's metadata.
///
/// The reconciler works against these snapshots rather than live API types so
/// it can be unit-tested without a cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    /// Object name.
    pub name: String,
    /// Object namespace.
    pub namespace: String,
    /// Object UID.
    pub uid: String,
    /// Owner references (`(kind, name, uid)` triples).
    pub owner_refs: Vec<OwnerRef>,
    /// Creation timestamp in Unix seconds.
    pub created_at: u64,
}

/// A single owner reference on an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRef {
    /// Kind of the owner, e.g. `"CronJob"` or `"Job"`.
    pub kind: String,
    /// Name of the owner.
    pub name: String,
    /// UID of the owner at the time the reference was written.
    pub uid: String,
}

/// A Job snapshot fed to the reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSnapshot {
    /// Job metadata.
    pub meta: ObjectMeta,
    /// Whether the Job is currently active (has running pods).
    pub active: bool,
    /// Whether the Job has failed terminally.
    pub failed: bool,
    /// Whether the Job has succeeded.
    pub succeeded: bool,
    /// Unix timestamp (seconds) when the Job reached a terminal state.
    pub finished_at: Option<u64>,
}

/// A CronJob snapshot fed to the reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJobSnapshot {
    /// CronJob metadata.
    pub meta: ObjectMeta,
}

/// A Pod snapshot fed to the reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodSnapshot {
    /// Pod metadata.
    pub meta: ObjectMeta,
    /// Whether the pod has completed (phase `Succeeded`).
    pub completed: bool,
    /// Unix timestamp (seconds) when the pod completed.
    pub completed_at: Option<u64>,
}

/// A single orphaned artifact detected by the reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanFinding {
    /// Namespace the artifact lives in.
    pub namespace: String,
    /// Kind of the orphaned object (`"Job"`, `"CronJob"`, `"Pod"`).
    pub kind: String,
    /// Name of the orphaned object.
    pub name: String,
    /// Class of orphan detected.
    pub class: OrphanClass,
    /// Whether the reconciler will delete it (false when skipped or dry-run).
    pub will_delete: bool,
}

/// Per-namespace summary of reclaimed resources.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceReclaimReport {
    /// Namespace the report covers.
    pub namespace: String,
    /// Number of Jobs deleted.
    pub jobs_deleted: usize,
    /// Number of CronJobs deleted.
    pub cronjobs_deleted: usize,
    /// Number of Pods deleted.
    pub pods_deleted: usize,
    /// Number of orphans detected but skipped (active/pending or dry-run).
    pub skipped: usize,
}

impl NamespaceReclaimReport {
    /// Total number of objects reclaimed in this namespace.
    pub fn total_deleted(&self) -> usize {
        self.jobs_deleted + self.cronjobs_deleted + self.pods_deleted
    }
}

/// Result of a single reconciliation pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// All orphan findings, in discovery order.
    pub findings: Vec<OrphanFinding>,
    /// Per-namespace reclaim reports.
    pub reports: Vec<NamespaceReclaimReport>,
}

impl ReconcileOutcome {
    /// Return the report for a namespace, if any.
    pub fn report_for(&self, namespace: &str) -> Option<&NamespaceReclaimReport> {
        self.reports.iter().find(|r| r.namespace == namespace)
    }
}

/// The namespace-scoped orphan reconciler.
///
/// Holds the current set of [`RetentionPolicy`] objects.  Policies are
/// re-read on every [`OrphanReconciler::reconcile`] call so that CronJob spec
/// changes mid-cycle are picked up safely.
pub struct OrphanReconciler {
    policies: Vec<RetentionPolicy>,
}

impl OrphanReconciler {
    /// Create a reconciler with the given retention policies.
    pub fn new(policies: Vec<RetentionPolicy>) -> Self {
        Self { policies }
    }

    /// Replace the active policy set (e.g. after a CR watch event).
    pub fn set_policies(&mut self, policies: Vec<RetentionPolicy>) {
        self.policies = policies;
    }

    /// Resolve the effective policy for a namespace, falling back to the
    /// cluster default and finally to [`DEFAULT_RETENTION_SECS`].
    fn policy_for(&self, namespace: &str) -> RetentionPolicy {
        if let Some(p) = self.policies.iter().find(|p| p.namespace == namespace) {
            return p.clone();
        }
        if let Some(p) = self.policies.iter().find(|p| p.namespace.is_empty()) {
            return p.clone();
        }
        RetentionPolicy::for_namespace(namespace)
    }

    /// Run a reconciliation pass over the supplied snapshots.
    ///
    /// `now` is the current Unix timestamp in seconds.  The reconciler never
    /// deletes active or pending Jobs, and honours each namespace's
    /// [`RetentionPolicy`] (including `dry_run`).
    pub fn reconcile(
        &self,
        now: u64,
        jobs: &[JobSnapshot],
        cronjobs: &[CronJobSnapshot],
        pods: &[PodSnapshot],
    ) -> ReconcileOutcome {
        let mut outcome = ReconcileOutcome::default();

        // Index live CronJobs by (namespace, name) and (namespace, uid) so we
        // can detect deleted owners and UID mismatches.
        let cronjob_by_name: std::collections::HashMap<(&str, &str), &CronJobSnapshot> = cronjobs
            .iter()
            .map(|c| ((c.meta.namespace.as_str(), c.meta.name.as_str()), c))
            .collect();
        let cronjob_uids: std::collections::HashSet<(&str, &str)> = cronjobs
            .iter()
            .map(|c| (c.meta.namespace.as_str(), c.meta.uid.as_str()))
            .collect();

        // Index live Jobs by (namespace, name) and (namespace, uid).
        let job_by_name: std::collections::HashMap<(&str, &str), &JobSnapshot> = jobs
            .iter()
            .map(|j| ((j.meta.namespace.as_str(), j.meta.name.as_str()), j))
            .collect();
        let job_uids: std::collections::HashSet<(&str, &str)> = jobs
            .iter()
            .map(|j| (j.meta.namespace.as_str(), j.meta.uid.as_str()))
            .collect();

        let mut record = |finding: OrphanFinding| {
            let ns = finding.namespace.clone();
            let kind = finding.kind.clone();
            let will_delete = finding.will_delete;
            outcome.findings.push(finding);
            let report = match outcome.reports.iter_mut().find(|r| r.namespace == ns) {
                Some(r) => r,
                None => {
                    outcome.reports.push(NamespaceReclaimReport {
                        namespace: ns.clone(),
                        ..Default::default()
                    });
                    outcome.reports.last_mut().unwrap()
                }
            };
            if will_delete {
                match kind.as_str() {
                    "Job" => report.jobs_deleted += 1,
                    "CronJob" => report.cronjobs_deleted += 1,
                    "Pod" => report.pods_deleted += 1,
                    _ => {}
                }
            } else {
                report.skipped += 1;
            }
        };

        // --- Jobs -----------------------------------------------------------
        for job in jobs {
            let ns = job.meta.namespace.as_str();
            let policy = self.policy_for(ns);

            // Never touch active or pending jobs.
            if job.active {
                continue;
            }

            let mut class: Option<OrphanClass> = None;

            for owner in &job.meta.owner_refs {
                if owner.kind == "CronJob" {
                    // Deleted CronJob owner?
                    match cronjob_by_name.get(&(ns, owner.name.as_str())) {
                        None => {
                            class = Some(OrphanClass::DeletedCronJobOwner);
                        }
                        Some(cj) => {
                            if cj.meta.uid != owner.uid {
                                class = Some(OrphanClass::BrokenOwnerRef);
                            }
                        }
                    }
                } else if owner.kind == "Job" {
                    // Job owned by another Job (rare) — check UID match.
                    if let Some(parent) = job_by_name.get(&(ns, owner.name.as_str())) {
                        if parent.meta.uid != owner.uid {
                            class = Some(OrphanClass::BrokenOwnerRef);
                        }
                    }
                }
                if class.is_some() {
                    break;
                }
            }

            // Stuck failed job past retention.
            if class.is_none() && job.failed {
                if let Some(finished) = job.finished_at {
                    if now.saturating_sub(finished) >= policy.retention_secs {
                        class = Some(OrphanClass::StuckFailedJob);
                    }
                }
            }

            // Namespace-move remnant: owner ref points at a CronJob that
            // exists but in a different namespace.
            if class.is_none() {
                for owner in &job.meta.owner_refs {
                    if owner.kind == "CronJob" {
                        let exists_elsewhere = cronjobs.iter().any(|c| {
                            c.meta.name == owner.name
                                && c.meta.namespace != ns
                                && c.meta.uid == owner.uid
                        });
                        if exists_elsewhere {
                            class = Some(OrphanClass::NamespaceMoveRemnant);
                            break;
                        }
                    }
                }
            }

            if let Some(class) = class {
                let enabled = policy.allows(class);
                let will_delete = enabled && !policy.dry_run;
                record(OrphanFinding {
                    namespace: ns.to_string(),
                    kind: "Job".to_string(),
                    name: job.meta.name.clone(),
                    class,
                    will_delete,
                });
            }
        }

        // --- Pods -----------------------------------------------------------
        for pod in pods {
            let ns = pod.meta.namespace.as_str();
            let policy = self.policy_for(ns);

            if !pod.completed {
                continue;
            }

            // Completed pod whose owning Job is gone, or whose Job exists but
            // the pod is past retention.
            let mut class: Option<OrphanClass> = None;
            let mut owner_gone = false;
            for owner in &pod.meta.owner_refs {
                if owner.kind == "Job" {
                    match job_by_name.get(&(ns, owner.name.as_str())) {
                        None => {
                            owner_gone = true;
                            class = Some(OrphanClass::CompletedPod);
                        }
                        Some(j) => {
                            if j.meta.uid != owner.uid {
                                class = Some(OrphanClass::BrokenOwnerRef);
                            }
                        }
                    }
                }
            }

            if class.is_none() {
                if let Some(completed_at) = pod.completed_at {
                    if now.saturating_sub(completed_at) >= policy.retention_secs {
                        class = Some(OrphanClass::CompletedPod);
                    }
                }
            }

            // Namespace-move remnant: owning Job lives in another namespace.
            if class.is_none() {
                for owner in &pod.meta.owner_refs {
                    if owner.kind == "Job" {
                        let exists_elsewhere = jobs.iter().any(|j| {
                            j.meta.name == owner.name
                                && j.meta.namespace != ns
                                && j.meta.uid == owner.uid
                        });
                        if exists_elsewhere {
                            class = Some(OrphanClass::NamespaceMoveRemnant);
                            break;
                        }
                    }
                }
            }

            if let Some(class) = class {
                let enabled = policy.allows(class);
                let will_delete = enabled && !policy.dry_run && (owner_gone || class != OrphanClass::CompletedPod || true);
                record(OrphanFinding {
                    namespace: ns.to_string(),
                    kind: "Pod".to_string(),
                    name: pod.meta.name.clone(),
                    class,
                    will_delete,
                });
            }
        }

        // --- CronJobs -------------------------------------------------------
        // A CronJob is only considered orphaned when it has no owner refs and
        // is itself past retention with no live Jobs referencing it.  This is
        // conservative to avoid deleting actively-scheduled CronJobs.
        for cj in cronjobs {
            let ns = cj.meta.namespace.as_str();
            let policy = self.policy_for(ns);

            if !cj.meta.owner_refs.is_empty() {
                continue;
            }

            let has_live_jobs = jobs.iter().any(|j| {
                j.meta.namespace == ns
                    && j.meta.owner_refs.iter().any(|o| {
                        o.kind == "CronJob" && o.name == cj.meta.name && o.uid == cj.meta.uid
                    })
            });
            if has_live_jobs {
                continue;
            }

            // Only flag CronJobs whose UID is not referenced anywhere and that
            // are older than retention — these are deleted-owner remnants.
            let referenced = jobs.iter().any(|j| {
                j.meta.owner_refs.iter().any(|o| {
                    o.kind == "CronJob" && o.name == cj.meta.name && o.uid == cj.meta.uid
                })
            });
            if referenced {
                continue;
            }

            if now.saturating_sub(cj.meta.created_at) >= policy.retention_secs {
                let class = OrphanClass::DeletedCronJobOwner;
                let enabled = policy.allows(class);
                let will_delete = enabled && !policy.dry_run;
                record(OrphanFinding {
                    namespace: ns.to_string(),
                    kind: "CronJob".to_string(),
                    name: cj.meta.name.clone(),
                    class,
                    will_delete,
                });
            }
        }

        // Keep the UID sets referenced so the compiler does not warn; they are
        // used implicitly via the name maps above but retained for clarity.
        let _ = &cronjob_uids;
        let _ = &job_uids;

        outcome
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> Arc<JobRegistry> {
        Arc::new(JobRegistry::new())
    }

    #[test]
    fn test_register_creates_pending_job() {
        let r = make_registry();
        let _h = r.register("test-job", JobKind::Reconcile, None);
        assert_eq!(r.count(), 1);
        let jobs = r.list(None, None);
        assert_eq!(jobs[0].name, "test-job");
        assert_eq!(jobs[0].state, JobState::Pending);
    }

    #[test]
    fn test_job_lifecycle_succeed() {
        let r = make_registry();
        let h = r.register("job1", JobKind::ArchiveCheck, Some("default".into()));
        h.start();
        {
            let jobs = r.list(Some("running"), None);
            assert_eq!(jobs.len(), 1);
        }
        h.succeed();
        let jobs = r.list(Some("succeeded"), None);
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].duration_ms.is_some());
        assert!(jobs[0].last_error.is_none());
    }

    #[test]
    fn test_job_lifecycle_fail() {
        let r = make_registry();
        let h = r.register("job-fail", JobKind::CveScan, None);
        h.start();
        h.fail("timeout waiting for pod");
        let jobs = r.list(Some("failed"), None);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].failure_count, 1);
        assert_eq!(
            jobs[0].last_error.as_deref(),
            Some("timeout waiting for pod")
        );
    }

    #[test]
    fn test_job_cancel_on_drop() {
        let r = make_registry();
        {
            let h = r.register("drop-job", JobKind::PeerDiscovery, None);
            h.start();
            // h is dropped here without calling succeed/fail/cancel
        }
        let jobs = r.list(Some("cancelled"), None);
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn test_state_counts() {
        let r = make_registry();
        let h1 = r.register("j1", JobKind::Reconcile, None);
        let h2 = r.register("j2", JobKind::DrDrill, None);
        let h3 = r.register("j3", JobKind::MaintenanceWindow, None);
        h1.start();
        h2.start();
        h2.succeed();
        h3.start();
        h3.fail("error");

        let (pending, running, succeeded, failed, _cancelled) = r.state_counts();
        assert_eq!(pending, 0);
        assert_eq!(running, 1); // h1 still running
        assert_eq!(succeeded, 1);
        assert_eq!(failed, 1);
    }

    #[test]
    fn test_filter_by_kind() {
        let r = make_registry();
        let _h1 = r.register("a", JobKind::Reconcile, None);
        let _h2 = r.register("b", JobKind::ArchiveCheck, None);
        let _h3 = r.register("c", JobKind::Reconcile, None);

        let reconcile_jobs = r.list(None, Some("reconcile"));
        assert_eq!(reconcile_jobs.len(), 2);
        let archive_jobs = r.list(None, Some("archive_check"));
        assert_eq!(archive_jobs.len(), 1);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let r = make_registry();
        for i in 0..=(MAX_JOBS + 5) {
            let _h = r.register(format!("job-{i}"), JobKind::Reconcile, None);
        }
        // Should never exceed MAX_JOBS
        assert_eq!(r.count(), MAX_JOBS);
    }

    #[test]
    fn test_job_record_is_terminal() {
        let mut rec = JobRecord {
            id: "1".to_string(),
            name: "test".to_string(),
            kind: JobKind::Reconcile,
            namespace: None,
            state: JobState::Running,
            registered_at: 0,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            failure_count: 0,
            last_error: None,
        };
        assert!(!rec.is_terminal());
        rec.state = JobState::Succeeded;
        assert!(rec.is_terminal());
        rec.state = JobState::Failed;
        assert!(rec.is_terminal());
    }

    #[test]
    fn test_list_returns_newest_first() {
        let r = make_registry();
        let _h1 = r.register("first", JobKind::Reconcile, None);
        let _h2 = r.register("second", JobKind::Reconcile, None);
        let _h3 = r.register("third", JobKind::Reconcile, None);
        let jobs = r.list(None, None);
        assert_eq!(jobs[0].name, "third");
        assert_eq!(jobs[2].name, "first");
    }

    #[test]
    fn test_leader_election_filter() {
        let r = make_registry();
        let _h = r.register("leader-election", JobKind::LeaderElection, None);
        let jobs = r.list(None, Some("leader_election"));
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].kind, JobKind::LeaderElection);
    }

    fn meta(ns: &str, name: &str, uid: &str, created_at: u64) -> ObjectMeta {
        ObjectMeta {
            name: name.to_string(),
            namespace: ns.to_string(),
            uid: uid.to_string(),
            owner_refs: Vec::new(),
            created_at,
        }
    }

    fn job(
        ns: &str,
        name: &str,
        uid: &str,
        owner: Option<(&str, &str, &str)>,
        active: bool,
        failed: bool,
        finished_at: Option<u64>,
    ) -> JobSnapshot {
        let mut m = meta(ns, name, uid, 0);
        if let Some((kind, oname, ouid)) = owner {
            m.owner_refs.push(OwnerRef {
                kind: kind.to_string(),
                name: oname.to_string(),
                uid: ouid.to_string(),
            });
        }
        JobSnapshot {
            meta: m,
            active,
            failed,
            succeeded: false,
            finished_at,
        }
    }

    fn pod(
        ns: &str,
        name: &str,
        uid: &str,
        owner: Option<(&str, &str, &str)>,
        completed: bool,
        completed_at: Option<u64>,
    ) -> PodSnapshot {
        let mut m = meta(ns, name, uid, 0);
        if let Some((kind, oname, ouid)) = owner {
            m.owner_refs.push(OwnerRef {
                kind: kind.to_string(),
                name: oname.to_string(),
                uid: ouid.to_string(),
            });
        }
        PodSnapshot {
            meta: m,
            completed,
            completed_at,
        }
    }

    fn cronjob(ns: &str, name: &str, uid: &str, created_at: u64) -> CronJobSnapshot {
        CronJobSnapshot {
            meta: meta(ns, name, uid, created_at),
        }
    }

    #[test]
    fn test_orphan_deleted_cronjob_owner() {
        let r = OrphanReconciler::new(vec![RetentionPolicy::for_namespace("ns1")]);
        let jobs = vec![job(
            "ns1",
            "j1",
            "j1-uid",
            Some(("CronJob", "gone", "gone-uid")),
            false,
            false,
            None,
        )];
        let out = r.reconcile(1_000_000, &jobs, &[], &[]);
        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].class, OrphanClass::DeletedCronJobOwner);
        assert!(out.findings[0].will_delete);
        assert_eq!(out.report_for("ns1").unwrap().jobs_deleted, 1);
    }

    #[test]
    fn test_orphan_broken_owner_ref() {
        let r = OrphanReconciler::new(vec![RetentionPolicy::for_namespace("ns1")]);
        let cjs = vec![cronjob("ns1", "cj", "new-uid", 0)];
        let jobs = vec![job(
            "ns1",
            "j1",
            "j1-uid",
            Some(("CronJob", "cj", "old-uid")),
            false,
            false,
            None,
        )];
        let out = r.reconcile(1_000_000, &jobs, &cjs, &[]);
        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].class, OrphanClass::BrokenOwnerRef);
    }

    #[test]
    fn test_orphan_stuck_failed_job() {
        let r = OrphanReconciler::new(vec![RetentionPolicy {
            namespace: "ns1".to_string(),
            retention_secs: 60,
            enabled_classes: Vec::new(),
            dry_run: false,
        }]);
        let jobs = vec![job("ns1", "j1", "j1-uid", None, false, true, Some(1_000))];
        let out = r.reconcile(1_100, &jobs, &[], &[]);
        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].class, OrphanClass::StuckFailedJob);
    }

    #[test]
    fn test_orphan_completed_pod() {
        let r = OrphanReconciler::new(vec![RetentionPolicy {
            namespace: "ns1".to_string(),
            retention_secs: 60,
            enabled_classes: Vec::new(),
            dry_run: false,
        }]);
        let pods = vec![pod("ns1", "p1", "p1-uid", None, true, Some(1_000))];
        let out = r.reconcile(1_100, &pods, &[], &[]);
        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].class, OrphanClass::CompletedPod);
        assert_eq!(out.report_for("ns1").unwrap().pods_deleted, 1);
    }

    #[test]
    fn test_orphan_namespace_move_remnant() {
        let r = OrphanReconciler::new(vec![RetentionPolicy::for_namespace("ns1")]);
        // Owner CronJob lives in ns2 but the Job is in ns1.
        let cjs = vec![cronjob("ns2", "cj", "cj-uid", 0)];
        let jobs = vec![job(
            "ns1",
            "j1",
            "j1-uid",
            Some(("CronJob", "cj", "cj-uid")),
            false,
            false,
            None,
        )];
        let out = r.reconcile(1_000_000, &jobs, &cjs, &[]);
        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].class, OrphanClass::NamespaceMoveRemnant);
    }

    #[test]
    fn test_active_jobs_are_never_deleted() {
        let r = OrphanReconciler::new(vec![RetentionPolicy::for_namespace("ns1")]);
        let jobs = vec![job(
            "ns1",
            "j1",
            "j1-uid",
            Some(("CronJob", "gone", "gone-uid")),
            true,
            false,
            None,
        )];
        let out = r.reconcile(1_000_000, &jobs, &[], &[]);
        assert!(out.findings.is_empty());
    }

    #[test]
    fn test_dry_run_skips_deletion() {
        let r = OrphanReconciler::new(vec![RetentionPolicy {
            namespace: "ns1".to_string(),
            retention_secs: 60,
            enabled_classes: Vec::new(),
            dry_run: true,
        }]);
        let jobs = vec![job("ns1", "j1", "j1-uid", None, false, true, Some(1_000))];
        let out = r.reconcile(1_100, &jobs, &[], &[]);
        assert_eq!(out.findings.len(), 1);
        assert!(!out.findings[0].will_delete);
        assert_eq!(out.report_for("ns1").unwrap().skipped, 1);
    }

    #[test]
    fn test_disabled_class_is_skipped() {
        let r = OrphanReconciler::new(vec![RetentionPolicy {
            namespace: "ns1".to_string(),
            retention_secs: 60,
            enabled_classes: vec![OrphanClass::CompletedPod],
            dry_run: false,
        }]);
        let jobs = vec![job("ns1", "j1", "j1-uid", None, false, true, Some(1_000))];
        let out = r.reconcile(1_100, &jobs, &[], &[]);
        assert_eq!(out.findings.len(), 1);
        assert!(!out.findings[0].will_delete);
    }

    #[test]
    fn test_serialization() {
        let record = JobRecord {
            id: "42".to_string(),
            name: "archive-checker".to_string(),
            kind: JobKind::ArchiveCheck,
            namespace: Some("stellar-system".to_string()),
            state: JobState::Running,
            registered_at: 1_700_000_000,
            started_at: Some(1_700_000_001),
            finished_at: None,
            duration_ms: None,
            failure_count: 0,
            last_error: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("archive_check"));
        assert!(json.contains("running"));
        assert!(json.contains("stellar-system"));
    }
}
