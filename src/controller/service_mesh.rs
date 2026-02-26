//! Service Mesh Resource Management
//!
//! Provides functions to create and manage service mesh resources (Istio/Linkerd)
//! for mTLS enforcement, circuit breaking, retry policies, and traffic control.

use crate::crd::StellarNode;
use crate::error::Result;
use kube::api::{Api, DynamicObject, Patch, PatchParams};
use kube::discovery::ApiResource;
use kube::{Client, ResourceExt};
use serde_json::json;
use tracing::{info, instrument};

/// Ensure PeerAuthentication for Istio mTLS enforcement
///
/// Creates a Kubernetes PeerAuthentication resource that enforces mutual TLS
/// authentication for traffic destined to the pods of this StellarNode.
///
/// # Arguments
///
/// * `client` - Kubernetes client for API operations
/// * `node` - The StellarNode resource
///
/// # Returns
///
/// Returns Ok(()) on success, or an error if the operation fails.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_peer_authentication(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let Some(ref mesh_config) = node.spec.service_mesh else {
        return Ok(());
    };
    let Some(ref istio_config) = mesh_config.istio else {
        return Ok(());
    };

    let name = format!("{}-peer-auth", node.name_any());

    let mtls_mode = match istio_config.mtls_mode {
        crate::crd::MtlsMode::Strict => "STRICT",
        crate::crd::MtlsMode::Permissive => "PERMISSIVE",
    };

    let peer_auth = json!({
        "apiVersion": "security.istio.io/v1beta1",
        "kind": "PeerAuthentication",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/name": "stellar-operator",
                "app.kubernetes.io/instance": node.name_any(),
                "stellar.org/node": node.name_any()
            },
            "ownerReferences": [{
                "apiVersion": "stellar.org/v1alpha1",
                "kind": "StellarNode",
                "name": node.name_any(),
                "uid": node.metadata.uid.as_ref().unwrap_or(&"".to_string()),
                "controller": true,
                "blockOwnerDeletion": true
            }]
        },
        "spec": {
            "mtls": {
                "mode": mtls_mode
            },
            "selector": {
                "matchLabels": {
                    "stellar.org/node": node.name_any()
                }
            }
        }
    });

    let api_resource = ApiResource {
        group: "security.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "security.istio.io/v1beta1".to_string(),
        kind: "PeerAuthentication".to_string(),
        plural: "peerauthentications".to_string(),
    };

    let peer_auth_obj = DynamicObject::new(&name, &api_resource)
        .within(&namespace)
        .data(peer_auth);

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&peer_auth_obj),
    )
    .await?;

    info!("Ensured PeerAuthentication {}/{}", namespace, name);
    Ok(())
}

/// Ensure DestinationRule for Istio traffic policies
///
/// Creates a Kubernetes DestinationRule resource that configures:
/// - Circuit breaking with outlier detection
/// - Mutual TLS settings (ISTIO_MUTUAL)
/// - Load balancing strategy
/// - Connection pool settings
///
/// # Arguments
///
/// * `client` - Kubernetes client for API operations
/// * `node` - The StellarNode resource
///
/// # Returns
///
/// Returns Ok(()) on success, or an error if the operation fails.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_destination_rule(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let Some(ref mesh_config) = node.spec.service_mesh else {
        return Ok(());
    };
    let Some(ref istio_config) = mesh_config.istio else {
        return Ok(());
    };

    let name = format!("{}-dest-rule", node.name_any());
    let service_name = format!("{}-service", node.name_any());

    // Build TrafficPolicy with circuit breaker
    let mut traffic_policy = json!({
        "connectionPool": {
            "tcp": {
                "maxConnections": 100
            },
            "http": {
                "http1MaxPendingRequests": 100,
                "http2MaxRequests": 1000,
                "maxRequestsPerConnection": 2
            }
        },
        "loadBalancer": {
            "simple": "LEAST_REQUEST"
        }
    });

    if let Some(ref cb) = istio_config.circuit_breaker {
        traffic_policy["outlierDetection"] = json!({
            "consecutiveErrors": cb.consecutive_errors,
            "interval": format!("{}s", cb.time_window_secs),
            "baseEjectionTime": "30s",
            "minRequestVolume": cb.min_request_volume,
            "maxEjectionPercent": 50,
            "splitExternalLocalOriginErrors": true
        });
    }

    let dest_rule = json!({
        "apiVersion": "networking.istio.io/v1beta1",
        "kind": "DestinationRule",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/name": "stellar-operator",
                "app.kubernetes.io/instance": node.name_any(),
                "stellar.org/node": node.name_any()
            },
            "ownerReferences": [{
                "apiVersion": "stellar.org/v1alpha1",
                "kind": "StellarNode",
                "name": node.name_any(),
                "uid": node.metadata.uid.as_ref().unwrap_or(&"".to_string()),
                "controller": true,
                "blockOwnerDeletion": true
            }]
        },
        "spec": {
            "host": format!("{}.{}.svc.cluster.local", service_name, namespace),
            "trafficPolicy": traffic_policy,
            "tlsSettings": {
                "mode": "ISTIO_MUTUAL"
            }
        }
    });

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "DestinationRule".to_string(),
        plural: "destinationrules".to_string(),
    };

    let dest_rule_obj = DynamicObject::new(&name, &api_resource)
        .within(&namespace)
        .data(dest_rule);

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&dest_rule_obj),
    )
    .await?;

    info!("Ensured DestinationRule {}/{}", namespace, name);
    Ok(())
}

/// Ensure VirtualService with retry policy
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_virtual_service(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let Some(ref mesh_config) = node.spec.service_mesh else {
        return Ok(());
    };
    let Some(ref istio_config) = mesh_config.istio else {
        return Ok(());
    };

    let name = format!("{}-virtual-svc", node.name_any());
    let service_name = format!("{}-service", node.name_any());

    let mut retries = json!({
        "attempts": 3,
        "perTryTimeout": "5s"
    });

    if let Some(ref retry_cfg) = istio_config.retries {
        retries["attempts"] = json!(retry_cfg.max_retries);
        retries["perTryTimeout"] = json!(format!("{}ms", retry_cfg.backoff_ms));

        if !retry_cfg.retryable_status_codes.is_empty() {
            let status_codes = retry_cfg
                .retryable_status_codes
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",");
            retries["retryOn"] = json!(format!(
                "5xx,reset,connect-failure,retriable-4xx,{}",
                status_codes
            ));
        }
    }

    let virtual_svc = json!({
        "apiVersion": "networking.istio.io/v1beta1",
        "kind": "VirtualService",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/name": "stellar-operator",
                "app.kubernetes.io/instance": node.name_any(),
                "stellar.org/node": node.name_any()
            },
            "ownerReferences": [{
                "apiVersion": "stellar.org/v1alpha1",
                "kind": "StellarNode",
                "name": node.name_any(),
                "uid": node.metadata.uid.as_ref().unwrap_or(&"".to_string()),
                "controller": true,
                "blockOwnerDeletion": true
            }]
        },
        "spec": {
            "hosts": [
                format!("{}.{}.svc.cluster.local", service_name, namespace)
            ],
            "http": [
                {
                    "match": [{"uri": {"prefix": "/"}}],
                    "route": [
                        {
                            "destination": {
                                "host": format!("{}.{}.svc.cluster.local", service_name, namespace),
                                "port": {"number": 80}
                            },
                            "weight": 100
                        }
                    ],
                    "retries": retries,
                    "timeout": format!("{}s", istio_config.timeout_secs)
                }
            ]
        }
    });

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "VirtualService".to_string(),
        plural: "virtualservices".to_string(),
    };

    let virtual_svc_obj = DynamicObject::new(&name, &api_resource)
        .within(&namespace)
        .data(virtual_svc);

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&virtual_svc_obj),
    )
    .await?;

    info!("Ensured VirtualService {}/{}", namespace, name);
    Ok(())
}

/// Ensure RequestAuthentication for JWT validation (future extension)
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_request_authentication(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let Some(ref mesh_config) = node.spec.service_mesh else {
        return Ok(());
    };

    if !mesh_config.sidecar_injection {
        return Ok(());
    }

    let name = format!("{}-req-auth", node.name_any());

    let req_auth = json!({
        "apiVersion": "security.istio.io/v1beta1",
        "kind": "RequestAuthentication",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/name": "stellar-operator",
                "app.kubernetes.io/instance": node.name_any(),
                "stellar.org/node": node.name_any()
            },
            "ownerReferences": [{
                "apiVersion": "stellar.org/v1alpha1",
                "kind": "StellarNode",
                "name": node.name_any(),
                "uid": node.metadata.uid.as_ref().unwrap_or(&"".to_string()),
                "controller": true,
                "blockOwnerDeletion": true
            }]
        },
        "spec": {
            "selector": {
                "matchLabels": {
                    "stellar.org/node": node.name_any()
                }
            },
            "jwtRules": []
        }
    });

    let api_resource = ApiResource {
        group: "security.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "security.istio.io/v1beta1".to_string(),
        kind: "RequestAuthentication".to_string(),
        plural: "requestauthentications".to_string(),
    };

    let req_auth_obj = DynamicObject::new(&name, &api_resource)
        .within(&namespace)
        .data(req_auth);

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&req_auth_obj),
    )
    .await?;

    info!("Ensured RequestAuthentication {}/{}", namespace, name);
    Ok(())
}

/// Delete all service mesh resources for a node
///
/// Removes PeerAuthentication, DestinationRule, VirtualService, and RequestAuthentication
/// resources created for this StellarNode.
///
/// # Arguments
///
/// * `client` - Kubernetes client for API operations
/// * `node` - The StellarNode resource
///
/// # Returns
///
/// Returns Ok(()) on success, or an error if the operation fails.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_service_mesh_resources(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    let peer_auth_api = ApiResource {
        group: "security.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "security.istio.io/v1beta1".to_string(),
        kind: "PeerAuthentication".to_string(),
        plural: "peerauthentications".to_string(),
    };

    let dest_rule_api = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "DestinationRule".to_string(),
        plural: "destinationrules".to_string(),
    };

    let virtual_svc_api = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "VirtualService".to_string(),
        plural: "virtualservices".to_string(),
    };

    let req_auth_api = ApiResource {
        group: "security.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "security.istio.io/v1beta1".to_string(),
        kind: "RequestAuthentication".to_string(),
        plural: "requestauthentications".to_string(),
    };

    let apis = vec![
        (peer_auth_api, format!("{}-peer-auth", node.name_any())),
        (dest_rule_api, format!("{}-dest-rule", node.name_any())),
        (virtual_svc_api, format!("{}-virtual-svc", node.name_any())),
        (req_auth_api, format!("{}-req-auth", node.name_any())),
    ];

    for (api_resource, resource_name) in apis {
        let api: Api<DynamicObject> =
            Api::namespaced_with(client.clone(), &namespace, &api_resource);
        let _ = api.delete(&resource_name, &Default::default()).await;
    }

    info!(
        "Deleted service mesh resources for {}/{}",
        namespace,
        node.name_any()
    );
    Ok(())
}
