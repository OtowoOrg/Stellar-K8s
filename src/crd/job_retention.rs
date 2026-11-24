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
//! JobRetentionPolicy Custom Resource Definition (epic #1503).
//!
//! A namespace-scoped policy describing how long terminal `Job` and `Pod`
//! artifacts may linger and how aggressively the operator reclaims orphaned
//! ones. The policy replaces any hardcoded TTL in the codebase: the reconciler
//! in [`crate::controller::job_orphan_reconciler`] reads *only* these values
//! when deciding whether a `Job` or `Pod` is reclaimable.
//!
//! Retention windows are deliberately separate per artifact state so a failed
//! batch can be inspected quickly while a successful run is kept around for
//! post-mortem:
//!
//! - `completedJobRetentionSeconds` — how long a `Succeeded` Job is kept
//! - `failedJobRetentionSeconds` / `stuckJobGraceSeconds` — a `Failed` Job is
//!   reclaimed once it is older than `max(failedJobRetentionSeconds,
//!   stuckJobGraceSeconds)`; the grace period guarantees a Job that has just
//!   exhausted its backoff is not deleted before the owner can react
//! - `podRetentionSeconds` — how long a terminal (`Succeeded`/`Failed`) Pod is
//!   kept once its owning Job is terminal
//!
//! # Example
//!
//! ```yaml
//! apiVersion: stellar.org/v1alpha1
//! kind: JobRetentionPolicy
//! metadata:
//!   name: job-retention
//!   namespace: stellar
//! spec:
//!   completedJobRetentionSeconds: 3600
//!   failedJobRetentionSeconds: 900
//!   stuckJobGraceSeconds: 3600
//!   podRetentionSeconds: 1800
//!   reconcileOwnerReferences: true
//! ```

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

/// Default retention for terminal Pods (30 minutes).
pub const DEFAULT_POD_RETENTION_SECONDS: u32 = 1_800;
/// Default retention for failed Jobs (15 minutes).
pub const DEFAULT_FAILED_JOB_RETENTION_SECONDS: u32 = 900;
/// Default retention for succeeded Jobs (1 hour).
pub const DEFAULT_COMPLETED_JOB_RETENTION_SECONDS: u32 = 3_600;
/// Default extra grace before a failed Job is treated as stuck (1 hour).
pub const DEFAULT_STUCK_JOB_GRACE_SECONDS: u32 = 3_600;
/// Default requeue interval of the orphan sweep (5 minutes).
pub const DEFAULT_RECONCILE_INTERVAL_SECONDS: u32 = 300;
/// Default deletion grace period applied to reclaimed artifacts.
pub const DEFAULT_GRACE_PERIOD_SECONDS: i64 = 30;
/// Default label key used to scope the sweep to operator-managed artifacts.
pub const DEFAULT_WORKLOAD_LABEL: &str = "app.kubernetes.io/managed-by";
/// Default label value used to scope the sweep to operator-managed artifacts.
pub const DEFAULT_WORKLOAD_LABEL_VALUE: &str = "stellar-operator";

fn default_pod_retention_seconds() -> u32 {
    DEFAULT_POD_RETENTION_SECONDS
}
fn default_failed_job_retention_seconds() -> u32 {
    DEFAULT_FAILED_JOB_RETENTION_SECONDS
}
fn default_completed_job_retention_seconds() -> u32 {
    DEFAULT_COMPLETED_JOB_RETENTION_SECONDS
}
fn default_stuck_job_grace_seconds() -> u32 {
    DEFAULT_STUCK_JOB_GRACE_SECONDS
}
fn default_reconcile_interval_seconds() -> u32 {
    DEFAULT_RECONCILE_INTERVAL_SECONDS
}
fn default_grace_period_seconds() -> i64 {
    DEFAULT_GRACE_PERIOD_SECONDS
}
fn default_workload_label() -> String {
    DEFAULT_WORKLOAD_LABEL.to_string()
}
fn default_workload_label_value() -> String {
    DEFAULT_WORKLOAD_LABEL_VALUE.to_string()
}
fn default_true() -> bool {
    true
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "JobRetentionPolicy",
    status = "JobRetentionPolicyStatus",
    shortname = "jrp",
    printcolumn = r#"{"name":"Pods","type":"integer","jsonPath":".status.reclaimedPods"}"#,
    printcolumn = r#"{"name":"Jobs","type":"integer","jsonPath":".status.reclaimedJobs"}"#,
    printcolumn = r#"{"name":"Repaired","type":"integer","jsonPath":".status.repairedOwnerReferences"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct JobRetentionPolicySpec {
    /// How long a `Succeeded` Job is kept before it is reclaimed.
    #[serde(default = "default_completed_job_retention_seconds")]
    pub completed_job_retention_seconds: u32,
    /// Minimum retention of a `Failed` Job before it is reclaimed.
    #[serde(default = "default_failed_job_retention_seconds")]
    pub failed_job_retention_seconds: u32,
    /// Extra grace for a `Failed` Job; effective retention is
    /// `max(failedJobRetentionSeconds, stuckJobGraceSeconds)`.
    #[serde(default = "default_stuck_job_grace_seconds")]
    pub stuck_job_grace_seconds: u32,
    /// How long a terminal (`Succeeded`/`Failed`) Pod is kept after its owning
    /// Job reached a terminal state.
    #[serde(default = "default_pod_retention_seconds")]
    pub pod_retention_seconds: u32,
    /// Whether Jobs whose owning CronJob no longer exists are deleted.
    #[serde(default = "default_true")]
    pub delete_orphaned_jobs: bool,
    /// Whether terminal Pods past [`Self::pod_retention_seconds`] are deleted.
    #[serde(default = "default_true")]
    pub delete_orphaned_pods: bool,
    /// Whether `ownerReferences` left dangling by a partial deletion are
    /// re-pointed at the live owner instead of only reported.
    #[serde(default = "default_true")]
    pub reconcile_owner_references: bool,
    /// Whether Pods carrying the moved-namespace annotation are reclaimed even
    /// when the owning CronJob was deleted together with its namespace.
    #[serde(default = "default_true")]
    pub reclaim_after_namespace_move: bool,
    /// Label key an artifact must carry to be in scope of the sweep.
    #[serde(default = "default_workload_label")]
    pub workload_label: String,
    /// Label value an artifact must carry to be in scope of the sweep.
    #[serde(default = "default_workload_label_value")]
    pub workload_label_value: String,
    /// Annotation written by controllers recording the namespace an artifact
    /// was moved from; used to detect namespace-move orphans.
    #[serde(default = "default_previous_namespace_annotation")]
    pub previous_namespace_annotation: String,
    /// `DeleteOptions` grace period applied to reclaimed artifacts.
    #[serde(default = "default_grace_period_seconds")]
    pub grace_period_seconds: i64,
    /// How often the sweep should be requeued.
    #[serde(default = "default_reconcile_interval_seconds")]
    pub reconcile_interval_seconds: u32,
    /// Plan and report only; never issue a delete or a patch.
    #[serde(default)]
    pub dry_run: bool,
}

fn default_previous_namespace_annotation() -> String {
    PREVIOUS_NAMESPACE_ANNOTATION.to_string()
}

/// Annotation recording the namespace an artifact was moved from.
pub const PREVIOUS_NAMESPACE_ANNOTATION: &str = "stellar.org/previous-namespace";

impl Default for JobRetentionPolicySpec {
    fn default() -> Self {
        Self {
            completed_job_retention_seconds: default_completed_job_retention_seconds(),
            failed_job_retention_seconds: default_failed_job_retention_seconds(),
            stuck_job_grace_seconds: default_stuck_job_grace_seconds(),
            pod_retention_seconds: default_pod_retention_seconds(),
            delete_orphaned_jobs: true,
            delete_orphaned_pods: true,
            reconcile_owner_references: true,
            reclaim_after_namespace_move: true,
            workload_label: default_workload_label(),
            workload_label_value: default_workload_label_value(),
            previous_namespace_annotation: default_previous_namespace_annotation(),
            grace_period_seconds: default_grace_period_seconds(),
            reconcile_interval_seconds: default_reconcile_interval_seconds(),
            dry_run: false,
        }
    }
}

impl JobRetentionPolicySpec {
    /// Effective retention of a `Failed` Job.
    ///
    /// The stuck-job grace period is a floor: a Job that has just exhausted its
    /// backoff is never reclaimed before `stuck_job_grace_seconds` has passed,
    /// even when `failed_job_retention_seconds` is configured lower.
    pub fn effective_failed_job_retention_seconds(&self) -> u32 {
        self.failed_job_retention_seconds
            .max(self.stuck_job_grace_seconds)
    }

    /// Requeue delay after a completed sweep.
    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.reconcile_interval_seconds))
    }

    /// Is `labels` in scope of the sweep?
    ///
    /// Artifacts without a controller-owner chain are only reclaimed when they
    /// carry the configured workload label, so hand-written Jobs and Pods are
    /// never touched.
    pub fn in_scope(&self, labels: &std::collections::BTreeMap<String, String>) -> bool {
        labels.get(&self.workload_label) == Some(&self.workload_label_value)
    }
}

/// Per-class tally of artifacts reclaimed in the last successful sweep.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReclaimedByClass {
    /// Job whose owning CronJob no longer exists.
    pub deleted_cron_job: u32,
    /// `ownerReference` left dangling by a partial deletion.
    pub broken_owner_reference: u32,
    /// `Failed` Job older than the retention window.
    pub stuck_failed_job: u32,
    /// Terminal Pod past the pod retention window.
    pub completed_pod: u32,
    /// Remnant of an artifact moved out of its original namespace.
    pub namespace_move: u32,
    /// Artifact that carried no controller owner at all.
    pub deleted_owner_remnant: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRetentionPolicyStatus {
    /// Number of Jobs reclaimed in the last sweep.
    #[serde(default)]
    pub reclaimed_jobs: u32,
    /// Number of Pods reclaimed in the last sweep.
    #[serde(default)]
    pub reclaimed_pods: u32,
    /// Number of `ownerReferences` repaired in the last sweep.
    #[serde(default)]
    pub repaired_owner_references: u32,
    /// Per-class breakdown of the last sweep.
    #[serde(default)]
    pub reclaimed_by_class: ReclaimedByClass,
    /// Artifacts still in the plan but held back (too young, or out of scope).
    #[serde(default)]
    pub retained: u32,
    /// Whether the last sweep was a dry run.
    #[serde(default)]
    pub dry_run: bool,
    /// RFC 3339 timestamp of the last successful sweep.
    #[serde(default)]
    pub last_reconciled: Option<DateTime<Utc>>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn default_spec_matches_documented_windows() {
        let s = JobRetentionPolicySpec::default();
        assert_eq!(s.pod_retention_seconds, 1_800);
        assert_eq!(s.failed_job_retention_seconds, 900);
        assert_eq!(s.completed_job_retention_seconds, 3_600);
        assert_eq!(s.reconcile_interval_seconds, 300);
        assert!(s.delete_orphaned_jobs);
        assert!(s.delete_orphaned_pods);
        assert!(s.reconcile_owner_references);
        assert!(!s.dry_run);
    }

    #[test]
    fn spec_roundtrips_through_yaml_with_camel_case_keys() {
        let spec = JobRetentionPolicySpec {
            pod_retention_seconds: 60,
            dry_run: true,
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&spec).expect("serialize spec");
        assert!(yaml.contains("podRetentionSeconds: 60"), "{yaml}");
        assert!(yaml.contains("dryRun: true"), "{yaml}");
        let back: JobRetentionPolicySpec = serde_yaml::from_str(&yaml).expect("deserialize spec");
        assert_eq!(back.pod_retention_seconds, 60);
        assert!(back.dry_run);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let spec: JobRetentionPolicySpec = serde_yaml::from_str("{}").expect("empty spec");
        assert_eq!(spec, JobRetentionPolicySpec::default());
    }

    #[test]
    fn stuck_job_grace_is_a_floor_on_failed_retention() {
        let mut s = JobRetentionPolicySpec::default();
        s.failed_job_retention_seconds = 60;
        s.stuck_job_grace_seconds = 3_600;
        assert_eq!(s.effective_failed_job_retention_seconds(), 3_600);

        s.stuck_job_grace_seconds = 30;
        assert_eq!(s.effective_failed_job_retention_seconds(), 60);
    }

    #[test]
    fn scope_requires_the_configured_workload_label() {
        let s = JobRetentionPolicySpec::default();
        let mut labels = BTreeMap::new();
        assert!(!s.in_scope(&labels));
        labels.insert(
            DEFAULT_WORKLOAD_LABEL.to_string(),
            DEFAULT_WORKLOAD_LABEL_VALUE.to_string(),
        );
        assert!(s.in_scope(&labels));
        labels.insert(DEFAULT_WORKLOAD_LABEL.to_string(), "other".to_string());
        assert!(!s.in_scope(&labels));
    }

    #[test]
    fn interval_derives_requeue_delay() {
        let mut s = JobRetentionPolicySpec::default();
        s.reconcile_interval_seconds = 42;
        assert_eq!(s.interval(), std::time::Duration::from_secs(42));
    }
}
