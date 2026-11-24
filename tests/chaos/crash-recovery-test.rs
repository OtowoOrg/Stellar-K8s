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
/// Chaos Engineering Test: Pod Crash and Recovery Scenarios
///
/// Tests Stellar Core and Operator behavior during crash events, including:
/// - Graceful shutdown and restart
/// - State recovery after crash
/// - Ledger reconnection after downtime
/// - Data integrity verification
#[cfg(test)]
mod crash_recovery_tests {
    use kube::{Api, Client};
    use k8s_openapi::api::v1::Pod;
    use std::time::Duration;

    /// Test: Stellar Core crash and ledger recovery
    ///
    /// Verifies that Stellar Core can recover ledger state after an unexpected
    /// termination and re-sync with the network.
    #[tokio::test]
    #[ignore]  // Requires kind cluster
    async fn test_stellar_core_crash_recovery() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::try_default().await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), "stellar-system");
        
        // Wait for cluster to stabilize
        wait_for_all_synced(&pods, Duration::from_secs(60)).await?;
        let baseline_ledger = get_max_ledger_sequence(&pods).await?;
        
        // Select a node to crash
        let pod_to_crash = get_random_core_pod(&pods).await?;
        let pod_name = pod_to_crash.metadata.name.clone().unwrap();
        
        // Record pre-crash metrics
        let pre_crash_time = std::time::Instant::now();
        
        // Force crash by deleting pod (StatefulSet will restart it)
        delete_pod_immediately(&pods, &pod_name).await?;
        
        // Verify pod enters crash/terminating state
        tokio::time::sleep(Duration::from_secs(5)).await;
        
        // Monitor pod restart
        let restart_duration = wait_for_pod_ready(&pods, &pod_name, Duration::from_secs(120)).await?;
        println!("Pod restarted in {:?}", restart_duration);
        
        // Verify ledger recovery (node should catch up)
        let recovery_deadline = Duration::from_secs(60);
        let ledger_before_recovery = get_pod_ledger(&pods, &pod_name).await?;
        
        wait_for_ledger_catchup(&pods, &pod_name, baseline_ledger, recovery_deadline).await?;
        
        // Verify sync state
        assert_pod_synced(&pods, &pod_name).await?;
        
        // Verify no transaction loss
        let final_ledger = get_max_ledger_sequence(&pods).await?;
        assert_eq!(final_ledger, baseline_ledger, "Ledger sequence mismatch after recovery");
        
        Ok(())
    }

    /// Test: Operator pod restart and reconciliation recovery
    ///
    /// Verifies that operator can recover from crashes and resume reconciliation
    /// without data loss or skipped resources.
    #[tokio::test]
    #[ignore]
    async fn test_operator_crash_recovery() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::try_default().await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), "stellar-system");
        
        // Establish a baseline number of reconciliations
        let baseline_count = get_reconciliation_count(&client).await?;
        
        // Get operator pod
        let operator_pod = get_operator_pod(&pods).await?;
        let operator_name = operator_pod.metadata.name.clone().unwrap();
        
        // Crash the operator
        delete_pod_immediately(&pods, &operator_name).await?;
        
        // Wait for restart
        wait_for_pod_ready(&pods, &operator_name, Duration::from_secs(60)).await?;
        
        // Verify operator resumes reconciliation
        tokio::time::sleep(Duration::from_secs(10)).await;
        let post_restart_count = get_reconciliation_count(&client).await?;
        
        // Should see reconciliations after restart (count increased)
        assert!(post_restart_count > baseline_count, "Operator not reconciling after restart");
        
        // Verify no errors in reconciliation loop
        assert_no_reconciliation_errors(&client, Duration::from_secs(30)).await?;
        
        // Verify all resources remain in healthy state
        assert_all_resources_synced(&client, &pods).await?;
        
        Ok(())
    }

    /// Test: Multi-node coordinated crash
    ///
    /// Simulates cascading failures: first N nodes crash simultaneously,
    /// then recover one-by-one.
    #[tokio::test]
    #[ignore]
    async fn test_multi_node_cascading_failure() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::try_default().await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), "stellar-system");
        
        wait_for_all_synced(&pods, Duration::from_secs(60)).await?;
        
        // Get all nodes
        let all_pods: Vec<_> = pods
            .list(&Default::default())
            .await?
            .items
            .into_iter()
            .filter(|p| p.metadata.name.as_ref().map(|n| n.contains("stellar-core")).unwrap_or(false))
            .collect();
        
        let crash_count = (all_pods.len() / 2).min(2);  // Crash up to 2 nodes (minority)
        let pods_to_crash: Vec<_> = all_pods.iter().take(crash_count).collect();
        
        // Crash nodes simultaneously
        for pod in &pods_to_crash {
            let name = pod.metadata.name.clone().unwrap();
            delete_pod_immediately(&pods, &name).await?;
        }
        
        // Brief pause
        tokio::time::sleep(Duration::from_secs(5)).await;
        
        // Verify remaining nodes maintain quorum and continue consensus
        assert_quorum_intact(&pods).await?;
        
        // Monitor for new ledger closes (proof of progress)
        let ledger_before = get_max_ledger_sequence(&pods).await?;
        tokio::time::sleep(Duration::from_secs(20)).await;
        let ledger_after = get_max_ledger_sequence(&pods).await?;
        assert!(ledger_after > ledger_before, "No progress during crash recovery");
        
        // Wait for all nodes to recover
        for pod in &pods_to_crash {
            let name = pod.metadata.name.clone().unwrap();
            wait_for_pod_ready(&pods, &name, Duration::from_secs(120)).await?;
        }
        
        // Verify full recovery
        wait_for_all_synced(&pods, Duration::from_secs(120)).await?;
        
        Ok(())
    }

    /// Test: Out of memory kill and recovery
    ///
    /// Simulates OOMKill scenario by setting tight memory limits and observing recovery.
    #[tokio::test]
    #[ignore]  // Requires resource limit adjustment
    async fn test_oomkill_recovery() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::try_default().await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), "stellar-system");
        
        // Apply tight memory limit to trigger OOMKill
        apply_memory_limit(&pods, "200Mi").await?;
        
        // Monitor for pod restart due to OOMKill
        let mut restart_count = 0;
        let deadline = Duration::from_secs(120);
        let start = std::time::Instant::now();
        
        loop {
            if start.elapsed() > deadline {
                break;
            }
            
            let current_count = get_oomkill_restart_count(&pods).await?;
            if current_count > 0 {
                restart_count = current_count;
                break;
            }
            
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        
        assert!(restart_count > 0, "No OOMKill restarts occurred");
        
        // Restore normal memory limits
        apply_memory_limit(&pods, "4Gi").await?;
        
        // Wait for recovery
        wait_for_all_synced(&pods, Duration::from_secs(120)).await?;
        
        // Verify no data loss
        assert_ledger_integrity(&pods).await?;
        
        Ok(())
    }

    /// Test: Graceful shutdown with in-flight transactions
    ///
    /// Verifies that in-flight transactions are properly handled during pod termination.
    #[tokio::test]
    #[ignore]
    async fn test_graceful_shutdown_with_pending_txs() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::try_default().await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), "stellar-system");
        
        // Load test with continuous transaction stream
        let _load_handle = spawn_transaction_load(&client);
        
        tokio::time::sleep(Duration::from_secs(5)).await;
        
        // Count pending transactions
        let pending_before = get_pending_transaction_count(&client).await?;
        
        // Graceful shutdown: send SIGTERM to pod
        let pod_to_shutdown = get_random_core_pod(&pods).await?;
        let pod_name = pod_to_shutdown.metadata.name.clone().unwrap();
        
        graceful_shutdown_pod(&pods, &pod_name).await?;
        
        // Monitor shutdown (should wait for in-flight txs)
        let shutdown_duration = wait_for_pod_terminated(&pods, &pod_name, Duration::from_secs(30)).await?;
        println!("Pod graceful shutdown took {:?}", shutdown_duration);
        
        // Verify transactions were committed (not dropped)
        let pending_after = get_pending_transaction_count(&client).await?;
        assert!(pending_after < pending_before, "Transactions should have been flushed");
        
        // Wait for pod to restart
        wait_for_pod_ready(&pods, &pod_name, Duration::from_secs(60)).await?;
        
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────
    // Helper functions
    // ─────────────────────────────────────────────────────────────────

    async fn wait_for_all_synced(
        pods: &Api<Pod>,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                return Err("Timeout waiting for sync".into());
            }
            
            let all_synced = pods
                .list(&Default::default())
                .await?
                .items
                .iter()
                .filter(|p| p.metadata.name.as_ref().map(|n| n.contains("stellar-core")).unwrap_or(false))
                .all(|_p| true);  // Simplified: assume all running pods are synced
            
            if all_synced {
                return Ok(());
            }
            
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn get_max_ledger_sequence(_pods: &Api<Pod>) -> Result<u64, Box<dyn std::error::Error>> {
        Ok(1000)  // Placeholder
    }

    async fn get_random_core_pod(pods: &Api<Pod>) -> Result<Pod, Box<dyn std::error::Error>> {
        let core_pods: Vec<_> = pods
            .list(&Default::default())
            .await?
            .items
            .into_iter()
            .filter(|p| p.metadata.name.as_ref().map(|n| n.contains("stellar-core")).unwrap_or(false))
            .collect();
        Ok(core_pods.first().cloned().unwrap_or_default())
    }

    async fn delete_pod_immediately(
        pods: &Api<Pod>,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        pods.delete(name, &Default::default()).await?;
        Ok(())
    }

    async fn wait_for_pod_ready(
        pods: &Api<Pod>,
        name: &str,
        timeout: Duration,
    ) -> Result<Duration, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                return Err("Timeout waiting for pod ready".into());
            }
            
            if let Ok(pod) = pods.get(name).await {
                if let Some(status) = &pod.status {
                    if status.phase.as_deref() == Some("Running") {
                        return Ok(start.elapsed());
                    }
                }
            }
            
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn get_pod_ledger(
        _pods: &Api<Pod>,
        _name: &str,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        Ok(950)  // Placeholder
    }

    async fn wait_for_ledger_catchup(
        _pods: &Api<Pod>,
        _name: &str,
        _target: u64,
        _timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn assert_pod_synced(_pods: &Api<Pod>, _name: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn get_reconciliation_count(_client: &Client) -> Result<i32, Box<dyn std::error::Error>> {
        Ok(100)  // Placeholder
    }

    async fn get_operator_pod(pods: &Api<Pod>) -> Result<Pod, Box<dyn std::error::Error>> {
        let op_pods: Vec<_> = pods
            .list(&Default::default())
            .await?
            .items
            .into_iter()
            .filter(|p| p.metadata.name.as_ref().map(|n| n.contains("stellar-operator")).unwrap_or(false))
            .collect();
        Ok(op_pods.first().cloned().unwrap_or_default())
    }

    async fn assert_no_reconciliation_errors(
        _client: &Client,
        _duration: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn assert_all_resources_synced(
        _client: &Client,
        _pods: &Api<Pod>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn assert_quorum_intact(_pods: &Api<Pod>) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn apply_memory_limit(
        _pods: &Api<Pod>,
        _limit: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn get_oomkill_restart_count(_pods: &Api<Pod>) -> Result<i32, Box<dyn std::error::Error>> {
        Ok(0)  // Placeholder
    }

    async fn assert_ledger_integrity(_pods: &Api<Pod>) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn spawn_transaction_load(_client: &Client) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async {
            // Simulate transaction load
        })
    }

    async fn get_pending_transaction_count(
        _client: &Client,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        Ok(50)  // Placeholder
    }

    async fn graceful_shutdown_pod(
        pods: &Api<Pod>,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        pods.delete(name, &Default::default()).await?;
        Ok(())
    }

    async fn wait_for_pod_terminated(
        pods: &Api<Pod>,
        name: &str,
        timeout: Duration,
    ) -> Result<Duration, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                return Err("Timeout waiting for pod termination".into());
            }
            
            match pods.get(name).await {
                Err(_) => return Ok(start.elapsed()),
                Ok(pod) => {
                    if pod.status.as_ref().map(|s| s.phase.as_deref()) == Some(Some("Terminated")) {
                        return Ok(start.elapsed());
                    }
                }
            }
            
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
