#![allow(clippy::needless_pass_by_value)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Env, String, Symbol, Vec,
};

pub mod compare;

pub use compare::{assert_snapshot, compare_roots, SnapshotComparison, SnapshotDiff};

const MAX_ROOT_ENTRIES: u32 = 1000;
const CLUSTER_REGISTRY: Symbol = symbol_short!("clusters");

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotStatus {
    Valid,
    Diverged,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateRootEntry {
    pub ledger_index: u32,
    pub cluster_id: String,
    pub state_root: String,
    pub submitted_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotAssertion {
    pub ledger_index: u32,
    pub cluster_id: String,
    pub state_root: String,
    pub status: SnapshotStatus,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum SnapshotError {
    InvalidCluster = 1,
    MissingStateRoot = 2,
    InvalidLedgerIndex = 3,
    DivergenceDetected = 4,
    StorageLimitExceeded = 5,
}

#[contract]
pub struct SnapshotAssertContract;

#[contractimpl]
impl SnapshotAssertContract {
    pub fn __constructor(env: Env) {
        env.storage().instance().set(&symbol_short!("init"), &true);
    }

    pub fn register_cluster(env: Env, cluster_id: String) -> Result<(), SnapshotError> {
        if cluster_id.is_empty() {
            return Err(SnapshotError::InvalidCluster);
        }

        let mut clusters: Vec<String> = env
            .storage()
            .persistent()
            .get(&CLUSTER_REGISTRY)
            .unwrap_or_else(|| Vec::new(&env));

        let mut already_registered = false;
        for existing in clusters.iter() {
            if existing == cluster_id {
                already_registered = true;
                break;
            }
        }

        if !already_registered {
            clusters.push_back(cluster_id.clone());
            env.storage().persistent().set(&CLUSTER_REGISTRY, &clusters);
        }

        env.storage().persistent().set(&cluster_id, &true);
        Ok(())
    }

    pub fn submit_snapshot(
        env: Env,
        cluster_id: String,
        ledger_index: u32,
        state_root: String,
    ) -> Result<SnapshotAssertion, SnapshotError> {
        if cluster_id.is_empty() {
            return Err(SnapshotError::InvalidCluster);
        }
        if ledger_index == 0 {
            return Err(SnapshotError::InvalidLedgerIndex);
        }
        if !env.storage().persistent().has(&cluster_id) {
            return Err(SnapshotError::InvalidCluster);
        }

        let key = (ledger_index, cluster_id.clone());
        let entry = StateRootEntry {
            ledger_index,
            cluster_id: cluster_id.clone(),
            state_root: state_root.clone(),
            submitted_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&key, &entry);
        Self::purge_stale_entries(&env, ledger_index);

        let comparison = Self::compare_snapshot(env.clone(), ledger_index)?;
        let status = if comparison.has_divergence {
            SnapshotStatus::Diverged
        } else {
            SnapshotStatus::Valid
        };

        Ok(SnapshotAssertion {
            ledger_index,
            cluster_id: cluster_id.clone(),
            state_root,
            status,
        })
    }

    pub fn get_snapshot_for_cluster(
        env: Env,
        ledger_index: u32,
        cluster_id: String,
    ) -> Option<StateRootEntry> {
        let key = (ledger_index, cluster_id);
        env.storage()
            .persistent()
            .get::<(u32, String), StateRootEntry>(&key)
    }

    pub fn compare_snapshot(
        env: Env,
        ledger_index: u32,
    ) -> Result<SnapshotComparison, SnapshotError> {
        let mut roots: Vec<(String, String)> = Vec::new(&env);
        let cluster_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&CLUSTER_REGISTRY)
            .unwrap_or_else(|| Vec::new(&env));

        for cluster_id in cluster_ids.iter() {
            if let Some(entry) = env
                .storage()
                .persistent()
                .get::<(u32, String), StateRootEntry>(&(ledger_index, cluster_id.clone()))
            {
                roots.push_back((cluster_id.clone(), entry.state_root));
            }
        }

        if roots.is_empty() {
            return Err(SnapshotError::MissingStateRoot);
        }

        let comparison = compare::compare_roots(&env, ledger_index, roots);
        if comparison.has_divergence {
            env.events().publish(
                ("StateDivergenceDetected",),
                (
                    ledger_index,
                    comparison.reference_cluster.clone(),
                    comparison.reference_root.clone(),
                    comparison.conflicting_clusters.clone(),
                ),
            );
        }

        Ok(comparison)
    }

    fn purge_stale_entries(env: &Env, ledger_index: u32) {
        let cutoff = ledger_index.saturating_sub(MAX_ROOT_ENTRIES);
        let cluster_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&CLUSTER_REGISTRY)
            .unwrap_or_else(|| Vec::new(env));

        for cluster_id in cluster_ids.iter() {
            let stale_key = (cutoff, cluster_id.clone());
            if env.storage().persistent().has(&stale_key) {
                env.storage().persistent().remove(&stale_key);
            }
        }
    }
}
