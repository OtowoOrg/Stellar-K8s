//! Ingress management.

use super::prelude::*;
use super::helpers::*;

// ============================================================================
// Ingress — called by the reconciler when spec.ingress is configured
// ============================================================================

/// Ensure a Kubernetes Ingress resource exists for the node.
/// Called from the reconciler for Horizon and SorobanRpc node types when
/// `spec.ingress` is set.
#[allow(dead_code)] // called via reconciler ingress path; conditional on feature flag
pub async fn ensure_ingress(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let ingress_cfg = match &node.spec.ingress {
        Some(cfg)
            if matches!(
                node.spec.node_type,
                NodeType::Horizon | NodeType::SorobanRpc
            ) =>
        {
            cfg
        }
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "ingress");

    let ingress = build_ingress(node, ingress_cfg);

    api.patch(&name, &patch_params(dry_run), &Patch::Apply(&ingress))
        .await?;

    info!("Ingress ensured for {}/{}", namespace, name);

    if let Some(cfg) = node.spec.strategy.canary() {
        if node
            .status
            .as_ref()
            .and_then(|status| status.canary_version.as_ref())
            .is_some()
        {
            let canary_name = format!("{name}-canary");
            let mut canary_ingress = build_ingress(node, ingress_cfg);
            canary_ingress.metadata.name = Some(canary_name.clone());

            // Use the live canary weight from status if available (progressive stepping),
            // otherwise fall back to the configured initial weight.
            let effective_weight = node
                .status
                .as_ref()
                .and_then(|s| s.canary_weight)
                .unwrap_or(cfg.weight);

            let mut annotations = canary_ingress
                .metadata
                .annotations
                .clone()
                .unwrap_or_default();
            annotations.insert(
                "nginx.ingress.kubernetes.io/canary".to_string(),
                "true".to_string(),
            );
            annotations.insert(
                "nginx.ingress.kubernetes.io/canary-weight".to_string(),
                effective_weight.to_string(),
            );
            annotations.insert(
                "traefik.ingress.kubernetes.io/service.weights".to_string(),
                format!("{}:{}", node.name_any(), effective_weight),
            );

            canary_ingress.metadata.annotations = Some(annotations);

            if let Some(spec) = &mut canary_ingress.spec {
                if let Some(rules) = &mut spec.rules {
                    for rule in rules {
                        if let Some(http) = &mut rule.http {
                            for path in &mut http.paths {
                                if let Some(backend) = &mut path.backend.service {
                                    backend.name = format!("{}-canary", node.name_any());
                                }
                            }
                        }
                    }
                }
            }

            api.patch(
                &canary_name,
                &patch_params(dry_run),
                &Patch::Apply(&canary_ingress),
            )
            .await?;
            info!("Canary Ingress ensured for {}/{}", namespace, canary_name);

            // Istio VirtualService traffic splitting (when ingress class is "istio")
            if ingress_cfg
                .class_name
                .as_deref()
                .map(|c| c == "istio")
                .unwrap_or(false)
            {
                ensure_istio_canary_virtual_service(
                    client,
                    node,
                    ingress_cfg,
                    effective_weight,
                    dry_run,
                )
                .await?;
            }
        } else {
            let canary_name = format!("{name}-canary");
            let _ = api.delete(&canary_name, &delete_params(dry_run)).await;

            // Clean up Istio VirtualService if it exists
            if ingress_cfg
                .class_name
                .as_deref()
                .map(|c| c == "istio")
                .unwrap_or(false)
            {
                delete_istio_canary_virtual_service(client, node, dry_run).await?;
            }
        }
    }

    Ok(())
}

/// Ensure an Istio VirtualService that splits traffic between stable and canary services.
///
/// Creates a VirtualService using the Istio networking API via DynamicObject.
/// The stable service receives `(100 - weight)%` and the canary receives `weight%`.
async fn ensure_istio_canary_virtual_service(
    client: &Client,
    node: &StellarNode,
    ingress_cfg: &IngressConfig,
    canary_weight: i32,
    _dry_run: bool,
) -> Result<()> {
    use kube::api::DynamicObject;
    use kube::discovery::ApiResource;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let stable_weight = 100 - canary_weight.clamp(0, 100);
    let vs_name = format!("{}-canary-vs", node.name_any());

    let hosts: Vec<String> = ingress_cfg.hosts.iter().map(|h| h.host.clone()).collect();

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "VirtualService".to_string(),
        plural: "virtualservices".to_string(),
    };

    let mut vs = DynamicObject::new(&vs_name, &api_resource).within(&namespace);
    vs.data = serde_json::json!({
        "spec": {
            "hosts": hosts,
            "http": [{
                "route": [
                    {
                        "destination": {
                            "host": node.name_any(),
                            "port": { "number": 8000 }
                        },
                        "weight": stable_weight
                    },
                    {
                        "destination": {
                            "host": format!("{}-canary", node.name_any()),
                            "port": { "number": 8000 }
                        },
                        "weight": canary_weight
                    }
                ]
            }]
        }
    });

    let api: kube::Api<DynamicObject> =
        kube::Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api
        .patch(
            &vs_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&vs),
        )
        .await
    {
        Ok(_) => {
            info!(
                "Istio VirtualService {}/{} updated: stable={}% canary={}%",
                namespace, vs_name, stable_weight, canary_weight
            );
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to apply Istio VirtualService (Istio may not be installed): {}",
                e
            );
            Ok(()) // Non-fatal — Nginx annotations still work
        }
    }
}

/// Delete the Istio VirtualService for a canary rollout.
async fn delete_istio_canary_virtual_service(
    client: &Client,
    node: &StellarNode,
    _dry_run: bool,
) -> Result<()> {
    use kube::api::DynamicObject;
    use kube::discovery::ApiResource;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let vs_name = format!("{}-canary-vs", node.name_any());

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "VirtualService".to_string(),
        plural: "virtualservices".to_string(),
    };

    let api: kube::Api<DynamicObject> =
        kube::Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api.delete(&vs_name, &DeleteParams::default()).await {
        Ok(_) => {
            info!("Deleted Istio VirtualService {}/{}", namespace, vs_name);
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => {
            warn!("Failed to delete Istio VirtualService: {}", e);
        }
    }

    Ok(())
}

pub(crate) fn build_ingress(node: &StellarNode, config: &IngressConfig) -> Ingress {
    let labels = standard_labels(node);
    let name = resource_name(node, "ingress");

    let service_port = match node.spec.node_type {
        NodeType::Horizon | NodeType::SorobanRpc => 8000,
        NodeType::Validator => 11626,
    };

    let mut annotations = config.annotations.clone().unwrap_or_default();
    if let Some(issuer) = &config.cert_manager_issuer {
        annotations.insert("cert-manager.io/issuer".to_string(), issuer.clone());
    }
    if let Some(cluster_issuer) = &config.cert_manager_cluster_issuer {
        annotations.insert(
            "cert-manager.io/cluster-issuer".to_string(),
            cluster_issuer.clone(),
        );
    }

    if let Some(dns_config) = &config.external_dns {
        annotations.insert(
            "external-dns.alpha.kubernetes.io/hostname".to_string(),
            dns_config.hostname.clone(),
        );
        annotations.insert(
            "external-dns.alpha.kubernetes.io/ttl".to_string(),
            dns_config.ttl.to_string(),
        );
        if let Some(provider) = &dns_config.provider {
            annotations.insert(
                "external-dns.alpha.kubernetes.io/provider".to_string(),
                provider.clone(),
            );
        }
        if let Some(extra_annotations) = &dns_config.annotations {
            for (k, v) in extra_annotations {
                annotations.insert(k.clone(), v.clone());
            }
        }
    }

    if let Some(rl) = &config.rate_limit {
        if let Some(rps) = rl.requests_per_second {
            annotations.insert(
                "nginx.ingress.kubernetes.io/limit-rps".to_string(),
                rps.to_string(),
            );
        }
        if let Some(rpm) = rl.requests_per_minute {
            annotations.insert(
                "nginx.ingress.kubernetes.io/limit-rpm".to_string(),
                rpm.to_string(),
            );
        }
        if let Some(conns) = rl.connections {
            annotations.insert(
                "nginx.ingress.kubernetes.io/limit-connections".to_string(),
                conns.to_string(),
            );
        }
        if let Some(burst) = rl.burst_multiplier {
            annotations.insert(
                "nginx.ingress.kubernetes.io/limit-burst-multiplier".to_string(),
                burst.to_string(),
            );
        }
        if let Some(whitelist) = &rl.whitelist_cidrs {
            annotations.insert(
                "nginx.ingress.kubernetes.io/limit-whitelist".to_string(),
                whitelist.clone(),
            );
        }
    }

    let rules: Vec<IngressRule> = config
        .hosts
        .iter()
        .map(|host| IngressRule {
            host: Some(host.host.clone()),
            http: Some(HTTPIngressRuleValue {
                paths: host
                    .paths
                    .iter()
                    .map(|p| HTTPIngressPath {
                        path: Some(p.path.clone()),
                        path_type: p.path_type.clone().unwrap_or_else(|| "Prefix".to_string()),
                        backend: IngressBackend {
                            service: Some(IngressServiceBackend {
                                name: node.name_any(),
                                port: Some(ServiceBackendPort {
                                    number: Some(service_port),
                                    name: None,
                                }),
                            }),
                            ..Default::default()
                        },
                    })
                    .collect(),
            }),
        })
        .collect();

    let tls = config.tls_secret_name.as_ref().map(|secret| {
        vec![IngressTLS {
            hosts: Some(config.hosts.iter().map(|h| h.host.clone()).collect()),
            secret_name: Some(secret.clone()),
        }]
    });

    Ingress {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        spec: Some(IngressSpec {
            ingress_class_name: config.class_name.clone(),
            rules: Some(rules),
            tls,
            ..Default::default()
        }),
        status: None,
    }
}

pub async fn delete_ingress(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    if node.spec.ingress.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "ingress");

    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => info!("Deleted Ingress {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("Ingress {} not found, already deleted", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

