use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PersistentVolumeClaim, PersistentVolumeClaimSpec,
    PersistentVolumeClaimVolumeSource, Pod, PodSpec, PodTemplateSpec, Volume, VolumeMount,
    VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::api::core::v1::TypedLocalObjectReference;
use kube::api::{Api, DynamicObject, ListParams, PostParams};
use kube::discovery::ApiResource;
use kube::{Client, ResourceExt};

use crate::controller::resources::{owner_reference, standard_labels};
use crate::crd::{LedgerSnapshotExportConfig, StellarNode};
use crate::error::{Error, Result};

const EXPORT_IMAGE: &str = "amazon/aws-cli:latest";

fn volume_snapshot_api_resource() -> ApiResource {
    ApiResource {
        group: "snapshot.storage.k8s.io".to_string(),
        version: "v1".to_string(),
        api_version: "snapshot.storage.k8s.io/v1".to_string(),
        kind: "VolumeSnapshot".to_string(),
        plural: "volumesnapshots".to_string(),
    }
}

fn export_resource_names(node: &StellarNode, ledger_sequence: u64) -> (String, String) {
    let node_name = node.name_any();
    let prefix = &node_name[..node_name.len().min(20)];
    (
        format!("{prefix}-export-snap-{ledger_sequence}"),
        format!("{prefix}-export-data-{ledger_sequence}"),
    )
}

/// Build a one-shot S3 export job backed by a CSI snapshot clone of the suspended source PVC.
pub fn build_export_job(
    node: &StellarNode,
    config: &LedgerSnapshotExportConfig,
    ledger_sequence: u64,
    source_pvc_name: &str,
) -> Job {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let job_name = format!(
        "ledger-export-{}-{ledger_sequence}",
        &node_name[..node_name.len().min(24)]
    );
    let mut labels = standard_labels(node);
    labels.insert(
        "stellar.org/job-type".to_string(),
        "ledger-export".to_string(),
    );

    let script = r#"set -euo pipefail
mkdir -p /scratch/manifest
cd /data
find . -type f -print0 | sort -z | xargs -0 sha256sum > /scratch/manifest/files.sha256
printf 'ledger_sequence=%s\nnetwork=%s\n' "$LEDGER_SEQUENCE" "$NETWORK" > /scratch/manifest/snapshot-manifest.txt
ARCHIVE="/scratch/ledger-state-${LEDGER_SEQUENCE}.tar.gz"
tar -czf "$ARCHIVE" -C /data . -C /scratch/manifest files.sha256 snapshot-manifest.txt
sha256sum "$ARCHIVE" > "$ARCHIVE.sha256"
DESTINATION="${SNAPSHOT_DESTINATION%/}"
aws s3 cp "$ARCHIVE" "$DESTINATION/$(basename "$ARCHIVE")"
aws s3 cp "$ARCHIVE.sha256" "$DESTINATION/$(basename "$ARCHIVE").sha256"
echo "Exported ledger $LEDGER_SEQUENCE to $DESTINATION; SHA256: $(cut -d ' ' -f 1 "$ARCHIVE.sha256")"
"#;

    let container = Container {
        name: "ledger-export".to_string(),
        image: Some(EXPORT_IMAGE.to_string()),
        command: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
        args: Some(vec![script.to_string()]),
        env: Some(vec![
            EnvVar {
                name: "SNAPSHOT_DESTINATION".to_string(),
                value: Some(config.destination.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "LEDGER_SEQUENCE".to_string(),
                value: Some(ledger_sequence.to_string()),
                ..Default::default()
            },
            EnvVar {
                name: "NETWORK".to_string(),
                value: Some(node.spec.network.to_string()),
                ..Default::default()
            },
            EnvVar {
                name: "AWS_ACCESS_KEY_ID".to_string(),
                value: None,
                value_from: Some(secret_key(
                    &config.credentials_secret_ref,
                    "AWS_ACCESS_KEY_ID",
                )),
            },
            EnvVar {
                name: "AWS_SECRET_ACCESS_KEY".to_string(),
                value: None,
                value_from: Some(secret_key(
                    &config.credentials_secret_ref,
                    "AWS_SECRET_ACCESS_KEY",
                )),
            },
            EnvVar {
                name: "AWS_DEFAULT_REGION".to_string(),
                value: None,
                value_from: Some(secret_key(
                    &config.credentials_secret_ref,
                    "AWS_DEFAULT_REGION",
                )),
            },
        ]),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "node-data".to_string(),
                mount_path: "/data".to_string(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: "scratch".to_string(),
                mount_path: "/scratch".to_string(),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    let mut pod_labels = standard_labels(node);
    pod_labels.insert(
        "stellar.org/job-type".to_string(),
        "ledger-export".to_string(),
    );

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
                    labels: Some(pod_labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("OnFailure".to_string()),
                    containers: vec![container],
                    volumes: Some(vec![
                        Volume {
                            name: "node-data".to_string(),
                            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                                claim_name: source_pvc_name.to_string(),
                                read_only: Some(true),
                            }),
                            ..Default::default()
                        },
                        Volume {
                            name: "scratch".to_string(),
                            empty_dir: Some(Default::default()),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    }
}

fn secret_key(secret_name: &str, key: &str) -> k8s_openapi::api::core::v1::EnvVarSource {
    k8s_openapi::api::core::v1::EnvVarSource {
        secret_key_ref: Some(k8s_openapi::api::core::v1::SecretKeySelector {
            name: Some(secret_name.to_string()),
            key: key.to_string(),
            optional: Some(key == "AWS_DEFAULT_REGION"),
        }),
        ..Default::default()
    }
}

/// Create the export job once for a source ledger sequence.
pub async fn ensure_export_job(
    client: &Client,
    node: &StellarNode,
    config: &LedgerSnapshotExportConfig,
    ledger_sequence: u64,
) -> Result<Option<String>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let selector = format!(
        "app.kubernetes.io/instance={},stellar.org/job-type!=ledger-export",
        node.name_any()
    );
    let active_node_pods = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(Error::KubeError)?
        .items
        .into_iter()
        .filter(|pod| {
            !matches!(
                pod.status
                    .as_ref()
                    .and_then(|status| status.phase.as_deref()),
                Some("Succeeded" | "Failed")
            )
        })
        .count();
    if active_node_pods > 0 {
        return Err(Error::ConfigError(format!(
            "Cannot export ledger state for {}/{} while a node pod is still active",
            namespace,
            node.name_any()
        )));
    }

    let (snapshot_name, export_pvc_name) = export_resource_names(node, ledger_sequence);
    if !ensure_export_volume_snapshot(client, node, &namespace, &snapshot_name).await? {
        return Ok(None);
    }
    ensure_export_pvc(client, node, &namespace, &snapshot_name, &export_pvc_name).await?;

    let api: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let job = build_export_job(node, config, ledger_sequence, &export_pvc_name);
    let name = job.metadata.name.clone().unwrap_or_default();
    match api.get(&name).await {
        Ok(_) => Ok(Some(name)),
        Err(kube::Error::Api(error)) if error.code == 404 => {
            api.create(&PostParams::default(), &job)
                .await
                .map_err(Error::KubeError)?;
            Ok(Some(name))
        }
        Err(error) => Err(Error::KubeError(error)),
    }
}

async fn ensure_export_volume_snapshot(
    client: &Client,
    node: &StellarNode,
    namespace: &str,
    snapshot_name: &str,
) -> Result<bool> {
    let api_resource = volume_snapshot_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &api_resource);
    match api.get(snapshot_name).await {
        Ok(snapshot) => Ok(snapshot
            .data
            .get("status")
            .and_then(|status| status.get("readyToUse"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)),
        Err(kube::Error::Api(error)) if error.code == 404 => {
            let snapshot = DynamicObject {
                types: Some(kube::core::TypeMeta {
                    api_version: api_resource.api_version.clone(),
                    kind: api_resource.kind.clone(),
                }),
                metadata: ObjectMeta {
                    name: Some(snapshot_name.to_string()),
                    namespace: Some(namespace.to_string()),
                    labels: Some(standard_labels(node)),
                    owner_references: Some(vec![owner_reference(node)]),
                    ..Default::default()
                },
                data: serde_json::json!({
                    "spec": {
                        "source": { "persistentVolumeClaimName": format!("{}-data", node.name_any()) }
                    }
                }),
            };
            api.create(&PostParams::default(), &snapshot)
                .await
                .map_err(Error::KubeError)?;
            Ok(false)
        }
        Err(error) => Err(Error::KubeError(error)),
    }
}

async fn ensure_export_pvc(
    client: &Client,
    node: &StellarNode,
    namespace: &str,
    snapshot_name: &str,
    pvc_name: &str,
) -> Result<()> {
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    match api.get(pvc_name).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(error)) if error.code == 404 => {
            let mut requests = std::collections::BTreeMap::new();
            requests.insert(
                "storage".to_string(),
                Quantity(node.spec.storage.size.clone()),
            );
            let pvc = PersistentVolumeClaim {
                metadata: ObjectMeta {
                    name: Some(pvc_name.to_string()),
                    namespace: Some(namespace.to_string()),
                    labels: Some(standard_labels(node)),
                    owner_references: Some(vec![owner_reference(node)]),
                    ..Default::default()
                },
                spec: Some(PersistentVolumeClaimSpec {
                    access_modes: Some(vec!["ReadWriteOnce".to_string()]),
                    storage_class_name: Some(node.spec.storage.storage_class.clone()),
                    data_source: Some(TypedLocalObjectReference {
                        api_group: Some("snapshot.storage.k8s.io".to_string()),
                        kind: "VolumeSnapshot".to_string(),
                        name: snapshot_name.to_string(),
                    }),
                    resources: Some(VolumeResourceRequirements {
                        requests: Some(requests),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            api.create(&PostParams::default(), &pvc)
                .await
                .map_err(Error::KubeError)?;
            Ok(())
        }
        Err(error) => Err(Error::KubeError(error)),
    }
}
