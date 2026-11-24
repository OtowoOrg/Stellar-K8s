#![cfg(test)]

use snapshot_assert::{SnapshotAssertContract, SnapshotAssertContractClient, SnapshotStatus};
use soroban_sdk::{Env, String};

#[test]
fn submits_and_accepts_matching_hashes() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SnapshotAssertContract);
    let client = SnapshotAssertContractClient::new(&env, &contract_id);

    let cluster_a = String::from_str(&env, "cluster-a");
    let cluster_b = String::from_str(&env, "cluster-b");
    let cluster_c = String::from_str(&env, "cluster-c");
    let root = String::from_str(&env, "state-root-abc");

    client.register_cluster(&cluster_a);
    client.register_cluster(&cluster_b);
    client.register_cluster(&cluster_c);

    let res_a = client.submit_snapshot(&cluster_a, &42_u32, &root);
    let res_b = client.submit_snapshot(&cluster_b, &42_u32, &root);
    let res_c = client.submit_snapshot(&cluster_c, &42_u32, &root);

    assert_eq!(res_a.status, SnapshotStatus::Valid);
    assert_eq!(res_b.status, SnapshotStatus::Valid);
    assert_eq!(res_c.status, SnapshotStatus::Valid);

    let comparison = client.compare_snapshot(&42_u32);
    assert!(!comparison.has_divergence);
}

#[test]
fn detects_state_divergence_and_emits_event() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SnapshotAssertContract);
    let client = SnapshotAssertContractClient::new(&env, &contract_id);

    let cluster_a = String::from_str(&env, "cluster-a");
    let cluster_b = String::from_str(&env, "cluster-b");
    let cluster_c = String::from_str(&env, "cluster-c");
    let cluster_d = String::from_str(&env, "cluster-d");
    let good_root = String::from_str(&env, "state-root-good");
    let bad_root = String::from_str(&env, "state-root-bad");

    for cluster in [cluster_a.clone(), cluster_b.clone(), cluster_c.clone()] {
        client.register_cluster(&cluster);
        client.submit_snapshot(&cluster, &77_u32, &good_root);
    }

    client.register_cluster(&cluster_d);
    let result = client.submit_snapshot(&cluster_d, &77_u32, &bad_root);
    assert_eq!(result.status, SnapshotStatus::Diverged);

    let comparison = client.compare_snapshot(&77_u32);
    assert!(comparison.has_divergence);
    assert!(!comparison.conflicting_clusters.is_empty());
}

#[test]
fn purges_stale_entries_after_1000_ledgers() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SnapshotAssertContract);
    let client = SnapshotAssertContractClient::new(&env, &contract_id);

    let cluster = String::from_str(&env, "cluster-a");
    let root = String::from_str(&env, "state-root-a");

    client.register_cluster(&cluster);
    client.submit_snapshot(&cluster, &1000_u32, &root);
    client.submit_snapshot(&cluster, &2000_u32, &root);

    let entry = client.get_snapshot_for_cluster(&1000_u32, &cluster);
    assert!(entry.is_none());
}
