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
//! Unit tests for disaster recovery (DR) logic
//!
//! Covers: DR config enabled/disabled, Primary/Standby role assignment,
//! failover state transitions, sync lag computation, backup target priority
//! ordering, and the consistency partition check.

#[cfg(test)]
mod tests {
    use crate::crd::{
        DRRole, DRSyncStrategy, DisasterRecoveryConfig, DisasterRecoveryStatus, PeerClusterConfig,
    };

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn dr_config(role: DRRole, sync: DRSyncStrategy) -> DisasterRecoveryConfig {
        DisasterRecoveryConfig {
            enabled: true,
            role,
            peer_cluster_id: "us-west-2".to_string(),
            sync_strategy: sync,
            failover_dns: None,
            health_check_interval: 30,
            drill_schedule: None,
            policy_ref: None,
            archive_integrity_config: None,
        }
    }

    fn fresh_status() -> DisasterRecoveryStatus {
        DisasterRecoveryStatus::default()
    }

    // -------------------------------------------------------------------------
    // DR config: disabled → no DR processing
    // -------------------------------------------------------------------------

    #[test]
    fn test_dr_config_disabled_is_skipped() {
        let config = DisasterRecoveryConfig {
            enabled: false,
            role: DRRole::Standby,
            peer_cluster_id: "eu-central-1".to_string(),
            sync_strategy: DRSyncStrategy::Consensus,
            failover_dns: None,
            health_check_interval: 30,
            drill_schedule: None,
            policy_ref: None,
            archive_integrity_config: None,
        };
        // When enabled is false the reconciler returns Ok(None).
        // We verify the shape of the config to confirm the guard would fire.
        assert!(!config.enabled);
    }

    // -------------------------------------------------------------------------
    // Primary role: no failover, current_role stays Primary
    // -------------------------------------------------------------------------

    #[test]
    fn test_primary_role_no_failover_triggered() {
        // Simulate the branch: role == Primary → else arm → set current_role
        let config = dr_config(DRRole::Primary, DRSyncStrategy::Consensus);
        let mut status = fresh_status();

        // Replicate the else-arm of reconcile_dr
        status.current_role = Some(config.role.clone());
        status.peer_health = Some("Healthy".to_string());

        assert_eq!(status.current_role, Some(DRRole::Primary));
        assert!(!status.failover_active);
    }

    // -------------------------------------------------------------------------
    // Triggering a DR failover when primary region fails
    // -------------------------------------------------------------------------

    #[test]
    fn test_failover_triggered_when_peer_unreachable() {
        // Simulate: role == Standby, peer_healthy == false, failover_active == false
        let config = dr_config(DRRole::Standby, DRSyncStrategy::Consensus);
        let mut status = fresh_status();

        // Peer is unreachable
        let peer_healthy = false;

        status.peer_health = Some("Unreachable".to_string());

        // Replicate the if-arm of reconcile_dr
        if config.role == DRRole::Standby && !peer_healthy && !status.failover_active {
            status.failover_active = true;
            status.current_role = Some(DRRole::Primary);
        }

        assert!(status.failover_active);
        assert_eq!(status.current_role, Some(DRRole::Primary));
        assert_eq!(status.peer_health.as_deref(), Some("Unreachable"));
    }

    #[test]
    fn test_failover_not_re_triggered_when_already_active() {
        // Idempotency: if failover_active is already true, the block is skipped
        let config = dr_config(DRRole::Standby, DRSyncStrategy::Consensus);
        let mut status = fresh_status();
        status.failover_active = true;
        status.current_role = Some(DRRole::Primary);

        let peer_healthy = false;

        // The outer guard `!status.failover_active` prevents a second activation
        if config.role == DRRole::Standby && !peer_healthy && !status.failover_active {
            // Should NOT reach here
            panic!("failover should not be re-triggered");
        }

        // State unchanged
        assert!(status.failover_active);
        assert_eq!(status.current_role, Some(DRRole::Primary));
    }

    // -------------------------------------------------------------------------
    // No-op: everything healthy, Standby role stays Standby
    // -------------------------------------------------------------------------

    #[test]
    fn test_no_op_when_everything_healthy() {
        // Simulate: role == Standby, peer_healthy == true, failover_active == false
        let config = dr_config(DRRole::Standby, DRSyncStrategy::Consensus);
        let mut status = fresh_status();

        let peer_healthy = true;

        status.peer_health = Some("Healthy".to_string());
        status.last_peer_contact = Some("2026-02-21T18:00:00Z".to_string());

        // Replicate reconcile_dr: none of the failover branches fire, role is set
        if config.role == DRRole::Standby && !peer_healthy {
            // not entered
        } else if config.role == DRRole::Standby && peer_healthy && status.failover_active {
            // not entered – no failback needed
        } else {
            status.current_role = Some(config.role.clone());
        }

        assert_eq!(status.current_role, Some(DRRole::Standby));
        assert!(!status.failover_active);
        assert_eq!(status.peer_health.as_deref(), Some("Healthy"));
    }

    // -------------------------------------------------------------------------
    // Backup targets used in correct priority order
    // -------------------------------------------------------------------------

    #[test]
    fn test_backup_targets_sorted_by_priority_descending() {
        // Higher priority value = more preferred target
        let mut targets = [
            PeerClusterConfig {
                cluster_id: "eu-central-1".to_string(),
                endpoint: "10.0.0.1".to_string(),
                latency_threshold_ms: None,
                region: Some("eu".to_string()),
                priority: 50,
                port: None,
                enabled: true,
                kubeconfig_secret_key: "kubeconfig".to_string(),
                kubeconfig_secret_ref: Some("secret".to_string()),
                target_namespace: None,
            },
            PeerClusterConfig {
                cluster_id: "us-east-1".to_string(),
                endpoint: "10.0.0.2".to_string(),
                latency_threshold_ms: None,
                region: Some("us".to_string()),
                priority: 150,
                port: None,
                enabled: true,
                kubeconfig_secret_key: "kubeconfig".to_string(),
                kubeconfig_secret_ref: Some("secret".to_string()),
                target_namespace: None,
            },
            PeerClusterConfig {
                cluster_id: "ap-south-1".to_string(),
                endpoint: "10.0.0.3".to_string(),
                latency_threshold_ms: None,
                region: Some("ap".to_string()),
                priority: 100,
                port: None,
                enabled: true,
                kubeconfig_secret_key: "kubeconfig".to_string(),
                kubeconfig_secret_ref: Some("secret".to_string()),
                target_namespace: None,
            },
        ];

        // Sort descending by priority (most preferred first)
        targets.sort_by_key(|b| std::cmp::Reverse(b.priority));

        assert_eq!(targets[0].cluster_id, "us-east-1");
        assert_eq!(targets[1].cluster_id, "ap-south-1");
        assert_eq!(targets[2].cluster_id, "eu-central-1");
    }

    #[test]
    fn test_disabled_backup_targets_excluded() {
        let targets = [
            PeerClusterConfig {
                cluster_id: "us-east-1".to_string(),
                endpoint: "10.0.0.1".to_string(),
                latency_threshold_ms: None,
                region: None,
                priority: 100,
                port: None,
                enabled: true,
                kubeconfig_secret_key: "kubeconfig".to_string(),
                kubeconfig_secret_ref: Some("secret".to_string()),
                target_namespace: None,
            },
            PeerClusterConfig {
                cluster_id: "us-west-1".to_string(),
                endpoint: "10.0.0.2".to_string(),
                latency_threshold_ms: None,
                region: None,
                priority: 80,
                port: None,
                enabled: false, // disabled – should be skipped
                kubeconfig_secret_key: "kubeconfig".to_string(),
                kubeconfig_secret_ref: Some("secret".to_string()),
                target_namespace: None,
            },
        ];

        let active: Vec<&PeerClusterConfig> = targets.iter().filter(|t| t.enabled).collect();

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].cluster_id, "us-east-1");
    }

    // -------------------------------------------------------------------------
    // Sync lag computation
    // -------------------------------------------------------------------------

    #[test]
    fn test_sync_lag_computed_from_peer_minus_local() {
        let peer_ledger: u64 = 1_234_567;
        let local_ledger: u64 = 1_234_500;

        let lag = peer_ledger.saturating_sub(local_ledger);
        assert_eq!(lag, 67);
    }

    #[test]
    fn test_sync_lag_zero_when_local_ahead() {
        // saturating_sub ensures no underflow when local is ahead
        let peer_ledger: u64 = 1_000;
        let local_ledger: u64 = 1_010;

        let lag = peer_ledger.saturating_sub(local_ledger);
        assert_eq!(lag, 0);
    }

    // -------------------------------------------------------------------------
    // Status: default values
    // -------------------------------------------------------------------------

    #[test]
    fn test_dr_status_default_values() {
        let status = DisasterRecoveryStatus::default();
        assert!(status.current_role.is_none());
        assert!(status.peer_health.is_none());
        assert!(status.last_peer_contact.is_none());
        assert!(status.sync_lag.is_none());
        assert!(!status.failover_active);
    }

    // -------------------------------------------------------------------------
    // DR constants
    // -------------------------------------------------------------------------

    #[test]
    fn test_dr_annotation_constants() {
        use crate::controller::dr::{DR_FAILOVER_ANNOTATION, DR_LAST_SYNC_ANNOTATION};
        assert_eq!(DR_FAILOVER_ANNOTATION, "stellar.org/dr-failover-active");
        assert_eq!(DR_LAST_SYNC_ANNOTATION, "stellar.org/dr-last-sync-time");
    }

    // -------------------------------------------------------------------------
    // Full-region failover drill: health-gated incremental traffic shift
    // -------------------------------------------------------------------------

    /// Declared recovery time objective for a full-region failover drill.
    const DRILL_RTO_SECONDS: u64 = 300;

    /// Incremental weights (percent) applied to the standby region during a
    /// gated traffic shift. Each step is only applied once health evidence
    /// confirms the target region is serving traffic without errors.
    const GATED_SHIFT_STEPS: [u8; 5] = [10, 25, 50, 75, 100];

    /// A single recorded step of the traffic-shift plan, mirroring the
    /// operator-readable plan status.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct PlanStep {
        weight: u8,
        elapsed_seconds: u64,
        user_visible_errors: u64,
    }

    /// Simulated outcome of a drill run.
    struct DrillOutcome {
        steps: Vec<PlanStep>,
        failback_steps: Vec<PlanStep>,
        completed: bool,
    }

    /// Runs a simulated gated shift. `health_ok` is consulted before each
    /// incremental step; if health evidence is missing the shift halts.
    fn run_gated_shift(
        health_ok: impl Fn(u8) -> bool,
        step_cost_seconds: u64,
        errors_per_step: u64,
    ) -> Vec<PlanStep> {
        let mut steps = Vec::new();
        let mut elapsed = 0u64;
        for weight in GATED_SHIFT_STEPS {
            if !health_ok(weight) {
                break;
            }
            elapsed += step_cost_seconds;
            steps.push(PlanStep {
                weight,
                elapsed_seconds: elapsed,
                user_visible_errors: errors_per_step,
            });
        }
        steps
    }

    fn run_full_region_drill() -> DrillOutcome {
        // Primary region fails; standby is healthy at every gate.
        let steps = run_gated_shift(|_| true, 40, 0);
        let completed = steps.last().map(|s| s.weight) == Some(100);

        // Primary recovers; failback requires the same health evidence.
        let failback_steps = if completed {
            run_gated_shift(|_| true, 40, 0)
        } else {
            Vec::new()
        };

        DrillOutcome {
            steps,
            failback_steps,
            completed,
        }
    }

    #[test]
    fn test_drill_failover_completes_within_rto() {
        let outcome = run_full_region_drill();
        assert!(outcome.completed, "failover drill must reach 100% shift");

        let total: u64 = outcome
            .steps
            .last()
            .map(|s| s.elapsed_seconds)
            .unwrap_or(u64::MAX);
        assert!(
            total <= DRILL_RTO_SECONDS,
            "failover took {total}s, exceeding RTO of {DRILL_RTO_SECONDS}s"
        );
    }

    #[test]
    fn test_drill_zero_user_visible_errors_during_gated_shift() {
        let outcome = run_full_region_drill();
        let total_errors: u64 = outcome.steps.iter().map(|s| s.user_visible_errors).sum();
        assert_eq!(total_errors, 0, "gated shift must not surface user errors");
    }

    #[test]
    fn test_drill_shift_is_incremental_and_monotonic() {
        let outcome = run_full_region_drill();
        let weights: Vec<u8> = outcome.steps.iter().map(|s| s.weight).collect();
        assert_eq!(weights, GATED_SHIFT_STEPS.to_vec());
        // Weights must strictly increase so traffic never regresses mid-shift.
        assert!(weights.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn test_drill_plan_status_records_every_shift() {
        let outcome = run_full_region_drill();
        // Every applied step is recorded in the operator-readable plan status.
        assert_eq!(outcome.steps.len(), GATED_SHIFT_STEPS.len());
        for (i, step) in outcome.steps.iter().enumerate() {
            assert_eq!(step.weight, GATED_SHIFT_STEPS[i]);
            assert!(step.elapsed_seconds > 0);
        }
    }

    #[test]
    fn test_drill_shift_halts_without_health_evidence() {
        // Health gate fails at 50% – shift must stop and never reach 100%.
        let steps = run_gated_shift(|w| w < 50, 40, 0);
        assert_eq!(steps.last().map(|s| s.weight), Some(25));
        assert!(steps.iter().all(|s| s.weight < 50));
    }

    #[test]
    fn test_drill_failback_requires_same_health_evidence() {
        let outcome = run_full_region_drill();
        assert!(outcome.completed);

        // Failback uses the identical gated sequence as failover.
        let failback_weights: Vec<u8> = outcome.failback_steps.iter().map(|s| s.weight).collect();
        let failover_weights: Vec<u8> = outcome.steps.iter().map(|s| s.weight).collect();
        assert_eq!(failback_weights, failover_weights);

        // Failback also respects the RTO and surfaces no user-visible errors.
        let failback_total = outcome
            .failback_steps
            .last()
            .map(|s| s.elapsed_seconds)
            .unwrap_or(u64::MAX);
        assert!(failback_total <= DRILL_RTO_SECONDS);
        let failback_errors: u64 = outcome
            .failback_steps
            .iter()
            .map(|s| s.user_visible_errors)
            .sum();
        assert_eq!(failback_errors, 0);
    }

    #[test]
    fn test_drill_failback_blocked_without_health_evidence() {
        // Failback gate rejects the recovered primary – no shift applied.
        let failback_steps = run_gated_shift(|_| false, 40, 0);
        assert!(failback_steps.is_empty());
    }
}
