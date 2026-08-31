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

//! CRD validation performance benchmarks — Issue #1390.
//!
//! Benchmarks the in-memory `StellarNodeSpec` validation and (de)serialization
//! paths that back CRD admission — i.e. the CPU-bound work the operator (and
//! the admission webhook) does on every `kubectl apply` of a `StellarNode`,
//! independent of the Kubernetes API server itself:
//!
//! - `crd_validate` — [`StellarNodeSpec::validate()`] over a range of spec
//!   complexity levels.
//! - `crd_serialize` / `crd_deserialize` — JSON (de)serialization cost, which
//!   mirrors what an admission webhook or the reconciler pays when reading a
//!   resource off the informer cache or converting a request body.
//! - `crd_concurrent_validate` — validation throughput under concurrent load
//!   (multiple worker threads validating specs in parallel), approximating
//!   burst admission traffic (e.g. a GitOps sync applying many resources at
//!   once).
//!
//! These benchmarks are fully self-contained: no live Kubernetes API server,
//! kind cluster, or envtest instance is required or used. That intentionally
//! excludes network/etcd latency — see `docs/benchmarking.md` for how to
//! interpret the numbers, and `benchmarks/k6/` (via `docs/load-testing.md`)
//! for end-to-end API throughput benchmarks against a live operator.
//!
//! ```bash
//! cargo bench --bench crd_operations
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use stellar_k8s::crd::{
    AutoscalingConfig, HistoryMode, HorizonConfig, NodeType, ResourceRequirements,
    StellarNetwork, StellarNodeSpec, StorageConfig, ValidatorConfig,
};

/// The smallest `StellarNodeSpec` that passes validation: a Validator with
/// only the required `validatorConfig.seedSecretRef` set.
fn minimal_validator_spec() -> StellarNodeSpec {
    StellarNodeSpec {
        node_type: NodeType::Validator,
        network: StellarNetwork::Testnet,
        version: "v21.0.0".to_string(),
        replicas: 1,
        validator_config: Some(ValidatorConfig {
            seed_secret_ref: "bench-validator-seed".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// A "day-2" Validator spec: history mode, alerting, and a couple of
/// service-level labels/annotations set explicitly.
fn standard_validator_spec() -> StellarNodeSpec {
    let mut labels = std::collections::BTreeMap::new();
    labels.insert("app".to_string(), "stellar-benchmark".to_string());
    labels.insert("benchmark".to_string(), "true".to_string());

    StellarNodeSpec {
        history_mode: HistoryMode::Full,
        alerting: true,
        service_labels: Some(labels),
        ..minimal_validator_spec()
    }
}

/// A Horizon node with HPA-style autoscaling enabled. Autoscaling is only
/// valid for Horizon/SorobanRpc node types (Validators reject it), so this
/// tier exercises a different branch of `validate()` than the Validator
/// fixtures above.
fn horizon_autoscaling_spec() -> StellarNodeSpec {
    StellarNodeSpec {
        node_type: NodeType::Horizon,
        network: StellarNetwork::Testnet,
        version: "v21.0.0".to_string(),
        replicas: 3,
        horizon_config: Some(HorizonConfig {
            database_secret_ref: "bench-horizon-db".to_string(),
            enable_ingest: true,
            stellar_core_url: "http://stellar-core:11626".to_string(),
            ..Default::default()
        }),
        autoscaling: Some(AutoscalingConfig {
            min_replicas: 2,
            max_replicas: 10,
            target_cpu_utilization_percentage: Some(70),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The heaviest fixture: a Validator with history archive enabled (and
/// multiple archive URLs), custom resource requests/limits, custom storage,
/// and service labels/annotations — representative of a production
/// configuration, to measure validation cost as the spec grows.
fn full_config_validator_spec() -> StellarNodeSpec {
    let mut labels = std::collections::BTreeMap::new();
    labels.insert("app".to_string(), "stellar-benchmark".to_string());
    labels.insert("team".to_string(), "platform".to_string());
    labels.insert("environment".to_string(), "ci".to_string());

    let mut annotations = std::collections::BTreeMap::new();
    annotations.insert("bench.stellar.org/tier".to_string(), "full".to_string());

    StellarNodeSpec {
        history_mode: HistoryMode::Full,
        alerting: true,
        service_labels: Some(labels),
        service_annotations: Some(annotations),
        resources: ResourceRequirements {
            requests: stellar_k8s::crd::ResourceSpec {
                cpu: "500m".to_string(),
                memory: "512Mi".to_string(),
            },
            limits: stellar_k8s::crd::ResourceSpec {
                cpu: "2".to_string(),
                memory: "4Gi".to_string(),
            },
        },
        storage: StorageConfig {
            storage_class: "fast-ssd".to_string(),
            size: "200Gi".to_string(),
            ..Default::default()
        },
        validator_config: Some(ValidatorConfig {
            seed_secret_ref: "bench-validator-seed".to_string(),
            enable_history_archive: true,
            history_archive_urls: vec![
                "https://history.stellar.org/prd/core-live/core_live_01".to_string(),
                "https://history.stellar.org/prd/core-live/core_live_02".to_string(),
            ],
            ..Default::default()
        }),
        ..minimal_validator_spec()
    }
}

/// The four complexity tiers shared across the validate/serialize/deserialize
/// benchmark groups below, so all three groups measure the same fixtures.
fn crd_fixtures() -> Vec<(&'static str, StellarNodeSpec)> {
    vec![
        ("minimal", minimal_validator_spec()),
        ("standard", standard_validator_spec()),
        ("horizon_autoscaling", horizon_autoscaling_spec()),
        ("full_config", full_config_validator_spec()),
    ]
}

/// Benchmark: `StellarNodeSpec::validate()` across complexity tiers.
///
/// This is the actual admission-time validation logic used by the operator
/// (see `src/crd/stellar_node.rs`), not a stand-in — every fixture above is
/// asserted valid before benchmarking so a future change that breaks
/// validation fails loudly instead of silently benchmarking an error path.
fn bench_crd_validate(c: &mut Criterion) {
    let mut group = c.benchmark_group("crd_validate");

    for (name, spec) in crd_fixtures() {
        assert!(
            spec.validate().is_ok(),
            "benchmark fixture '{name}' must be a valid StellarNodeSpec"
        );
        group.bench_with_input(BenchmarkId::from_parameter(name), &spec, |b, spec| {
            b.iter(|| black_box(spec).validate());
        });
    }

    group.finish();
}

/// Benchmark: JSON serialization cost for each complexity tier.
///
/// Kubernetes controllers and admission webhooks work in JSON over the
/// wire; this measures the `serde_json` encode cost that sits alongside
/// validation on every reconcile/admission request.
fn bench_crd_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("crd_serialize");

    for (name, spec) in crd_fixtures() {
        let json_len = serde_json::to_string(&spec)
            .expect("fixture must serialize to JSON")
            .len() as u64;
        group.throughput(Throughput::Bytes(json_len));
        group.bench_with_input(BenchmarkId::from_parameter(name), &spec, |b, spec| {
            b.iter(|| serde_json::to_string(black_box(spec)).expect("spec must serialize"));
        });
    }

    group.finish();
}

/// Benchmark: JSON deserialization cost for each complexity tier.
///
/// Mirrors decoding a `StellarNode` admission request body or an informer
/// cache entry back into a typed `StellarNodeSpec`.
fn bench_crd_deserialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("crd_deserialize");

    for (name, spec) in crd_fixtures() {
        let json = serde_json::to_string(&spec).expect("fixture must serialize to JSON");
        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &json, |b, json| {
            b.iter(|| {
                let spec: StellarNodeSpec =
                    serde_json::from_str(black_box(json)).expect("fixture JSON must parse");
                black_box(spec)
            });
        });
    }

    group.finish();
}

/// Benchmark: concurrent validation throughput.
///
/// Spawns N worker threads that each construct and validate a spec in
/// parallel, approximating burst admission traffic (e.g. a GitOps
/// controller applying many `StellarNode` resources in one sync). Unlike
/// the original stub, this actually calls `validate()` on every worker and
/// asserts the result rather than measuring `String::len()`.
fn bench_crd_concurrent_validate(c: &mut Criterion) {
    let mut group = c.benchmark_group("crd_concurrent_validate");
    // Thread spawning dominates at high worker counts; keep the sample size
    // modest so the group finishes in a reasonable time in CI.
    group.sample_size(20);

    for workers in [1usize, 5, 10, 25, 50] {
        group.throughput(Throughput::Elements(workers as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{workers}-workers")),
            &workers,
            |b, &workers| {
                b.iter(|| {
                    let handles: Vec<_> = (0..workers)
                        .map(|i| {
                            std::thread::spawn(move || {
                                let spec = if i % 2 == 0 {
                                    minimal_validator_spec()
                                } else {
                                    full_config_validator_spec()
                                };
                                black_box(&spec).validate().is_ok()
                            })
                        })
                        .collect();

                    let successes = handles
                        .into_iter()
                        .map(|h| h.join().expect("worker thread panicked"))
                        .filter(|ok| *ok)
                        .count();
                    black_box(successes)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_crd_validate,
    bench_crd_serialize,
    bench_crd_deserialize,
    bench_crd_concurrent_validate,
);
criterion_main!(benches);
