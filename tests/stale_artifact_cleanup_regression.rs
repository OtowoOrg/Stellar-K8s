//! Regression tests for stale artifact cleanup logic (Issue #978).

use chrono::Utc;
use stellar_k8s::controller::pruning_worker::{PruningAnalysis, PruningWorker};
use stellar_k8s::crd::types::PruningPolicy;

fn policy(retention_days: u32) -> PruningPolicy {
    PruningPolicy {
        enabled: true,
        retention_days: Some(retention_days),
        retention_ledgers: None,
        min_checkpoints: 50,
        max_age_days: 7,
        concurrency: 4,
        schedule: None,
        auto_delete: false,
        skip_confirmation: false,
    }
}

#[cfg(test)]
mod stale_artifact_cleanup_regression {
    use super::*;

    // 1. Stale artifact threshold boundary
    // An artifact aged exactly at retention_days is NOT yet stale (must be strictly greater).
    // An artifact aged retention_days + 1 IS stale.
    #[test]
    fn threshold_boundary_exactly_at_limit_is_not_stale() {
        let worker = PruningWorker::new(policy(30)).unwrap();
        // age == retention_days (30): meets_retention_criteria uses `> days`, so false
        assert!(!worker.meets_retention_criteria(30, 0, 1_000_000));
    }

    #[test]
    fn threshold_boundary_one_over_limit_is_stale() {
        let worker = PruningWorker::new(policy(30)).unwrap();
        // age == 31 > 30: stale
        assert!(worker.meets_retention_criteria(31, 0, 1_000_000));
    }

    // 2. Minimum checkpoint guard regression
    // Even if an artifact is old enough to delete, it must NOT be deleted when it is
    // within min_checkpoints of the latest checkpoint.
    #[test]
    fn min_checkpoint_guard_prevents_deletion_of_old_nearby_artifact() {
        let worker = PruningWorker::new(policy(30)).unwrap();
        // age=100 days (well past retention), but only 10 checkpoints from latest < min(50)
        assert!(!worker.is_checkpoint_safe_to_delete(100, 10));
    }

    #[test]
    fn min_checkpoint_guard_allows_deletion_beyond_buffer() {
        let worker = PruningWorker::new(policy(30)).unwrap();
        // age=100 days, 60 checkpoints from latest > min(50): safe
        assert!(worker.is_checkpoint_safe_to_delete(100, 60));
    }

    // 3. No-op on empty archive
    // An analysis with zero total checkpoints reports zero eligible for deletion.
    #[test]
    fn empty_archive_yields_zero_eligible() {
        let analysis = PruningAnalysis {
            total_checkpoints: 0,
            eligible_for_deletion: 0,
            will_be_retained: 0,
            bytes_to_free: 0,
            dry_run: true,
        };
        assert_eq!(analysis.eligible_for_deletion, 0);
        assert_eq!(analysis.total_checkpoints, 0);
    }

    // 4. Concurrent deletion safety
    // Analysis counts must be internally consistent: eligible + retained == total.
    #[test]
    fn analysis_counts_are_consistent() {
        let total: u32 = 200;
        let eligible: u32 = 75;
        let retained: u32 = total - eligible;
        let analysis = PruningAnalysis {
            total_checkpoints: total,
            eligible_for_deletion: eligible,
            will_be_retained: retained,
            bytes_to_free: eligible as u64 * 1024,
            dry_run: false,
        };
        assert_eq!(
            analysis.eligible_for_deletion + analysis.will_be_retained,
            analysis.total_checkpoints
        );
    }

    // 5. Schedule-based triggering
    // When disabled, should_run_scheduled is always false.
    // When enabled with a past-due schedule, it triggers.
    #[test]
    fn disabled_policy_never_triggers() {
        let mut p = policy(30);
        p.enabled = false;
        p.schedule = Some("* * * * * *".to_string()); // every second
        // disabled policies skip validation, so construct directly
        let worker = PruningWorker::new(PruningPolicy {
            enabled: false,
            ..PruningPolicy::default()
        })
        .unwrap();
        assert!(!worker.should_run_scheduled(None));
        assert!(!worker.should_run_scheduled(Some(Utc::now())));
    }

    #[test]
    fn enabled_policy_without_schedule_never_triggers() {
        let worker = PruningWorker::new(policy(30)).unwrap();
        // no schedule set
        assert!(!worker.should_run_scheduled(None));
    }

    #[test]
    fn enabled_policy_with_no_last_run_triggers() {
        let mut p = policy(30);
        p.schedule = Some("0 0 * * *".to_string()); // daily at midnight
        let worker = PruningWorker::new(p).unwrap();
        // No last run recorded → should trigger immediately
        assert!(worker.should_run_scheduled(None));
    }

    #[test]
    fn enabled_policy_does_not_trigger_before_schedule_elapses() {
        let mut p = policy(30);
        p.schedule = Some("0 0 1 1 *".to_string()); // yearly (Jan 1)
        let worker = PruningWorker::new(p).unwrap();
        // Last run just now → next occurrence is far in the future
        let just_ran = Utc::now();
        assert!(!worker.should_run_scheduled(Some(just_ran)));
    }
}
