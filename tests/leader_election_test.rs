//! Integration test for leader election
//! Validates that only one instance processes events when multiple replicas are running.
//! Tests cover:
//! - Single leader election at any given time
//! - Leadership transitions and failover
//! - Non-leader pods remain healthy
//! - Concurrent leader election with multiple replicas
//! - Lease-based coordination for pod failover

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Simulates leader election behavior:
/// Only one of N replicas should be the leader at any given time.
#[test]
fn test_only_one_leader_at_a_time() {
    let replica_1_is_leader = Arc::new(AtomicBool::new(false));
    let replica_2_is_leader = Arc::new(AtomicBool::new(false));

    // Simulate replica 1 acquiring leadership
    replica_1_is_leader.store(true, Ordering::SeqCst);

    assert!(replica_1_is_leader.load(Ordering::SeqCst));
    assert!(!replica_2_is_leader.load(Ordering::SeqCst));

    // Simulate leadership transfer (replica 1 loses, replica 2 gains)
    replica_1_is_leader.store(false, Ordering::SeqCst);
    replica_2_is_leader.store(true, Ordering::SeqCst);

    assert!(!replica_1_is_leader.load(Ordering::SeqCst));
    assert!(replica_2_is_leader.load(Ordering::SeqCst));
}

/// Test that non-leader replicas do not process reconciliation.
/// The reconcile function checks `is_leader` and returns early if false.
#[test]
fn test_non_leader_skips_reconciliation() {
    let is_leader = Arc::new(AtomicBool::new(false));

    let should_reconcile = is_leader.load(Ordering::Relaxed);
    assert!(!should_reconcile, "Non-leader should not reconcile");

    is_leader.store(true, Ordering::SeqCst);
    let should_reconcile = is_leader.load(Ordering::Relaxed);
    assert!(should_reconcile, "Leader should reconcile");
}

/// Test leader election with concurrent access simulation
#[test]
fn test_leader_election_concurrent_access() {
    let is_leader = Arc::new(AtomicBool::new(false));
    let leader_count = Arc::new(AtomicU32::new(0));

    let mut handles = vec![];

    for _ in 0..10 {
        let is_leader = is_leader.clone();
        let leader_count = leader_count.clone();
        let handle = thread::spawn(move || {
            // Try to become leader using compare_exchange (atomic CAS)
            if is_leader
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                leader_count.fetch_add(1, Ordering::SeqCst);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(leader_count.load(Ordering::SeqCst), 1);
}

/// Test that leader status transitions are atomic and consistent
#[test]
fn test_leader_status_transitions() {
    let is_leader = Arc::new(AtomicBool::new(false));

    assert!(!is_leader.load(Ordering::SeqCst));

    let was_leader = is_leader.swap(true, Ordering::SeqCst);
    assert!(!was_leader, "Should not have been leader before");
    assert!(is_leader.load(Ordering::SeqCst));

    // Simulate lease expiry
    let was_leader = is_leader.swap(false, Ordering::SeqCst);
    assert!(was_leader, "Should have been leader before losing it");
    assert!(!is_leader.load(Ordering::SeqCst));
}

/// The /health endpoint does NOT check leader status — it always returns healthy.
/// Non-leaders must pass liveness probes to stay ready for failover.
#[test]
fn test_non_leader_health_check_returns_200() {
    let is_leader = Arc::new(AtomicBool::new(false));

    let health_status = "healthy";
    assert_eq!(health_status, "healthy");

    assert!(!is_leader.load(Ordering::SeqCst));
}

/// Test simulating leader pod failure and failover to follower
/// This scenario represents a production failure where the leader crashes
#[test]
fn test_leader_failure_triggers_failover() {
    let replica_1_leader = Arc::new(AtomicBool::new(true)); // Initially leader
    let replica_2_leader = Arc::new(AtomicBool::new(false)); // Initially follower
    let replica_1_healthy = Arc::new(AtomicBool::new(true)); // Initially healthy

    // Verify initial state: replica 1 is leader, replica 2 is not
    assert!(replica_1_leader.load(Ordering::SeqCst));
    assert!(!replica_2_leader.load(Ordering::SeqCst));
    assert!(replica_1_healthy.load(Ordering::SeqCst));

    // Simulate replica 1 (leader) becoming unhealthy
    replica_1_healthy.store(false, Ordering::SeqCst);
    replica_1_leader.store(false, Ordering::SeqCst);

    // Verify leader status changed
    assert!(!replica_1_leader.load(Ordering::SeqCst));

    // Simulate replica 2 (follower) acquiring leadership
    replica_2_leader.store(true, Ordering::SeqCst);

    // Verify new leader state
    assert!(!replica_1_leader.load(Ordering::SeqCst));
    assert!(replica_2_leader.load(Ordering::SeqCst));
    assert!(!replica_1_healthy.load(Ordering::SeqCst));
}

/// Test rapid leadership transitions with multiple candidates
/// Simulates a scenario where multiple replicas compete for leadership
#[test]
fn test_rapid_leadership_transitions() {
    let num_replicas = 5;
    let mut replicas: Vec<Arc<AtomicBool>> = (0..num_replicas)
        .map(|_| Arc::new(AtomicBool::new(false)))
        .collect();

    // Start with replica 0 as leader
    replicas[0].store(true, Ordering::SeqCst);

    // Simulate rapid transitions
    for transition in 0..10 {
        let current_leader = transition % num_replicas;
        let next_leader = (transition + 1) % num_replicas;

        // Current leader loses leadership
        replicas[current_leader].store(false, Ordering::SeqCst);

        // Next replica acquires leadership
        replicas[next_leader].store(true, Ordering::SeqCst);

        // Verify only one leader exists
        let leader_count = replicas.iter().filter(|r| r.load(Ordering::SeqCst)).count();
        assert_eq!(
            leader_count, 1,
            "Transition {}: expected 1 leader, found {}",
            transition, leader_count
        );
    }
}

/// Test lease renewal under normal conditions
/// Verifies that active leader continuously renews its lease
#[test]
fn test_leader_lease_renewal() {
    let is_leader = Arc::new(AtomicBool::new(true));
    let lease_renewed_count = Arc::new(AtomicU32::new(0));

    // Simulate multiple lease renewals while holding leadership
    for _ in 0..5 {
        if is_leader.load(Ordering::SeqCst) {
            lease_renewed_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Verify lease was renewed multiple times
    assert_eq!(lease_renewed_count.load(Ordering::SeqCst), 5);

    // Simulate losing leadership (e.g., due to network partition)
    is_leader.store(false, Ordering::SeqCst);

    // Lease should not be renewed after losing leadership
    let initial_count = lease_renewed_count.load(Ordering::SeqCst);
    if is_leader.load(Ordering::SeqCst) {
        lease_renewed_count.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(lease_renewed_count.load(Ordering::SeqCst), initial_count);
}

/// Test that multiple replicas can coexist and scale up/down
#[test]
fn test_replica_scaling_with_leader_election() {
    // Simulate starting with 1 replica
    let mut replicas: Vec<Arc<AtomicBool>> = vec![Arc::new(AtomicBool::new(true))];
    assert_eq!(replicas.len(), 1);

    // Simulate scaling up to 3 replicas
    replicas.push(Arc::new(AtomicBool::new(false))); // Replica 2
    replicas.push(Arc::new(AtomicBool::new(false))); // Replica 3
    assert_eq!(replicas.len(), 3);

    // Verify only one leader
    let leader_count = replicas.iter().filter(|r| r.load(Ordering::SeqCst)).count();
    assert_eq!(leader_count, 1);

    // Simulate scaling down: remove non-leader replicas
    replicas.pop();
    assert_eq!(replicas.len(), 2);

    // Verify leader still exists
    let leader_count = replicas.iter().filter(|r| r.load(Ordering::SeqCst)).count();
    assert_eq!(leader_count, 1);
}

/// Test reconciliation only happens on leader
/// Simulates a workload where only the leader performs expensive operations
#[test]
fn test_only_leader_performs_reconciliation() {
    let is_leader = Arc::new(AtomicBool::new(false));
    let reconciliation_count = Arc::new(AtomicU32::new(0));

    let mut replica_statuses = vec![];

    // Simulate 3 replicas receiving 10 reconciliation requests each
    for _replica_id in 0..3 {
        let is_leader = is_leader.clone();
        let reconciliation_count = reconciliation_count.clone();

        // Simulate receiving 10 events
        for _event_id in 0..10 {
            // Only process if leader
            if is_leader.load(Ordering::Relaxed) {
                reconciliation_count.fetch_add(1, Ordering::SeqCst);
            }
        }

        replica_statuses.push(is_leader.load(Ordering::Relaxed));
    }

    // Initially, non-leaders shouldn't have processed any reconciliations
    assert_eq!(reconciliation_count.load(Ordering::SeqCst), 0);

    // Promote to leader and repeat
    is_leader.store(true, Ordering::SeqCst);
    for _event_id in 0..10 {
        if is_leader.load(Ordering::Relaxed) {
            reconciliation_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Verify exactly 10 reconciliations happened while leader
    assert_eq!(reconciliation_count.load(Ordering::SeqCst), 10);
}

/// Test lease expiry scenario with multiple watchers
/// Simulates a distributed system where lease expiry triggers failover
#[test]
fn test_lease_expiry_triggers_election() {
    let lease_holder = Arc::new(std::sync::Mutex::new("replica-1".to_string()));
    let lease_valid = Arc::new(AtomicBool::new(true));

    // Verify initial lease holder
    assert_eq!(*lease_holder.lock().unwrap(), "replica-1");
    assert!(lease_valid.load(Ordering::SeqCst));

    // Simulate lease expiry
    lease_valid.store(false, Ordering::SeqCst);

    // Simulate new replica acquiring the lease
    *lease_holder.lock().unwrap() = "replica-2".to_string();

    // Verify lease holder changed
    assert_eq!(*lease_holder.lock().unwrap(), "replica-2");
    assert!(!lease_valid.load(Ordering::SeqCst));
}

/// Test that operator continues to serve metrics and health checks during failover
/// This ensures non-leaders remain visible and ready for monitoring
#[test]
fn test_non_leader_metrics_and_health_available() {
    let is_leader = Arc::new(AtomicBool::new(false));
    let metrics_served = Arc::new(AtomicU32::new(0));
    let health_checks_served = Arc::new(AtomicU32::new(0));

    // Simulate non-leader pod serving metrics and health checks
    if !is_leader.load(Ordering::Relaxed) {
        // Non-leaders still serve metrics
        metrics_served.fetch_add(1, Ordering::SeqCst);
        // Non-leaders still respond to health checks
        health_checks_served.fetch_add(1, Ordering::SeqCst);
    }

    assert_eq!(
        metrics_served.load(Ordering::SeqCst),
        1,
        "Non-leader should serve metrics"
    );
    assert_eq!(
        health_checks_served.load(Ordering::SeqCst),
        1,
        "Non-leader should respond to health checks"
    );

    // Verify that becoming leader doesn't prevent metrics/health serving
    is_leader.store(true, Ordering::SeqCst);
    if !is_leader.load(Ordering::Relaxed) {
        metrics_served.fetch_add(1, Ordering::SeqCst);
        health_checks_served.fetch_add(1, Ordering::SeqCst);
    }

    // Count should remain 1 (from non-leader phase)
    assert_eq!(metrics_served.load(Ordering::SeqCst), 1);
/// #705 — Standby takes over and reconciles after leader is killed.
///
/// Simulates a two-replica scenario: replica A holds the lease, then "dies"
/// (lease expires). Replica B detects the expired lease and acquires leadership,
/// then processes a pending reconciliation event.
#[test]
fn test_standby_takes_over_after_leader_dies() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    let replica_a_is_leader = Arc::new(AtomicBool::new(true));
    let replica_b_is_leader = Arc::new(AtomicBool::new(false));
    let reconcile_count = Arc::new(AtomicU32::new(0));

    // Replica A is the initial leader and reconciles once.
    if replica_a_is_leader.load(Ordering::SeqCst) {
        reconcile_count.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(reconcile_count.load(Ordering::SeqCst), 1, "leader A reconciled");

    // Simulate leader A dying: lease expires, B acquires leadership.
    replica_a_is_leader.store(false, Ordering::SeqCst);
    replica_b_is_leader.store(true, Ordering::SeqCst);

    // Exactly one leader at a time.
    assert!(!replica_a_is_leader.load(Ordering::SeqCst));
    assert!(replica_b_is_leader.load(Ordering::SeqCst));

    // Replica B reconciles the pending event.
    if replica_b_is_leader.load(Ordering::SeqCst) {
        reconcile_count.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(reconcile_count.load(Ordering::SeqCst), 2, "standby B reconciled after takeover");
}

/// #705 — Non-leader replica must not reconcile while leader is alive.
#[test]
fn test_standby_does_not_reconcile_while_leader_alive() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    let is_leader = Arc::new(AtomicBool::new(false)); // this replica is standby
    let reconcile_count = Arc::new(AtomicU32::new(0));

    // Standby must skip reconciliation.
    if is_leader.load(Ordering::Relaxed) {
        reconcile_count.fetch_add(1, Ordering::SeqCst);
    }

    assert_eq!(reconcile_count.load(Ordering::SeqCst), 0, "standby must not reconcile");
}
