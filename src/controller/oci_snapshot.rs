//! OCI-based Ledger Snapshot Sync
//!
//! Packages the contents of a validator/RPC node's data PVC into an OCI image
//! layer and pushes it to any OCI-compatible registry (GHCR, Docker Hub, etc.).
//! Pulling the image bootstraps a new node's PVC without waiting for a full
//! catchup, dramatically reducing cross-region spin-up time.
//!
//! ## How it works (push)
//! 1. The operator creates a one-shot Kubernetes Job (`<node>-snapshot-push-<ledger>`).
//! 2. The Job mounts the node PVC read-only and tars it into a temporary scratch volume.
//! 3. `crane push` (from `gcr.io/go-containerregistry/crane`) packages the tarball as
//!    an OCI image layer and pushes it to the registry.
//! 4. Registry credentials come from a K8s Secret mounted as `~/.docker/config.json`.
//!
//! ## How it works (pull)
//! 1. The operator creates a Job (`<node>-snapshot-pull`) before the node pod starts.
//! 2. The Job pulls the OCI image and extracts the layer tarball onto the node PVC.
//! 3. Once the Job succeeds the operator proceeds with normal node reconciliation.

use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PodSpec, PodTemplateSpec, ProjectedVolumeSource, SecretProjection, Volume,
    VolumeMount, VolumeProjection,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, PostParams};
use kube::{Client, ResourceExt};
use tracing::{debug, info};

use crate::controller::resources::{owner_reference, standard_labels};
use crate::crd::{OciSnapshotConfig, StellarNode, TagStrategy};
use crate::error::{Error, Result};

// Image used to run `crane` – Alpine-based, no Docker daemon required.
const CRANE_IMAGE: &str = "gcr.io/go-containerregistry/crane:latest";

// Path inside the Job pod where the node PVC is mounted.
const DATA_MOUNT_PATH: &str = "/data";

// Scratch volume for the intermediate tarball.
const SCRATCH_MOUNT_PATH: &str = "/scratch";

// Where the registry credential secret is projected.
const DOCKER_CONFIG_PATH: &str = "/root/.docker";

// ─── Tag helpers ─────────────────────────────────────────────────────────────

/// Resolve the OCI image tag according to the configured [`TagStrategy`].
pub fn resolve_tag(cfg: &OciSnapshotConfig, ledger_seq: u64) -> String {
    match cfg.tag_strategy {
        TagStrategy::LatestLedger => format!("snapshot-{ledger_seq}"),
        TagStrategy::Fixed => cfg
            .fixed_tag
            .clone()
            .unwrap_or_else(|| "latest".to_string()),
    }
}

/// Build the full `registry/image:tag` reference for the push target.
pub fn push_image_ref(cfg: &OciSnapshotConfig, ledger_seq: u64) -> String {
    let tag = resolve_tag(cfg, ledger_seq);
    format!("{}/{}:{}", cfg.registry, cfg.image, tag)
}

/// Resolve the full image reference to pull from.
///
/// If `cfg.pull_image_ref` is explicitly set it is used verbatim; otherwise the
/// reference is constructed from registry/image and the tag strategy.
pub fn pull_image_ref(cfg: &OciSnapshotConfig, ledger_seq: u64) -> String {
    cfg.pull_image_ref
        .clone()
        .unwrap_or_else(|| push_image_ref(cfg, ledger_seq))
}

// ─── Job name helpers ─────────────────────────────────────────────────────────

/// Kubernetes Job name for a push snapshot Job.
pub fn push_job_name(node: &StellarNode, ledger_seq: u64) -> String {
    // Keep the name within K8s 63-char limit by taking the first 40 chars of the node name.
    let node_name = &node.name_any()[..node.name_any().len().min(40)];
    format!("{node_name}-snap-push-{ledger_seq}")
}

/// Kubernetes Job name for a pull snapshot Job.
pub fn pull_job_name(node: &StellarNode) -> String {
    let node_name = &node.name_any()[..node.name_any().len().min(48)];
    format!("{node_name}-snap-pull")
}

// ─── Volume helpers ───────────────────────────────────────────────────────────

/// Build the credential secret projected volume.
///
/// The Secret must contain a `config.json` key holding Docker credential JSON.
/// It is mounted at `/root/.docker/config.json` inside the Job pod.
fn credential_volume(secret_name: &str) -> Volume {
    Volume {
        name: "docker-credentials".to_string(),
        projected: Some(ProjectedVolumeSource {
            sources: Some(vec![VolumeProjection {
                secret: Some(SecretProjection {
                    name: Some(secret_name.to_string()),
                    items: Some(vec![k8s_openapi::api::core::v1::KeyToPath {
                        key: "config.json".to_string(),
                        path: "config.json".to_string(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build the PVC volume for the node's data directory.
fn pvc_volume(pvc_name: &str) -> Volume {
    Volume {
        name: "node-data".to_string(),
        persistent_volume_claim: Some(
            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                claim_name: pvc_name.to_string(),
                read_only: Some(false),
            },
        ),
        ..Default::default()
    }
}

/// Build a read-only PVC volume for use in push Jobs.
fn pvc_volume_readonly(pvc_name: &str) -> Volume {
    Volume {
        name: "node-data".to_string(),
        persistent_volume_claim: Some(
            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                claim_name: pvc_name.to_string(),
                read_only: Some(true),
            },
        ),
        ..Default::default()
    }
}

/// Build an `emptyDir` scratch volume for intermediate tarballs.
fn scratch_volume() -> Volume {
    Volume {
        name: "scratch".to_string(),
        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ─── Job builders ─────────────────────────────────────────────────────────────

/// Build a push snapshot Job.
///
/// The Job:
/// 1. Tars the PVC contents into `/scratch/snapshot.tar`.
/// 2. Calls `crane push /scratch/snapshot.tar <image_ref>`.
///
/// # Arguments
/// * `node` – StellarNode resource (for labels, owner reference, PVC name)
/// * `cfg` – OCI snapshot configuration
/// * `ledger_seq` – current ledger sequence used to generate the image tag
pub fn build_snapshot_push_job(
    node: &StellarNode,
    cfg: &OciSnapshotConfig,
    ledger_seq: u64,
) -> Job {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let job_name = push_job_name(node, ledger_seq);
    let image_ref = push_image_ref(cfg, ledger_seq);
    let pvc_name = format!("{}-data", node.name_any());

    let mut labels = standard_labels(node);
    labels.insert(
        "stellar.org/job-type".to_string(),
        "snapshot-push".to_string(),
    );

    let container = Container {
        name: "snapshot-push".to_string(),
        image: Some(CRANE_IMAGE.to_string()),
        command: Some(vec!["sh".to_string(), "-c".to_string()]),
        args: Some(vec![format!(
            "set -e; \
             echo 'Packaging ledger snapshot...'; \
             tar -czf {SCRATCH_MOUNT_PATH}/snapshot.tar.gz -C {DATA_MOUNT_PATH} .; \
             echo 'Pushing to OCI registry: {image_ref}'; \
             crane push {SCRATCH_MOUNT_PATH}/snapshot.tar.gz {image_ref}; \
             echo 'Push complete.'"
        )]),
        env: Some(vec![EnvVar {
            name: "DOCKER_CONFIG".to_string(),
            value: Some(DOCKER_CONFIG_PATH.to_string()),
            ..Default::default()
        }]),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "node-data".to_string(),
                mount_path: DATA_MOUNT_PATH.to_string(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: "scratch".to_string(),
                mount_path: SCRATCH_MOUNT_PATH.to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "docker-credentials".to_string(),
                mount_path: DOCKER_CONFIG_PATH.to_string(),
                read_only: Some(true),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    Job {
        metadata: ObjectMeta {
            name: Some(job_name),
            namespace: Some(namespace),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(3),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(standard_labels(node)),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("OnFailure".to_string()),
                    containers: vec![container],
                    volumes: Some(vec![
                        pvc_volume_readonly(&pvc_name),
                        scratch_volume(),
                        credential_volume(&cfg.credential_secret_name),
                    ]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    }
}

/// Build a pull snapshot Job.
///
/// The Job:
/// 1. Calls `crane pull <image_ref> /scratch/snapshot.tar`.
/// 2. Extracts the tarball into the node PVC at `/data`.
///
/// # Arguments
/// * `node` – StellarNode resource
/// * `cfg` – OCI snapshot configuration
/// * `ledger_seq` – used when constructing the pull image ref from `tag_strategy`
pub fn build_snapshot_pull_job(
    node: &StellarNode,
    cfg: &OciSnapshotConfig,
    ledger_seq: u64,
) -> Job {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let job_name = pull_job_name(node);
    let image_ref = pull_image_ref(cfg, ledger_seq);
    let pvc_name = format!("{}-data", node.name_any());

    let mut labels = standard_labels(node);
    labels.insert(
        "stellar.org/job-type".to_string(),
        "snapshot-pull".to_string(),
    );

    let container = Container {
        name: "snapshot-pull".to_string(),
        image: Some(CRANE_IMAGE.to_string()),
        command: Some(vec!["sh".to_string(), "-c".to_string()]),
        args: Some(vec![format!(
            "set -e; \
             echo 'Pulling OCI snapshot: {image_ref}'; \
             crane pull {image_ref} {SCRATCH_MOUNT_PATH}/snapshot.tar; \
             echo 'Extracting snapshot to {DATA_MOUNT_PATH}'; \
             tar -xf {SCRATCH_MOUNT_PATH}/snapshot.tar -C {DATA_MOUNT_PATH}; \
             echo 'Pull complete.'"
        )]),
        env: Some(vec![EnvVar {
            name: "DOCKER_CONFIG".to_string(),
            value: Some(DOCKER_CONFIG_PATH.to_string()),
            ..Default::default()
        }]),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "node-data".to_string(),
                mount_path: DATA_MOUNT_PATH.to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "scratch".to_string(),
                mount_path: SCRATCH_MOUNT_PATH.to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "docker-credentials".to_string(),
                mount_path: DOCKER_CONFIG_PATH.to_string(),
                read_only: Some(true),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    Job {
        metadata: ObjectMeta {
            name: Some(job_name),
            namespace: Some(namespace),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(3),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(standard_labels(node)),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("OnFailure".to_string()),
                    containers: vec![container],
                    volumes: Some(vec![
                        pvc_volume(&pvc_name),
                        scratch_volume(),
                        credential_volume(&cfg.credential_secret_name),
                    ]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    }
}

// ─── Idempotent Job creation ──────────────────────────────────────────────────

/// Idempotently create the snapshot push Job.
///
/// If a Job with the same name already exists it is left unchanged (skip).
/// Returns the Job name on success.
pub async fn ensure_snapshot_push_job(
    client: &Client,
    node: &StellarNode,
    cfg: &OciSnapshotConfig,
    ledger_seq: u64,
) -> Result<String> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let job = build_snapshot_push_job(node, cfg, ledger_seq);
    let job_name = job.metadata.name.clone().unwrap_or_default();

    match api.get(&job_name).await {
        Ok(_) => {
            debug!("Snapshot push Job {} already exists, skipping", job_name);
            Ok(job_name)
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("Creating snapshot push Job {}", job_name);
            api.create(&PostParams::default(), &job)
                .await
                .map_err(Error::KubeError)?;
            Ok(job_name)
        }
        Err(e) => Err(Error::KubeError(e)),
    }
}

/// Idempotently create the snapshot pull Job.
///
/// Returns the Job name on success.
pub async fn ensure_snapshot_pull_job(
    client: &Client,
    node: &StellarNode,
    cfg: &OciSnapshotConfig,
    ledger_seq: u64,
) -> Result<String> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let job = build_snapshot_pull_job(node, cfg, ledger_seq);
    let job_name = job.metadata.name.clone().unwrap_or_default();

    match api.get(&job_name).await {
        Ok(_) => {
            debug!("Snapshot pull Job {} already exists, skipping", job_name);
            Ok(job_name)
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("Creating snapshot pull Job {}", job_name);
            api.create(&PostParams::default(), &job)
                .await
                .map_err(Error::KubeError)?;
            Ok(job_name)
        }
        Err(e) => Err(Error::KubeError(e)),
    }
}

/// Check whether a snapshot Job has completed successfully.
///
/// Returns `true` if `status.succeeded >= 1`.
pub async fn is_snapshot_job_done(
    client: &Client,
    node: &StellarNode,
    job_name: &str,
) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);

    match api.get(job_name).await {
        Ok(job) => {
            let succeeded = job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0);
            Ok(succeeded >= 1)
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
        Err(e) => Err(Error::KubeError(e)),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        HistoryMode, NodeType, OciSnapshotConfig, ResourceRequirements, RolloutStrategy,
        StellarNetwork, StellarNode, StellarNodeSpec, StorageConfig, TagStrategy, ValidatorConfig,
    };

    fn test_cfg(tag_strategy: TagStrategy, fixed_tag: Option<&str>) -> OciSnapshotConfig {
        OciSnapshotConfig {
            enabled: true,
            registry: "ghcr.io".to_string(),
            image: "myorg/stellar-snapshot".to_string(),
            tag_strategy,
            fixed_tag: fixed_tag.map(|s| s.to_string()),
            credential_secret_name: "registry-creds".to_string(),
            push: true,
            pull: false,
            pull_image_ref: None,
        }
    }

    fn make_node(name: &str) -> StellarNode {
        StellarNode {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                uid: Some("test-uid-1234".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21".to_string(),
                history_mode: HistoryMode::Recent,
                resources: ResourceRequirements::default(),
                storage: StorageConfig::default(),
                validator_config: Some(ValidatorConfig {
                    seed_secret_ref: "test-seed-secret".to_string(),
                    seed_secret_source: Default::default(),
                    enable_history_archive: false,
                    history_archive_urls: vec![],
                    quorum_set: None,
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
                vpa_config: None,
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
                read_replica_config: None,
                oci_snapshot: None,
                service_mesh: None,
                read_pool_endpoint: None,
                resource_meta: None,
            },
            status: None,
        }
    }

    // ── Tag strategy ──────────────────────────────────────────────────────────

    #[test]
    fn test_tag_strategy_latest_ledger() {
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        assert_eq!(resolve_tag(&cfg, 12_345_678), "snapshot-12345678");
    }

    #[test]
    fn test_tag_strategy_fixed_explicit() {
        let cfg = test_cfg(TagStrategy::Fixed, Some("stable"));
        assert_eq!(resolve_tag(&cfg, 999), "stable");
    }

    #[test]
    fn test_tag_strategy_fixed_fallback_to_latest() {
        let cfg = test_cfg(TagStrategy::Fixed, None);
        assert_eq!(resolve_tag(&cfg, 1), "latest");
    }

    #[test]
    fn test_push_image_ref_format() {
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        assert_eq!(
            push_image_ref(&cfg, 42),
            "ghcr.io/myorg/stellar-snapshot:snapshot-42"
        );
    }

    #[test]
    fn test_pull_image_ref_explicit_overrides() {
        let mut cfg = test_cfg(TagStrategy::LatestLedger, None);
        cfg.pull_image_ref = Some("ghcr.io/other/image:v1".to_string());
        assert_eq!(pull_image_ref(&cfg, 99), "ghcr.io/other/image:v1");
    }

    #[test]
    fn test_pull_image_ref_falls_back_to_push_ref() {
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        assert_eq!(
            pull_image_ref(&cfg, 50),
            "ghcr.io/myorg/stellar-snapshot:snapshot-50"
        );
    }

    // ── Push Job structure ────────────────────────────────────────────────────

    #[test]
    fn test_build_push_job_has_crane_image() {
        let node = make_node("my-validator");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1000);
        let container = &job.spec.unwrap().template.spec.unwrap().containers[0];
        assert_eq!(container.image.as_deref(), Some(CRANE_IMAGE));
    }

    #[test]
    fn test_build_push_job_name_contains_ledger() {
        let node = make_node("validator-a");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 7777);
        assert!(job.metadata.name.as_deref().unwrap().contains("7777"));
    }

    #[test]
    fn test_build_push_job_has_pvc_volume() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1);
        let volumes = job
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .volumes
            .unwrap_or_default();
        assert!(
            volumes.iter().any(|v| v.name == "node-data"),
            "push Job must have a node-data PVC volume"
        );
    }

    #[test]
    fn test_build_push_job_mounts_credentials() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1);
        let spec = job.spec.unwrap().template.spec.unwrap();
        let volumes = spec.volumes.unwrap_or_default();
        assert!(
            volumes.iter().any(|v| v.name == "docker-credentials"),
            "push Job must mount the docker-credentials projected volume"
        );
        let mounts = spec.containers[0].volume_mounts.clone().unwrap_or_default();
        assert!(
            mounts.iter().any(|m| m.name == "docker-credentials"),
            "push container must mount docker-credentials"
        );
    }

    #[test]
    fn test_build_push_job_has_scratch_volume() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1);
        let volumes = job
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .volumes
            .unwrap_or_default();
        assert!(volumes.iter().any(|v| v.name == "scratch"));
    }

    #[test]
    fn test_build_push_job_owner_reference() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1);
        let owners = job.metadata.owner_references.unwrap_or_default();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].name, "my-node");
    }

    #[test]
    fn test_build_push_job_pvc_read_only() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1);
        let spec = job.spec.unwrap().template.spec.unwrap();
        let mounts = spec.containers[0].volume_mounts.clone().unwrap_or_default();
        let data_mount = mounts.iter().find(|m| m.name == "node-data").unwrap();
        assert_eq!(
            data_mount.read_only,
            Some(true),
            "push Job should mount PVC read-only"
        );
    }

    // ── Pull Job structure ────────────────────────────────────────────────────

    #[test]
    fn test_build_pull_job_targets_correct_image() {
        let node = make_node("bootstrap-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_pull_job(&node, &cfg, 555);
        let spec = job.spec.unwrap().template.spec.unwrap();
        let args = spec.containers[0].args.clone().unwrap_or_default();
        assert!(
            args.iter().any(|a| a.contains("snapshot-555")),
            "pull Job must reference the resolved image tag"
        );
    }

    #[test]
    fn test_build_pull_job_pvc_writable() {
        let node = make_node("bootstrap-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_pull_job(&node, &cfg, 1);
        let spec = job.spec.unwrap().template.spec.unwrap();
        let mounts = spec.containers[0].volume_mounts.clone().unwrap_or_default();
        let data_mount = mounts.iter().find(|m| m.name == "node-data").unwrap();
        // read_only should be None or false for pull Jobs
        assert!(
            data_mount.read_only != Some(true),
            "pull Job should mount PVC writable"
        );
    }

    #[test]
    fn test_build_pull_job_owner_reference() {
        let node = make_node("another-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_pull_job(&node, &cfg, 1);
        let owners = job.metadata.owner_references.unwrap_or_default();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].name, "another-node");
    }

    #[test]
    fn test_build_pull_job_mounts_credentials() {
        let node = make_node("bootstrap-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_pull_job(&node, &cfg, 1);
        let spec = job.spec.unwrap().template.spec.unwrap();
        let volumes = spec.volumes.unwrap_or_default();
        assert!(
            volumes.iter().any(|v| v.name == "docker-credentials"),
            "pull Job must mount credentials"
        );
    }

    #[test]
    fn test_job_backoff_limit() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let push = build_snapshot_push_job(&node, &cfg, 1);
        let pull = build_snapshot_pull_job(&node, &cfg, 1);
        assert_eq!(push.spec.as_ref().unwrap().backoff_limit, Some(3));
        assert_eq!(pull.spec.as_ref().unwrap().backoff_limit, Some(3));
    }

    #[test]
    fn test_push_job_restart_policy_on_failure() {
        let node = make_node("my-node");
        let cfg = test_cfg(TagStrategy::LatestLedger, None);
        let job = build_snapshot_push_job(&node, &cfg, 1);
        let restart = job.spec.unwrap().template.spec.unwrap().restart_policy;
        assert_eq!(restart.as_deref(), Some("OnFailure"));
    }
}
