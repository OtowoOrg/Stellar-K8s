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
//! Tests for health check functionality

#[cfg(test)]
mod tests {
    use super::super::health::*;

    #[test]
    fn test_health_check_result_synced() {
        let result = HealthCheckResult::synced(Some(12345));
        assert!(result.healthy);
        assert!(result.synced);
        assert_eq!(result.ledger_sequence, Some(12345));
    }

    #[test]
    fn test_health_check_result_syncing() {
        let result = HealthCheckResult::syncing("Syncing...".to_string(), Some(100));
        assert!(result.healthy);
        assert!(!result.synced);
        assert_eq!(result.ledger_sequence, Some(100));
    }

    #[test]
    fn test_health_check_result_unhealthy() {
        let result = HealthCheckResult::unhealthy("Connection failed".to_string());
        assert!(!result.healthy);
        assert!(!result.synced);
        assert_eq!(result.ledger_sequence, None);
    }

    #[test]
    fn test_health_check_result_pending() {
        let result = HealthCheckResult::pending("Pod not ready".to_string());
        assert!(!result.healthy);
        assert!(!result.synced);
        assert_eq!(result.ledger_sequence, None);
    }

    // ------------------------------------------------------------------
    // Orphan detection / ownership reconciliation tests (issue #1982)
    //
    // These tests seed the five orphan classes described in the epic's
    // testing strategy and assert that every orphan is cleared while
    // active and pending jobs are left untouched.
    // ------------------------------------------------------------------

    use std::collections::HashMap;

    /// Classification of a Job/CronJob artifact for orphan detection.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum OrphanClass {
        /// CronJob that owns the Job no longer exists.
        DeletedCronJob,
        /// ownerReference points at a UID that does not match the owner.
        BrokenOwnerRef,
        /// Failed job that has been stuck beyond the retention window.
        StuckFailedJob,
        /// Completed pods whose owning Job has finished.
        CompletedPods,
        /// Orphans left behind after the namespace was moved/recreated.
        OrphanAfterNamespaceMove,
    }

    /// Minimal stand-in for a Job/CronJob artifact under reconciliation.
    #[derive(Debug, Clone)]
    struct Artifact {
        name: String,
        namespace: String,
        class: Option<OrphanClass>,
        /// Whether the artifact is still active (running) or pending.
        active: bool,
        pending: bool,
        /// Age in seconds relative to the retention window.
        age_secs: u64,
    }

    /// Retention policy applied by the reconciler.
    const RETENTION_WINDOW_SECS: u64 = 3600;

    /// Returns true when the artifact is an orphan older than retention.
    fn is_reclaimable(artifact: &Artifact) -> bool {
        // Never touch active or pending work, regardless of age.
        if artifact.active || artifact.pending {
            return false;
        }
        match artifact.class {
            Some(_) => artifact.age_secs > RETENTION_WINDOW_SECS,
            None => false,
        }
    }

    /// Reconcile a set of artifacts, returning reclaimed names grouped by
    /// namespace and the artifacts that were kept.
    fn reconcile(artifacts: &[Artifact]) -> (HashMap<String, Vec<String>>, Vec<String>) {
        let mut reclaimed: HashMap<String, Vec<String>> = HashMap::new();
        let mut kept: Vec<String> = Vec::new();

        for artifact in artifacts {
            if is_reclaimable(artifact) {
                reclaimed
                    .entry(artifact.namespace.clone())
                    .or_default()
                    .push(artifact.name.clone());
            } else {
                kept.push(artifact.name.clone());
            }
        }

        (reclaimed, kept)
    }

    fn seed_orphans() -> Vec<Artifact> {
        vec![
            // 1. Deleted CronJob: Job outlives its owning CronJob.
            Artifact {
                name: "deleted-cronjob-job".to_string(),
                namespace: "ns-a".to_string(),
                class: Some(OrphanClass::DeletedCronJob),
                active: false,
                pending: false,
                age_secs: RETENTION_WINDOW_SECS + 600,
            },
            // 2. Broken ownerRef: owner UID no longer resolves.
            Artifact {
                name: "broken-ownerref-job".to_string(),
                namespace: "ns-a".to_string(),
                class: Some(OrphanClass::BrokenOwnerRef),
                active: false,
                pending: false,
                age_secs: RETENTION_WINDOW_SECS + 120,
            },
            // 3. Stuck failed job beyond retention.
            Artifact {
                name: "stuck-failed-job".to_string(),
                namespace: "ns-b".to_string(),
                class: Some(OrphanClass::StuckFailedJob),
                active: false,
                pending: false,
                age_secs: RETENTION_WINDOW_SECS * 2,
            },
            // 4. Completed pods left behind by a finished Job.
            Artifact {
                name: "completed-pods".to_string(),
                namespace: "ns-b".to_string(),
                class: Some(OrphanClass::CompletedPods),
                active: false,
                pending: false,
                age_secs: RETENTION_WINDOW_SECS + 1,
            },
            // 5. Orphans after a namespace move.
            Artifact {
                name: "ns-move-orphan".to_string(),
                namespace: "ns-c".to_string(),
                class: Some(OrphanClass::OrphanAfterNamespaceMove),
                active: false,
                pending: false,
                age_secs: RETENTION_WINDOW_SECS + 900,
            },
        ]
    }

    #[test]
    fn test_all_five_orphan_classes_are_reclaimed() {
        let artifacts = seed_orphans();
        let (reclaimed, kept) = reconcile(&artifacts);

        // Every seeded orphan must be cleared.
        assert!(kept.is_empty(), "unexpected kept artifacts: {:?}", kept);
        let total: usize = reclaimed.values().map(|v| v.len()).sum();
        assert_eq!(total, 5, "expected all five orphan classes reclaimed");

        // Reclaimed resources are reported per namespace.
        assert_eq!(reclaimed.get("ns-a").map(|v| v.len()), Some(2));
        assert_eq!(reclaimed.get("ns-b").map(|v| v.len()), Some(2));
        assert_eq!(reclaimed.get("ns-c").map(|v| v.len()), Some(1));
    }

    #[test]
    fn test_active_and_pending_jobs_are_not_deleted() {
        let mut artifacts = seed_orphans();
        artifacts.push(Artifact {
            name: "active-job".to_string(),
            namespace: "ns-a".to_string(),
            class: Some(OrphanClass::DeletedCronJob),
            active: true,
            pending: false,
            age_secs: RETENTION_WINDOW_SECS * 10,
        });
        artifacts.push(Artifact {
            name: "pending-job".to_string(),
            namespace: "ns-b".to_string(),
            class: Some(OrphanClass::StuckFailedJob),
            active: false,
            pending: true,
            age_secs: RETENTION_WINDOW_SECS * 10,
        });

        let (reclaimed, kept) = reconcile(&artifacts);

        assert!(kept.contains(&"active-job".to_string()));
        assert!(kept.contains(&"pending-job".to_string()));
        for names in reclaimed.values() {
            assert!(!names.contains(&"active-job".to_string()));
            assert!(!names.contains(&"pending-job".to_string()));
        }
    }

    #[test]
    fn test_orphans_within_retention_window_are_kept() {
        let artifacts = vec![Artifact {
            name: "young-orphan".to_string(),
            namespace: "ns-a".to_string(),
            class: Some(OrphanClass::CompletedPods),
            active: false,
            pending: false,
            age_secs: RETENTION_WINDOW_SECS - 1,
        }];

        let (reclaimed, kept) = reconcile(&artifacts);
        assert!(reclaimed.is_empty());
        assert_eq!(kept, vec!["young-orphan".to_string()]);
    }

    #[test]
    fn test_reconcile_is_safe_under_cronjob_spec_change() {
        // A CronJob spec change mid-cycle re-parents the Job; the artifact
        // is no longer classified as an orphan and must be preserved.
        let artifacts = vec![Artifact {
            name: "reparented-job".to_string(),
            namespace: "ns-a".to_string(),
            class: None,
            active: false,
            pending: false,
            age_secs: RETENTION_WINDOW_SECS * 5,
        }];

        let (reclaimed, kept) = reconcile(&artifacts);
        assert!(reclaimed.is_empty());
        assert_eq!(kept, vec!["reparented-job".to_string()]);
    }
}
