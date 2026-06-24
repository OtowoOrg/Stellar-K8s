//! Service management.

use super::helpers::*;
use super::prelude::*;

// ============================================================================
// Service
// ============================================================================

/// Ensure a Service exists for the node
#[instrument(skip(client, node, propagated_labels), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_service(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    propagated_labels: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    // Fetch existing resource labels for stale-label removal
    let existing_labels = match api.get(&name).await {
        Ok(existing) => existing.metadata.labels.clone().unwrap_or_default(),
        Err(kube::Error::Api(e)) if e.code == 404 => BTreeMap::new(),
        Err(e) => return Err(Error::KubeError(e)),
    };

    let mut service = build_service(node, enable_mtls);

    // Apply label propagation: merge propagated labels, then remove stale ones
    let base_labels = service.metadata.labels.clone().unwrap_or_default();
    let merged = LabelPropagator::merge_onto(&base_labels, propagated_labels);
    let final_labels =
        LabelPropagator::remove_stale_labels(&merged, propagated_labels, &existing_labels);
    service.metadata.labels = Some(final_labels);

    let patch = Patch::Apply(&service);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

/// Ensure a canary Service exists if needed
pub async fn ensure_canary_service(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    dry_run: bool,
) -> Result<()> {
    if node
        .status
        .as_ref()
        .and_then(|status| status.canary_version.as_ref())
        .is_none()
    {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-canary", node.name_any());

    let mut service = build_service(node, enable_mtls);
    service.metadata.name = Some(name.clone());

    if let Some(spec) = &mut service.spec {
        let mut labels = standard_labels(node);
        labels.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        spec.selector = Some(labels.clone());

        let meta = &mut service.metadata;
        meta.labels = Some(labels);
    }

    let patch = Patch::Apply(&service);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

pub(crate) fn build_service(node: &StellarNode, enable_mtls: bool) -> Service {
    let mut labels = standard_labels(node);
    merge_service_metadata_labels(&mut labels, node);
    let name = node.name_any();

    let mut annotations = BTreeMap::new();

    // Collect ExternalDNS config from ValidatorConfig or LoadBalancerConfig
    let mut dns_configs = Vec::new();
    if let Some(vc) = &node.spec.validator_config {
        if let Some(dns) = &vc.external_dns {
            dns_configs.push(dns);
        }
    }
    if let Some(lb) = &node.spec.load_balancer {
        if let Some(dns) = &lb.external_dns {
            dns_configs.push(dns);
        }
    }

    if !dns_configs.is_empty() {
        // Use the first one found, prioritize ValidatorConfig
        let dns_config = dns_configs[0];
        let mut hostnames = vec![dns_config.hostname.clone()];

        // Automatically generate _stellar-peering._tcp SRV record for validators
        if node.spec.node_type == NodeType::Validator {
            hostnames.push(format!("_stellar-peering._tcp.{}", dns_config.hostname));
        }

        annotations.insert(
            "external-dns.alpha.kubernetes.io/hostname".to_string(),
            hostnames.join(", "),
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

    merge_service_annotations(&mut annotations, node);

    let http_port_name = if enable_mtls { "https" } else { "http" }.to_string();

    let ports = match node.spec.node_type {
        NodeType::Validator => vec![
            ServicePort {
                name: Some("peer".to_string()),
                port: 11625,
                ..Default::default()
            },
            ServicePort {
                name: Some(http_port_name),
                port: 11626,
                ..Default::default()
            },
        ],
        NodeType::Horizon => vec![ServicePort {
            name: Some(http_port_name),
            port: 8000,
            ..Default::default()
        }],
        NodeType::SorobanRpc => vec![ServicePort {
            name: Some(http_port_name),
            port: 8000,
            ..Default::default()
        }],
    };

    Service {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(ServiceSpec {
            selector: Some(labels),
            ports: Some(ports),
            ..Default::default()
        }),
        status: None,
    }
}
