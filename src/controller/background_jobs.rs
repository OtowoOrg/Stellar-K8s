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
