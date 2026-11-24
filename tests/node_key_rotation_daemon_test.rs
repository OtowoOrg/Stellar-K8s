use async_trait::async_trait;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::Arc;
use stellar_k8s::controller::security::{
    ConsensusSnapshot, KeyRotationWorker, ManagedSeedSecret, MemorySecretManager, RotationOutcome,
    RotationStage, SeedGenerator, StellarCoreAdmin, ValidatorKeyRotationConfig,
    ValidatorSeedMaterial,
};
use stellar_k8s::{Error, Result};
use tokio::sync::Mutex;

#[derive(Clone)]
struct FixedSeedGenerator {
    material: ValidatorSeedMaterial,
}

impl SeedGenerator for FixedSeedGenerator {
    fn generate_seed(&self) -> Result<ValidatorSeedMaterial> {
        Ok(self.material.clone())
    }
}

#[derive(Clone)]
struct ScriptedCoreAdmin {
    snapshots: Arc<Mutex<VecDeque<ConsensusSnapshot>>>,
    applies: Arc<Mutex<u32>>,
    rollbacks: Arc<Mutex<u32>>,
}

impl ScriptedCoreAdmin {
    fn new(snapshots: Vec<ConsensusSnapshot>) -> Self {
        Self {
            snapshots: Arc::new(Mutex::new(snapshots.into())),
            applies: Arc::new(Mutex::new(0)),
            rollbacks: Arc::new(Mutex::new(0)),
        }
    }

    async fn apply_count(&self) -> u32 {
        *self.applies.lock().await
    }

    async fn rollback_count(&self) -> u32 {
        *self.rollbacks.lock().await
    }
}

#[async_trait]
impl StellarCoreAdmin for ScriptedCoreAdmin {
    async fn consensus_snapshot(&self) -> Result<ConsensusSnapshot> {
        self.snapshots
            .lock()
            .await
            .pop_front()
            .ok_or_else(|| Error::InternalError("test snapshot script exhausted".to_string()))
    }

    async fn apply_candidate(&self, _candidate: &ManagedSeedSecret) -> Result<()> {
        *self.applies.lock().await += 1;
        Ok(())
    }

    async fn rollback_to_previous(&self, _previous: &ManagedSeedSecret) -> Result<()> {
        *self.rollbacks.lock().await += 1;
        Ok(())
    }
}

fn config() -> ValidatorKeyRotationConfig {
    ValidatorKeyRotationConfig {
        enabled: true,
        interval_secs: 60,
        validation_window_secs: 0,
        validation_sample_interval_secs: 0,
        min_authenticated_peers: 1,
        max_unhealthy_samples: 0,
        rollback_on_failure: true,
        aws_region: None,
    }
}

fn healthy(sequence: u64) -> ConsensusSnapshot {
    ConsensusSnapshot {
        ledger_sequence: sequence,
        ledger_hash: format!("hash-{sequence}"),
        state: "Synced!".to_string(),
        authenticated_peer_count: 3,
        node_id: Some("GVALIDATOR".to_string()),
        observed_at: Utc::now(),
    }
}

fn unhealthy(sequence: u64) -> ConsensusSnapshot {
    ConsensusSnapshot {
        state: "Catching up".to_string(),
        authenticated_peer_count: 0,
        ..healthy(sequence)
    }
}

#[tokio::test]
async fn integration_rotation_promotes_candidate_with_clean_logs() {
    let previous = ValidatorSeedMaterial::generate().unwrap();
    let candidate = ValidatorSeedMaterial::generate().unwrap();
    let candidate_seed = candidate.secret_seed().to_string();
    let backend = Arc::new(MemorySecretManager::new(previous));
    let admin = Arc::new(ScriptedCoreAdmin::new(vec![healthy(100), healthy(101)]));
    let worker = KeyRotationWorker::new(
        "stellar-system",
        "validator-a",
        config(),
        backend.clone(),
        admin.clone(),
    )
    .with_seed_generator(Arc::new(FixedSeedGenerator {
        material: candidate.clone(),
    }));

    let report = worker.rotate_once().await.unwrap();
    let serialized_report = serde_json::to_string(&report).unwrap();

    assert_eq!(report.outcome, RotationOutcome::Completed);
    assert!(!report.rollback_performed);
    assert!(report
        .events
        .iter()
        .any(|event| event.stage == RotationStage::Completed));
    assert!(report
        .events
        .iter()
        .filter_map(|event| event.consensus_healthy)
        .all(|healthy| healthy));
    assert!(!serialized_report.contains(&candidate_seed));
    println!("rotation_report={serialized_report}");
    assert_eq!(
        backend.current().await.material.public_key,
        candidate.public_key
    );
    assert_eq!(admin.apply_count().await, 1);
    assert_eq!(admin.rollback_count().await, 0);
}

#[tokio::test]
async fn integration_rotation_rolls_back_on_consensus_drop() {
    let previous = ValidatorSeedMaterial::generate().unwrap();
    let previous_public_key = previous.public_key.clone();
    let candidate = ValidatorSeedMaterial::generate().unwrap();
    let backend = Arc::new(MemorySecretManager::new(previous));
    let admin = Arc::new(ScriptedCoreAdmin::new(vec![healthy(100), unhealthy(101)]));
    let worker = KeyRotationWorker::new(
        "stellar-system",
        "validator-a",
        config(),
        backend.clone(),
        admin.clone(),
    )
    .with_seed_generator(Arc::new(FixedSeedGenerator {
        material: candidate,
    }));

    let report = worker.rotate_once().await.unwrap();

    assert_eq!(report.outcome, RotationOutcome::RolledBack);
    assert!(report.rollback_performed);
    assert_eq!(
        backend.current().await.material.public_key,
        previous_public_key
    );
    assert_eq!(admin.apply_count().await, 1);
    assert_eq!(admin.rollback_count().await, 1);
}
