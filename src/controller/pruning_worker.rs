//! History Archive Pruning Worker
//!
//! Manages automated and manual pruning of Stellar history archives based on retention policies.
//! Provides dry-run mode, safety locks, and cloud-native bucket lifecycle integration.

use chrono::{DateTime, Utc};
use cron::Schedule;
use std::str::FromStr;
use tracing::warn;

use crate::crd::types::{PruningPolicy, PruningStatus};
use crate::Error;

/// Pruning worker for managing history archive retention
pub struct PruningWorker {
    policy: PruningPolicy,
}

impl PruningWorker {
    /// Create a new pruning worker with the given policy
    pub fn new(policy: PruningPolicy) -> Result<Self, Error> {
        policy.validate().map_err(Error::ConfigError)?;
        Ok(Self { policy })
    }

    /// Check if pruning should run based on schedule
    pub fn should_run_scheduled(&self, last_run: Option<DateTime<Utc>>) -> bool {
        if !self.policy.enabled {
            return false;
        }

        let Some(schedule) = &self.policy.schedule else {
            return false;
        };

        let Ok(cron) = Schedule::from_str(schedule) else {
            warn!("Invalid cron expression: {}", schedule);
            return false;
        };

        let now = Utc::now();
        match last_run {
            None => true,
            Some(last) => {
                // Check if next scheduled time has passed
                if let Some(next) = cron.after(&last).next() {
                    next <= now
                } else {
                    false
                }
            }
        }
    }

    /// Validate that a checkpoint is safe to delete
    pub fn is_checkpoint_safe_to_delete(
        &self,
        checkpoint_age_days: u32,
        checkpoint_count_from_latest: u32,
    ) -> bool {
        // Never delete if within minimum checkpoint buffer
        if checkpoint_count_from_latest < self.policy.min_checkpoints {
            return false;
        }

        // Never delete if too recent (safety lock)
        if checkpoint_age_days < self.policy.max_age_days {
            return false;
        }

        true
    }

    /// Determine if a checkpoint meets retention criteria
    pub fn meets_retention_criteria(
        &self,
        checkpoint_age_days: u32,
        checkpoint_ledger: u32,
        latest_ledger: u32,
    ) -> bool {
        match (self.policy.retention_days, self.policy.retention_ledgers) {
            (Some(days), _) => checkpoint_age_days > days,
            (_, Some(ledgers)) => {
                let ledger_distance = latest_ledger.saturating_sub(checkpoint_ledger);
                ledger_distance > ledgers
            }
            _ => false,
        }
    }

    /// Get the policy configuration
    pub fn policy(&self) -> &PruningPolicy {
        &self.policy
    }

    /// Check if auto-delete is enabled
    pub fn auto_delete_enabled(&self) -> bool {
        self.policy.auto_delete
    }

    /// Check if confirmation should be skipped
    pub fn skip_confirmation(&self) -> bool {
        self.policy.skip_confirmation
    }
}

/// Result of a pruning analysis
#[derive(Clone, Debug)]
pub struct PruningAnalysis {
    pub total_checkpoints: u32,
    pub eligible_for_deletion: u32,
    pub will_be_retained: u32,
    pub bytes_to_free: u64,
    pub dry_run: bool,
}

impl PruningAnalysis {
    /// Convert analysis to status
    pub fn to_status(&self, deleted_count: u32, message: String) -> PruningStatus {
        PruningStatus {
            last_run_time: Some(Utc::now().to_rfc3339()),
            last_run_status: Some("Success".to_string()),
            total_checkpoints: Some(self.total_checkpoints),
            deleted_count: Some(deleted_count),
            retained_count: Some(self.will_be_retained),
            bytes_freed: Some(self.bytes_to_free),
            message: Some(message),
            dry_run: Some(self.dry_run),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pruning_worker_creation() {
        let policy = PruningPolicy {
            enabled: true,
            retention_days: Some(30),
            retention_ledgers: None,
            min_checkpoints: 50,
            max_age_days: 7,
            concurrency: 10,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        let worker = PruningWorker::new(policy).unwrap();
        assert!(worker.policy.enabled);
        assert_eq!(worker.policy.retention_days, Some(30));
    }

    #[test]
    fn test_checkpoint_safety_validation() {
        let policy = PruningPolicy {
            enabled: true,
            retention_days: Some(30),
            retention_ledgers: None,
            min_checkpoints: 50,
            max_age_days: 7,
            concurrency: 10,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        let worker = PruningWorker::new(policy).unwrap();

        // Too recent - should not delete
        assert!(!worker.is_checkpoint_safe_to_delete(5, 100));

        // Within minimum buffer - should not delete
        assert!(!worker.is_checkpoint_safe_to_delete(10, 30));

        // Safe to delete
        assert!(worker.is_checkpoint_safe_to_delete(10, 100));
    }

    #[test]
    fn test_retention_criteria_time_based() {
        let policy = PruningPolicy {
            enabled: true,
            retention_days: Some(30),
            retention_ledgers: None,
            min_checkpoints: 50,
            max_age_days: 7,
            concurrency: 10,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        let worker = PruningWorker::new(policy).unwrap();

        // Older than retention - meets criteria
        assert!(worker.meets_retention_criteria(40, 1000, 2000));

        // Newer than retention - does not meet criteria
        assert!(!worker.meets_retention_criteria(20, 1000, 2000));
    }

    #[test]
    fn test_retention_criteria_ledger_based() {
        let policy = PruningPolicy {
            enabled: true,
            retention_days: None,
            retention_ledgers: Some(100000),
            min_checkpoints: 50,
            max_age_days: 7,
            concurrency: 10,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        let worker = PruningWorker::new(policy).unwrap();

        // Older than retention - meets criteria
        assert!(worker.meets_retention_criteria(0, 1000000, 2000000));

        // Newer than retention - does not meet criteria
        assert!(!worker.meets_retention_criteria(0, 1900000, 2000000));
    }

    #[test]
    fn test_invalid_policy_validation() {
        // Both retention policies specified
        let policy = PruningPolicy {
            enabled: true,
            retention_days: Some(30),
            retention_ledgers: Some(100000),
            min_checkpoints: 50,
            max_age_days: 7,
            concurrency: 10,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        assert!(PruningWorker::new(policy).is_err());

        // Min checkpoints too low
        let policy = PruningPolicy {
            enabled: true,
            retention_days: Some(30),
            retention_ledgers: None,
            min_checkpoints: 5,
            max_age_days: 7,
            concurrency: 10,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        assert!(PruningWorker::new(policy).is_err());

        // Concurrency is 0
        let policy = PruningPolicy {
            enabled: true,
            retention_days: Some(30),
            retention_ledgers: None,
            min_checkpoints: 50,
            max_age_days: 7,
            concurrency: 0,
            schedule: None,
            auto_delete: false,
            skip_confirmation: false,
        };

        assert!(PruningWorker::new(policy).is_err());
    }
}
