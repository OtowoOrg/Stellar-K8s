//! ServiceMonitor management.

use super::helpers::*;
use super::prelude::*;

// ============================================================================
// ServiceMonitor
// ============================================================================

fn service_monitor_api_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind {
        group: "monitoring.coreos.com".to_string(),
        version: "v1".to_string(),
        kind: "ServiceMonitor".to_string(),
    })
}

pub async fn ensure_service_monitor(client: &Client, node: &StellarNode) -> Result<()> {
    if !matches!(
        node.spec.node_type,
        NodeType::Horizon | NodeType::SorobanRpc
    ) {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service-monitor");
    let api_resource = service_monitor_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    let mut service_monitor = DynamicObject::new(&name, &api_resource).within(&namespace);
    service_monitor.metadata.labels = Some(standard_labels(node));
    service_monitor.metadata.owner_references = Some(vec![owner_reference(node)]);
    service_monitor.data = serde_json::to_value(serde_json::json!({
        "spec": {
            "jobLabel": "app.kubernetes.io/instance",
            "namespaceSelector": {
                "matchNames": [namespace]
            },
            "selector": {
                "matchLabels": {
                    "app.kubernetes.io/name": "stellar-node",
                    "app.kubernetes.io/instance": node.name_any()
                }
            },
            "endpoints": [
                {
                    "targetPort": 8000,
                    "path": "/metrics",
                    "interval": "30s",
                    "scheme": "http"
                }
            ]
        }
    }))
    .unwrap_or_default();

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&service_monitor),
    )
    .await
    .map_err(Error::KubeError)?;

    info!(
        "Ensured ServiceMonitor {}/{} for Prometheus Operator scraping",
        namespace, name
    );

    Ok(())
}

pub async fn delete_service_monitor(client: &Client, node: &StellarNode) -> Result<()> {
    if !matches!(
        node.spec.node_type,
        NodeType::Horizon | NodeType::SorobanRpc
    ) {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service-monitor");
    let api_resource = service_monitor_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted ServiceMonitor {}/{}", namespace, name),
        Err(kube::Error::Api(api_err)) if api_err.code == 404 => {
            info!(
                "ServiceMonitor {}/{} not found (already deleted)",
                namespace, name
            )
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

pub async fn delete_alerting(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => info!("Deleted alerting ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

pub async fn delete_canary_resources(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let canary_name = format!("{name}-canary");

    if node.spec.ingress.is_some() {
        let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
        let _ = api.delete(&canary_name, &delete_params(dry_run)).await;
    }

    let api_svc: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let _ = api_svc.delete(&canary_name, &delete_params(dry_run)).await;

    let api_deploy: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let _ = api_deploy
        .delete(&canary_name, &delete_params(dry_run))
        .await;

    Ok(())
}
