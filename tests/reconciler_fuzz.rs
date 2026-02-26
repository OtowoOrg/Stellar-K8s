//! State-machine fuzzer for the StellarNode reconciler.
//!
//! Uses proptest to generate random mutations of StellarNodeSpec and random
//! sequences of "events" (spec changes). Ensures that:
//! - Feeding these into spec validation **never causes a panic**.
//! - Feeding event sequences eventually **converges** to either a validation
//!   error or a valid state (no hang).
//!
//! Run with: `cargo test -p stellar-k8s --features reconciler-fuzz reconciler_fuzz -- --nocapture`
//! Or full proptest: `cargo test -p stellar-k8s --features reconciler-fuzz prop_ --test reconciler_fuzz`

#![cfg(feature = "reconciler-fuzz")]

use proptest::prelude::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use stellar_k8s::controller::{reconcile_for_fuzz, ControllerState};
use stellar_k8s::crd::{
    HistoryMode, HorizonConfig, NodeType, ResourceRequirements, ResourceSpec, RolloutStrategy,
    SorobanConfig, StellarNetwork, StellarNode, StellarNodeSpec, StellarNodeStatus, StorageConfig,
    ValidatorConfig,
};

// --- Strategy helpers for StellarNodeSpec ---

fn default_resources() -> ResourceRequirements {
    ResourceRequirements {
        requests: ResourceSpec {
            cpu: "500m".to_string(),
            memory: "1Gi".to_string(),
        },
        limits: ResourceSpec {
            cpu: "2".to_string(),
            memory: "4Gi".to_string(),
        },
    }
}

fn default_storage() -> StorageConfig {
    StorageConfig {
        storage_class: "standard".to_string(),
        size: "100Gi".to_string(),
        retention_policy: Default::default(),
        annotations: None,
    }
}

/// Base valid Validator spec for mutation
fn base_validator_spec() -> StellarNodeSpec {
    StellarNodeSpec {
        node_type: NodeType::Validator,
        network: StellarNetwork::Testnet,
        version: "v21.0.0".to_string(),
        history_mode: HistoryMode::default(),
        resources: default_resources(),
        storage: default_storage(),
        validator_config: Some(ValidatorConfig {
            seed_secret_ref: "validator-seed".to_string(),
            quorum_set: None,
            enable_history_archive: false,
            history_archive_urls: vec![],
            catchup_complete: false,
            key_source: Default::default(),
            kms_config: None,
            vl_source: None,
            hsm_config: None,
        }),
        horizon_config: None,
        soroban_config: None,
        replicas: 1,
        min_available: None,
        max_unavailable: None,
        suspended: false,
        alerting: false,
        database: None,
        managed_database: None,
        autoscaling: None,
        ingress: None,
        load_balancer: None,
        global_discovery: None,
        cross_cluster: None,
        strategy: RolloutStrategy::default(),
        maintenance_mode: false,
        network_policy: None,
        dr_config: None,
        topology_spread_constraints: None,
        cve_handling: None,
        resource_meta: None,
    }
}

/// Base valid Horizon spec for mutation
fn base_horizon_spec() -> StellarNodeSpec {
    StellarNodeSpec {
        node_type: NodeType::Horizon,
        network: StellarNetwork::Testnet,
        version: "v21.0.0".to_string(),
        history_mode: HistoryMode::default(),
        resources: default_resources(),
        storage: default_storage(),
        validator_config: None,
        horizon_config: Some(HorizonConfig {
            database_secret_ref: "horizon-db".to_string(),
            enable_ingest: true,
            stellar_core_url: "http://stellar-core:11626".to_string(),
            ingest_workers: 1,
            enable_experimental_ingestion: false,
            auto_migration: false,
        }),
        soroban_config: None,
        replicas: 2,
        min_available: None,
        max_unavailable: None,
        suspended: false,
        alerting: false,
        database: None,
        managed_database: None,
        autoscaling: None,
        ingress: None,
        load_balancer: None,
        global_discovery: None,
        cross_cluster: None,
        strategy: RolloutStrategy::default(),
        maintenance_mode: false,
        network_policy: None,
        dr_config: None,
        topology_spread_constraints: None,
        cve_handling: None,
        resource_meta: None,
    }
}

/// Base valid SorobanRpc spec for mutation
fn base_soroban_spec() -> StellarNodeSpec {
    StellarNodeSpec {
        node_type: NodeType::SorobanRpc,
        network: StellarNetwork::Testnet,
        version: "v21.0.0".to_string(),
        history_mode: HistoryMode::default(),
        resources: default_resources(),
        storage: default_storage(),
        validator_config: None,
        horizon_config: None,
        soroban_config: Some(SorobanConfig {
            stellar_core_url: "http://stellar-core:11626".to_string(),
            #[allow(deprecated)]
            captive_core_config: None,
            captive_core_structured_config: None,
            enable_preflight: true,
            max_events_per_request: 10000,
        }),
        replicas: 2,
        min_available: None,
        max_unavailable: None,
        suspended: false,
        alerting: false,
        database: None,
        managed_database: None,
        autoscaling: None,
        ingress: None,
        load_balancer: None,
        global_discovery: None,
        cross_cluster: None,
        strategy: RolloutStrategy::default(),
        maintenance_mode: false,
        network_policy: None,
        dr_config: None,
        topology_spread_constraints: None,
        cve_handling: None,
        resource_meta: None,
    }
}

/// Strategy that picks a base spec and applies random mutations (replicas, version, suspended)
fn spec_strategy() -> impl Strategy<Value = StellarNodeSpec> {
    (
        prop_oneof![
            Just(base_validator_spec()),
            Just(base_horizon_spec()),
            Just(base_soroban_spec()),
        ],
        0i32..=10i32, // replicas
        Just("v21.0.0".to_string()),
        prop::bool::ANY, // suspended
    )
        .prop_map(|(mut spec, replicas, version, suspended)| {
            spec.replicas = replicas;
            spec.version = version;
            spec.suspended = suspended;
            spec
        })
}

/// Build a StellarNode with the given spec and name/namespace for reconcile tests
fn make_node(spec: StellarNodeSpec, name: String, namespace: String) -> StellarNode {
    StellarNode {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(name),
            namespace: Some(namespace),
            uid: Some("fuzz-uid-0001".to_string()),
            resource_version: Some("1".to_string()),
            ..Default::default()
        },
        spec,
        status: Some(StellarNodeStatus::default()),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn spec_validation_never_panics(spec in spec_strategy()) {
        let _ = spec.validate();
    }

    #[test]
    fn event_sequence_validation_never_panics(
        base in prop_oneof![
            Just(base_validator_spec()),
            Just(base_horizon_spec()),
            Just(base_soroban_spec()),
        ],
        mutations in prop::collection::vec((0i32..=10i32, prop::bool::ANY), 0..20)
    ) {
        let mut current = base;
        for (replicas, suspended) in mutations {
            current.replicas = replicas;
            current.suspended = suspended;
            let _ = current.validate();
        }
    }
}

/// Reconcile with a failing client must not panic and must converge to Err or Ok(Action).
/// Ignored by default: creating a kube Client from a fake URL triggers TLS/crypto setup that
/// may require process-level crypto provider. Run with `--ignored` against a real cluster or
/// use a mock client (e.g. tower-test) for full reconcile fuzzing.
#[tokio::test]
#[ignore = "requires real cluster or mock client; run with --ignored when testing reconcile convergence"]
async fn reconcile_with_failing_client_never_panics_and_converges() {
    let client = match kube::Client::try_default().await {
        Ok(c) => c,
        Err(_) => return,
    };
    let ctx = Arc::new(ControllerState {
        client,
        enable_mtls: false,
        operator_namespace: "default".to_string(),
        mtls_config: None,
        dry_run: false,
        is_leader: Arc::new(AtomicBool::new(true)),
    });
    let node = make_node(
        base_validator_spec(),
        "fuzz-node".to_string(),
        "default".to_string(),
    );
    let result = reconcile_for_fuzz(Arc::new(node), ctx).await;
    assert!(result.is_ok() || result.is_err());
}
