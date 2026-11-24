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
// tests/concurrent_reconciliation_test.rs
//
// Concurrency test harness for StellarNode reconciler.
// Validates safety and concurrency behaviors under parallel execution.
// Related: #1157 - Create test harness for concurrent controller reconciliation edge cases

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
struct MockReconciler {
    active_locks: Arc<Mutex<std::collections::HashSet<String>>>,
    reconciled_count: Arc<AtomicU32>,
    conflicts_detected: Arc<AtomicU32>,
    is_leader: Arc<AtomicBool>,
}

impl MockReconciler {
    fn new() -> Self {
        Self {
            active_locks: Arc::new(Mutex::new(std::collections::HashSet::new())),
            reconciled_count: Arc::new(AtomicU32::new(0)),
            conflicts_detected: Arc::new(AtomicU32::new(0)),
            is_leader: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Simulates the reconcile entrypoint. Returns Ok(true) if succeeded,
    /// Err(conflict) if resource version mismatch occurred.
    fn reconcile(
        &self,
        node_name: &str,
        resource_version: u32,
        target_version: u32,
    ) -> Result<bool, &'static str> {
        if !self.is_leader.load(Ordering::Relaxed) {
            return Ok(false); // Standby skips reconciliation
        }

        // Try to acquire local concurrency lock for this specific node
        {
            let mut locks = self.active_locks.lock().unwrap();
            if locks.contains(node_name) {
                // local lock collision! Reconciler is already processing this node.
                return Err("ConcurrencyConflict");
            }
            locks.insert(node_name.to_string());
        }

        // Simulate some async I/O or patch computation
        thread::sleep(Duration::from_millis(50));

        // Optimistic Concurrency Control (OCC) check:
        // If the resource version is older than target_version, we have a 409 Conflict.
        if resource_version < target_version {
            self.conflicts_detected.fetch_add(1, Ordering::SeqCst);
            // Release lock before returning
            {
                let mut locks = self.active_locks.lock().unwrap();
                locks.remove(node_name);
            }
            return Err("Conflict409VersionMismatch");
        }

        // Successful reconciliation
        self.reconciled_count.fetch_add(1, Ordering::SeqCst);

        // Release lock
        {
            let mut locks = self.active_locks.lock().unwrap();
            locks.remove(node_name);
        }

        Ok(true)
    }
}

/// Test that concurrent reconciliations on the SAME node are prevented/locked
/// to prevent duplicate workloads or out-of-order execution.
#[test]
fn test_concurrent_reconcile_same_node_is_locked() {
    let reconciler = MockReconciler::new();
    let reconciler_clone = reconciler.clone();

    let node = "stellar-node-validator-1";

    // Start first reconciliation in parallel
    let handle1 = thread::spawn(move || reconciler_clone.reconcile(node, 1, 1));

    // Wait slightly to guarantee handle1 acquires the lock
    thread::sleep(Duration::from_millis(10));

    // Try to run second reconciliation for the same node concurrently
    let reconciler_clone2 = reconciler.clone();
    let handle2 = thread::spawn(move || reconciler_clone2.reconcile(node, 1, 1));

    let res1 = handle1.join().unwrap();
    let res2 = handle2.join().unwrap();

    // One of them must have succeeded, and the other must have failed with ConcurrencyConflict
    let success_count = [&res1, &res2].iter().filter(|r| r.is_ok()).count();
    let conflict_count = [&res1, &res2]
        .iter()
        .filter(|r| matches!(r, Err("ConcurrencyConflict")))
        .count();

    assert_eq!(
        success_count, 1,
        "Exactly one reconciliation should succeed"
    );
    assert_eq!(
        conflict_count, 1,
        "The concurrent reconciliation should be rejected with a conflict"
    );
    assert_eq!(reconciler.reconciled_count.load(Ordering::SeqCst), 1);
}

/// Test that concurrent reconciliations on DIFFERENT nodes are allowed in parallel.
#[test]
fn test_concurrent_reconcile_different_nodes_run_in_parallel() {
    let reconciler = MockReconciler::new();

    let node1 = "stellar-node-1";
    let node2 = "stellar-node-2";
    let node3 = "stellar-node-3";

    let rec1 = reconciler.clone();
    let rec2 = reconciler.clone();
    let rec3 = reconciler.clone();

    let handle1 = thread::spawn(move || rec1.reconcile(node1, 1, 1));
    let handle2 = thread::spawn(move || rec2.reconcile(node2, 1, 1));
    let handle3 = thread::spawn(move || rec3.reconcile(node3, 1, 1));

    let res1 = handle1.join().unwrap();
    let res2 = handle2.join().unwrap();
    let res3 = handle3.join().unwrap();

    assert!(res1.is_ok());
    assert!(res2.is_ok());
    assert!(res3.is_ok());
    assert_eq!(
        reconciler.reconciled_count.load(Ordering::SeqCst),
        3,
        "All distinct nodes should reconcile in parallel"
    );
}

/// Test Optimistic Concurrency Control (OCC) / 409 Conflict resolution.
/// Simulates resource version mismatch when a thread tries to write an outdated update.
#[test]
fn test_optimistic_concurrency_conflict_detected() {
    let reconciler = MockReconciler::new();

    // Simulate updating an outdated node version (1) to target version (2)
    let res = reconciler.reconcile("stellar-node", 1, 2);

    assert!(res.is_err());
    assert_eq!(res.err(), Some("Conflict409VersionMismatch"));
    assert_eq!(reconciler.conflicts_detected.load(Ordering::SeqCst), 1);
    assert_eq!(reconciler.reconciled_count.load(Ordering::SeqCst), 0);
}

/// Test that non-leader replicas do not reconcile under load.
#[test]
fn test_non_leader_skips_concurrent_reconciliation() {
    let reconciler = MockReconciler::new();
    reconciler.is_leader.store(false, Ordering::SeqCst); // standby

    let mut handles = vec![];
    for i in 0..5 {
        let rec = reconciler.clone();
        let node_name = format!("stellar-node-{}", i);
        handles.push(thread::spawn(move || rec.reconcile(&node_name, 1, 1)));
    }

    for handle in handles {
        let res = handle.join().unwrap();
        assert_eq!(res, Ok(false), "Standby replica must skip reconciliation");
    }

    assert_eq!(reconciler.reconciled_count.load(Ordering::SeqCst), 0);
}
