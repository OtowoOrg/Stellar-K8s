//! Forensic snapshot: PCAP (~60s), optional core dump, encrypted upload to S3.
//!
//! Trigger with `metadata.annotations["stellar.org/request-forensic-snapshot"] = "true"` on a
//! [`StellarNode`] that defines [`crate::crd::types::ForensicSnapshotConfig`].
//! Requires Kubernetes 1.23+ ephemeral containers. Use `enableShareProcessNamespace` on the
//! forensic config for `gcore` of `stellar-core`.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::{Client, ResourceExt};
use serde_json::json;
use tracing::{info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

pub const REQUEST_FORENSIC_ANNOTATION: &str = "stellar.org/request-forensic-snapshot";
const FORENSIC_NAME_PREFIX: &str = "stellar-k8s-forensic-";

/// Reconcile one-shot forensic capture using an ephemeral container on the validator pod.
pub async fn reconcile_forensic_snapshot(client: &Client, node: &StellarNode) -> Result<()> {
    let cfg = match &node.spec.forensic_snapshot {
        Some(c) => c,
        None => return Ok(()),
    };

    let requested = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(REQUEST_FORENSIC_ANNOTATION))
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if !requested {
        return Ok(());
    }

    if node.spec.node_type != crate::crd::types::NodeType::Validator {
        warn!(
            node = %node.name_any(),
            "Forensic snapshot requested on non-validator; skipping"
        );
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let selector = format!("app.kubernetes.io/instance={}", node.name_any());
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(Error::KubeError)?;

    let pod = list
        .items
        .into_iter()
        .find(|p| {
            p.status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .map(|ph| ph == "Running")
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            Error::ConfigError(format!(
                "No Running pod for StellarNode {}/{}; cannot attach forensic capture",
                namespace,
                node.name_any()
            ))
        })?;

    let pod_name = pod.name_any();

    if let Some((done, ok)) = latest_forensic_ephemeral_state(&pod) {
        if done {
            patch_stellar_forensic_phase(
                client,
                &namespace,
                &node.name_any(),
                if ok { "Complete" } else { "Failed" },
            )
            .await?;
            clear_request_annotation(client, &namespace, &node.name_any()).await?;
            if ok {
                info!(
                    node = %node.name_any(),
                    pod = %pod_name,
                    "Forensic snapshot finished and uploaded to S3"
                );
            }
            return Ok(());
        }
        // Still running
        patch_stellar_forensic_phase(client, &namespace, &node.name_any(), "Capturing").await?;
        return Ok(());
    }

    let bucket = cfg.s3_bucket.clone();
    let prefix = cfg
        .s3_prefix
        .clone()
        .unwrap_or_else(|| format!("forensic/{}", node.name_any()));
    let kms_key = cfg.kms_key_id.clone().unwrap_or_default();

    let ec_name = format!("{}{}", FORENSIC_NAME_PREFIX, chrono::Utc::now().timestamp());
    let script = build_capture_script(&bucket, &prefix, &kms_key);
    let mut ec = json!({
        "name": ec_name,
        "image": "amazonlinux:2023",
        "imagePullPolicy": "IfNotPresent",
        "command": ["/bin/bash", "-c"],
        "args": [script],
        "securityContext": {
            "capabilities": {
                "add": ["NET_RAW", "SYS_PTRACE"]
            }
        },
        "resources": {
            "limits": { "cpu": "1", "memory": "1Gi" },
            "requests": { "cpu": "100m", "memory": "256Mi" }
        }
    });

    if let Some(secret) = &cfg.credentials_secret_ref {
        ec.as_object_mut().unwrap().insert(
            "envFrom".to_string(),
            json!([{
                "secretRef": { "name": secret }
            }]),
        );
    }

    let patch_body = json!({
        "spec": {
            "ephemeralContainers": [ec]
        }
    });

    pods.patch_ephemeral_containers(
        &pod_name,
        &PatchParams::apply("stellar-k8s").force(),
        &Patch::Strategic(patch_body),
    )
    .await
    .map_err(Error::KubeError)?;

    patch_stellar_forensic_phase(client, &namespace, &node.name_any(), "Capturing").await?;
    info!(
        node = %node.name_any(),
        pod = %pod_name,
        "Started forensic ephemeral capture (PCAP + optional core, S3 upload)"
    );
    Ok(())
}

/// Latest forensic ephemeral: `Some((done, success))` or `None` if none registered.
fn latest_forensic_ephemeral_state(pod: &Pod) -> Option<(bool, bool)> {
    let st = pod.status.as_ref()?;
    let ecs = st.ephemeral_container_statuses.as_ref()?;
    let ours: Vec<_> = ecs
        .iter()
        .filter(|c| c.name.starts_with(FORENSIC_NAME_PREFIX))
        .collect();
    let last = ours.last()?;
    let state = last.state.as_ref()?;
    if let Some(term) = &state.terminated {
        return Some((true, term.exit_code == 0));
    }
    Some((false, false))
}

fn build_capture_script(bucket: &str, prefix: &str, kms_key: &str) -> String {
    let aws_suffix = if kms_key.is_empty() {
        String::new()
    } else {
        format!(" --sse aws:kms --sse-kms-key-id {kms_key}")
    };
    format!(
        r#"set -euo pipefail
dnf install -y -q aws-cli tcpdump gdb procps-ng tar gzip which >/dev/null 2>&1 || true
OUT=/tmp/forensic-${{RANDOM}}.tgz
PCAP=/tmp/stellar-net.pcap
tcpdump -i any -w "$PCAP" 2>/dev/null &
TP=$!
sleep 60
kill $TP 2>/dev/null || true
wait $TP 2>/dev/null || true
PID=$(pgrep -x stellar-core | head -1 || true)
if [ -n "$PID" ]; then
  gcore -o /tmp/stellar-core-dump "$PID" 2>/dev/null || true
fi
tar czvf "$OUT" "$PCAP" /tmp/stellar-core-dump 2>/dev/null || tar czvf "$OUT" "$PCAP" 2>/dev/null || true
KEY="{prefix}/forensic-$(date -u +%Y%m%dT%H%M%SZ).tgz"
aws s3 cp "$OUT" "s3://{bucket}/$KEY"{aws_suffix}
echo "Uploaded s3://{bucket}/$KEY"
"#
    )
}

async fn patch_stellar_forensic_phase(
    client: &Client,
    namespace: &str,
    name: &str,
    phase: &str,
) -> Result<()> {
    let api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    let patch = json!({ "status": { "forensicSnapshotPhase": phase } });
    api.patch_status(
        name,
        &PatchParams::apply("stellar-k8s"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;
    Ok(())
}

async fn clear_request_annotation(client: &Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    let patch = json!({
        "metadata": {
            "annotations": {
                REQUEST_FORENSIC_ANNOTATION: null
            }
        }
    });
    api.patch(
        name,
        &PatchParams::apply("stellar-k8s"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;
    Ok(())
}
