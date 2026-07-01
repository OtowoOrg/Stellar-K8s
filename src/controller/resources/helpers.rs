//! Shared labels and naming helpers.

use super::prelude::*;

pub(crate) const DIAGNOSTIC_SIDECAR_DEFAULT_CPU: &str = "50m";
pub(crate) const DIAGNOSTIC_SIDECAR_DEFAULT_MEMORY: &str = "64Mi";

/// Get the standard labels for a StellarNode's resources
pub(crate) fn standard_labels(node: &StellarNode) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "stellar-node".to_string(),
    );
    labels.insert("app.kubernetes.io/instance".to_string(), node.name_any());
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        node.spec.node_type.to_string().to_lowercase(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "stellar-operator".to_string(),
    );
    labels.insert(
        "stellar.org/node-type".to_string(),
        node.spec.node_type.to_string(),
    );
    labels.insert(
        "stellar-network".to_string(),
        node.spec
            .network
            .scheduling_label_value(&node.spec.custom_network_passphrase),
    );
    labels
}

fn render_annotation_template(value: &str, node: &StellarNode) -> String {
    let mut rendered = value.replace("{{name}}", &node.name_any());
    rendered = rendered.replace("${name}", &node.name_any());
    rendered = rendered.replace("{{namespace}}", &node.namespace().unwrap_or_default());
    rendered = rendered.replace("${namespace}", &node.namespace().unwrap_or_default());
    rendered = rendered.replace("{{nodeType}}", &node.spec.node_type.to_string());
    rendered = rendered.replace("${nodeType}", &node.spec.node_type.to_string());
    rendered = rendered.replace("{{network}}", &node.spec.network.to_string());
    rendered = rendered.replace("${network}", &node.spec.network.to_string());
    rendered
}

pub(crate) fn merge_service_annotations(
    annotations: &mut BTreeMap<String, String>,
    node: &StellarNode,
) {
    if let Some(service_annotations) = &node.spec.service_annotations {
        for (key, value) in service_annotations {
            annotations
                .entry(key.clone())
                .or_insert_with(|| render_annotation_template(value, node));
        }
    }
}

pub(crate) fn merge_service_metadata_labels(
    labels: &mut BTreeMap<String, String>,
    node: &StellarNode,
) {
    if let Some(service_labels) = &node.spec.service_labels {
        for (key, value) in service_labels {
            labels.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

/// Create an OwnerReference for garbage collection
pub(crate) fn owner_reference(node: &StellarNode) -> OwnerReference {
    OwnerReference {
        api_version: StellarNode::api_version(&()).to_string(),
        kind: StellarNode::kind(&()).to_string(),
        name: node.name_any(),
        uid: node.metadata.uid.clone().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }
}

/// Build the resource name for a given component
pub(crate) fn resource_name(node: &StellarNode, suffix: &str) -> String {
    format!("{}-{}", node.name_any(), suffix)
}

/// Apply a [`ProbeOverride`] on top of an optional base [`k8s_openapi::api::core::v1::Probe`].
/// Apply a [`ProbeOverride`] on top of an optional base [`k8s_openapi::api::core::v1::Probe`].
///
/// If `override_cfg` is `None`, the base probe is returned unchanged.
/// If `base` is `None` and `override_cfg` is `Some`, a minimal probe shell is created and the
/// overrides are applied so the operator can still honour user-supplied thresholds even when no
/// default probe is configured.
pub(crate) fn apply_probe_override_pub(
    base: Option<k8s_openapi::api::core::v1::Probe>,
    override_cfg: Option<&crate::crd::types::ProbeOverride>,
) -> Option<k8s_openapi::api::core::v1::Probe> {
    apply_probe_override(base, override_cfg)
}

pub(crate) fn apply_probe_override(
    base: Option<k8s_openapi::api::core::v1::Probe>,
    override_cfg: Option<&crate::crd::types::ProbeOverride>,
) -> Option<k8s_openapi::api::core::v1::Probe> {
    let cfg = match override_cfg {
        Some(c) => c,
        None => return base,
    };
    let mut probe = base.unwrap_or_default();
    if let Some(v) = cfg.initial_delay_seconds {
        probe.initial_delay_seconds = Some(v);
    }
    if let Some(v) = cfg.period_seconds {
        probe.period_seconds = Some(v);
    }
    if let Some(v) = cfg.timeout_seconds {
        probe.timeout_seconds = Some(v);
    }
    if let Some(v) = cfg.success_threshold {
        probe.success_threshold = Some(v);
    }
    if let Some(v) = cfg.failure_threshold {
        probe.failure_threshold = Some(v);
    }
    Some(probe)
}

/// Default liveness probe per node type.
///
/// - Validator: TCP socket on port 11625 (Stellar Core peer port)
/// - Horizon / SorobanRpc: HTTP GET /health on port 8000
fn default_liveness_probe(node_type: &crate::crd::NodeType) -> k8s_openapi::api::core::v1::Probe {
    use k8s_openapi::api::core::v1::{HTTPGetAction, Probe, TCPSocketAction};
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    match node_type {
        crate::crd::NodeType::Validator => Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(11625),
                ..Default::default()
            }),
            initial_delay_seconds: Some(30),
            period_seconds: Some(15),
            timeout_seconds: Some(5),
            failure_threshold: Some(3),
            success_threshold: Some(1),
            ..Default::default()
        },
        _ => Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/health".to_string()),
                port: IntOrString::Int(8000),
                ..Default::default()
            }),
            initial_delay_seconds: Some(20),
            period_seconds: Some(15),
            timeout_seconds: Some(5),
            failure_threshold: Some(3),
            success_threshold: Some(1),
            ..Default::default()
        },
    }
}

/// Default readiness probe per node type.
///
/// - Validator: exec probe that queries the Stellar-Core HTTP API (`/info`) and
///   marks the pod **Not Ready** when the node is in `CATCHING_UP` or `SYNCING`
///   state.  The pod remains Not Ready until the node is fully synced, preventing
///   traffic from being routed to a node that cannot yet participate in consensus.
///   The liveness probe (TCP socket) is intentionally kept separate so that a
///   syncing node is never restarted — only removed from the ready set.
/// - Horizon / SorobanRpc: HTTP GET /health on port 8000
fn default_readiness_probe(node_type: &crate::crd::NodeType) -> k8s_openapi::api::core::v1::Probe {
    use k8s_openapi::api::core::v1::{ExecAction, HTTPGetAction, Probe};
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    match node_type {
        crate::crd::NodeType::Validator => {
            // Query /info and fail if the node is CATCHING_UP or SYNCING.
            // wget is available in the stellar/stellar-core image.
            // Exit 1 (not ready) when state contains CATCHING_UP or SYNCING.
            let script = concat!(
                "RESP=$(wget -qO- http://localhost:11626/info 2>/dev/null) && ",
                "echo \"$RESP\" | grep -qv '\"state\".*\"CATCHING_UP\"' && ",
                "echo \"$RESP\" | grep -qv '\"state\".*\"SYNCING\"'"
            );
            Probe {
                exec: Some(ExecAction {
                    command: Some(vec![
                        "/bin/sh".to_string(),
                        "-c".to_string(),
                        script.to_string(),
                    ]),
                }),
                initial_delay_seconds: Some(15),
                period_seconds: Some(10),
                timeout_seconds: Some(5),
                failure_threshold: Some(3),
                success_threshold: Some(1),
                ..Default::default()
            }
        }
        _ => Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/health".to_string()),
                port: IntOrString::Int(8000),
                ..Default::default()
            }),
            initial_delay_seconds: Some(10),
            period_seconds: Some(10),
            timeout_seconds: Some(5),
            failure_threshold: Some(3),
            success_threshold: Some(1),
            ..Default::default()
        },
    }
}

/// Default startup probe per node type.
///
/// Allows extra time for initial ledger sync before liveness kicks in.
/// - Validator: 30 × 10s = 5 minutes max startup time
/// - Horizon / SorobanRpc: 30 × 10s = 5 minutes max startup time
fn default_startup_probe(node_type: &crate::crd::NodeType) -> k8s_openapi::api::core::v1::Probe {
    use k8s_openapi::api::core::v1::{HTTPGetAction, Probe, TCPSocketAction};
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    match node_type {
        crate::crd::NodeType::Validator => Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(11625),
                ..Default::default()
            }),
            initial_delay_seconds: Some(10),
            period_seconds: Some(10),
            timeout_seconds: Some(5),
            failure_threshold: Some(30),
            success_threshold: Some(1),
            ..Default::default()
        },
        _ => Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/health".to_string()),
                port: IntOrString::Int(8000),
                ..Default::default()
            }),
            initial_delay_seconds: Some(10),
            period_seconds: Some(10),
            timeout_seconds: Some(5),
            failure_threshold: Some(30),
            success_threshold: Some(1),
            ..Default::default()
        },
    }
}

/// Create PostParams with dry-run support
pub(crate) fn post_params(dry_run: bool) -> PostParams {
    if dry_run {
        PostParams {
            dry_run: true,
            ..Default::default()
        }
    } else {
        PostParams::default()
    }
}

/// Create PatchParams with dry-run support
pub(crate) fn patch_params(dry_run: bool) -> PatchParams {
    let mut params = PatchParams::apply("stellar-operator").force();
    if dry_run {
        params.dry_run = true;
    }
    params
}

/// Create DeleteParams with dry-run support
pub(crate) fn delete_params(dry_run: bool) -> DeleteParams {
    if dry_run {
        DeleteParams {
            dry_run: true,
            ..Default::default()
        }
    } else {
        DeleteParams::default()
    }
}
