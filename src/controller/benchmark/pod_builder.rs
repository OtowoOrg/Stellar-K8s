//! Ephemeral load-generator pod builder
//!
//! Constructs the Kubernetes `Pod` manifests for benchmark load-generator pods.
//! Each pod runs the configured load-generator image and is given a unique name
//! derived from the `StellarBenchmark` name and a pod index.
//!
//! Pods are created with:
//! - `restartPolicy: Never` (ephemeral, run-once semantics)
//! - Owner references pointing back to the `StellarBenchmark` resource so they
//!   are garbage-collected when the benchmark is deleted.
//! - Resource requests/limits from the benchmark spec.
//! - Standard environment variables consumed by the load-generator image.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    Container, EnvVar as K8sEnvVar, Pod, PodSpec, ResourceRequirements as K8sResourceRequirements,
    SecretEnvSource,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::{Resource, ResourceExt};
use tracing::debug;

use crate::crd::stellar_benchmark::{StellarBenchmark, StellarBenchmarkSpec};

/// Label applied to every load-generator pod so the controller can list them.
pub const BENCHMARK_POD_LABEL: &str = "stellar.org/benchmark";

/// Label value that identifies the pod as a load-generator.
pub const LOAD_GENERATOR_COMPONENT: &str = "load-generator";

/// Build the name for the `n`-th load-generator pod.
pub fn pod_name(benchmark_name: &str, index: u32) -> String {
    format!("{}-loadgen-{}", benchmark_name, index)
}

/// Build a `Pod` manifest for a single load-generator instance.
///
/// # Arguments
///
/// * `benchmark` – the owning `StellarBenchmark` resource.
/// * `index` – zero-based pod index (used to derive the pod name).
pub fn build_load_generator_pod(benchmark: &StellarBenchmark, index: u32) -> Pod {
    let spec = &benchmark.spec;
    let name = pod_name(&benchmark.name_any(), index);
    let namespace = benchmark
        .namespace()
        .unwrap_or_else(|| "default".to_string());

    let owner_ref = owner_reference(benchmark);
    let labels = pod_labels(&benchmark.name_any());

    let env = build_env(spec, index);
    let resources = build_resources(spec);

    // Tolerations
    let tolerations: Option<Vec<k8s_openapi::api::core::v1::Toleration>> =
        if spec.tolerations.is_empty() {
            None
        } else {
            Some(
                spec.tolerations
                    .iter()
                    .map(|t| k8s_openapi::api::core::v1::Toleration {
                        key: Some(t.key.clone()),
                        operator: t.operator.clone(),
                        value: t.value.clone(),
                        effect: t.effect.clone(),
                        toleration_seconds: None,
                    })
                    .collect(),
            )
        };

    // Node selector
    let node_selector = if spec.node_selector.is_empty() {
        None
    } else {
        Some(spec.node_selector.clone())
    };

    let container = Container {
        name: "load-generator".to_string(),
        image: Some(spec.load_generator_image.clone()),
        image_pull_policy: Some(spec.image_pull_policy.clone()),
        env: Some(env),
        resources: Some(resources),
        // The load-generator writes its results to stdout in JSON format.
        // The collector reads them via the Kubernetes logs API.
        ..Default::default()
    };

    debug!(
        pod_name = %name,
        benchmark = %benchmark.name_any(),
        index = index,
        "Building load-generator pod"
    );

    Pod {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace),
            labels: Some(labels),
            owner_references: Some(vec![owner_ref]),
            annotations: Some(BTreeMap::from([(
                "stellar.org/benchmark-index".to_string(),
                index.to_string(),
            )])),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![container],
            restart_policy: Some("Never".to_string()),
            service_account_name: spec.service_account_name.clone(),
            node_selector,
            tolerations,
            // Terminate the pod promptly once the load-generator exits.
            termination_grace_period_seconds: Some(10),
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn owner_reference(benchmark: &StellarBenchmark) -> OwnerReference {
    OwnerReference {
        api_version: StellarBenchmark::api_version(&()).to_string(),
        kind: StellarBenchmark::kind(&()).to_string(),
        name: benchmark.name_any(),
        uid: benchmark.metadata.uid.clone().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }
}

fn pod_labels(benchmark_name: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (BENCHMARK_POD_LABEL.to_string(), benchmark_name.to_string()),
        (
            "app.kubernetes.io/component".to_string(),
            LOAD_GENERATOR_COMPONENT.to_string(),
        ),
        (
            "app.kubernetes.io/managed-by".to_string(),
            "stellar-operator".to_string(),
        ),
    ])
}

fn build_env(spec: &StellarBenchmarkSpec, index: u32) -> Vec<K8sEnvVar> {
    let mut env = vec![
        K8sEnvVar {
            name: "TARGET_ENDPOINT".to_string(),
            value: Some(spec.target_endpoint.clone()),
            ..Default::default()
        },
        K8sEnvVar {
            name: "DURATION_SECONDS".to_string(),
            value: Some(spec.duration_seconds.to_string()),
            ..Default::default()
        },
        K8sEnvVar {
            name: "TARGET_TPS".to_string(),
            // Each pod handles its share of the total TPS.
            value: Some((spec.target_tps / spec.concurrency.max(1)).to_string()),
            ..Default::default()
        },
        K8sEnvVar {
            name: "NETWORK_PASSPHRASE".to_string(),
            value: Some(spec.network_passphrase.clone()),
            ..Default::default()
        },
        K8sEnvVar {
            name: "POD_INDEX".to_string(),
            value: Some(index.to_string()),
            ..Default::default()
        },
        K8sEnvVar {
            name: "CONCURRENCY".to_string(),
            value: Some(spec.concurrency.to_string()),
            ..Default::default()
        },
    ];

    // Append caller-supplied extra env vars.
    for ev in &spec.extra_env {
        env.push(K8sEnvVar {
            name: ev.name.clone(),
            value: Some(ev.value.clone()),
            ..Default::default()
        });
    }

    env
}

/// Build the pod with envFrom for the secret ref (separate function so we can
/// attach it to the container after the fact).
pub fn build_load_generator_pod_with_secret(benchmark: &StellarBenchmark, index: u32) -> Pod {
    let mut pod = build_load_generator_pod(benchmark, index);

    if let Some(secret_name) = &benchmark.spec.secret_ref {
        if let Some(spec) = pod.spec.as_mut() {
            if let Some(container) = spec.containers.first_mut() {
                container.env_from = Some(vec![k8s_openapi::api::core::v1::EnvFromSource {
                    secret_ref: Some(SecretEnvSource {
                        name: Some(secret_name.clone()),
                        optional: Some(false),
                    }),
                    config_map_ref: None,
                    prefix: None,
                }]);
            }
        }
    }

    pod
}

fn build_resources(spec: &StellarBenchmarkSpec) -> K8sResourceRequirements {
    let r = &spec.resources;
    let mut requests = BTreeMap::new();
    requests.insert("cpu".to_string(), Quantity(r.cpu_request.clone()));
    requests.insert("memory".to_string(), Quantity(r.memory_request.clone()));

    let mut limits = BTreeMap::new();
    limits.insert("cpu".to_string(), Quantity(r.cpu_limit.clone()));
    limits.insert("memory".to_string(), Quantity(r.memory_limit.clone()));

    K8sResourceRequirements {
        requests: Some(requests),
        limits: Some(limits),
        claims: None,
    }
}
