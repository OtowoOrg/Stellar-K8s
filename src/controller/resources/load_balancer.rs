//! MetalLB load balancer stubs.

use super::helpers::*;
use super::prelude::*;

// ============================================================================
// LoadBalancer Service (MetalLB Integration) — stubs, wiring in progress
// ============================================================================

#[instrument(skip(_client, _node), fields(name = %_node.name_any(), namespace = _node.namespace()))]
pub async fn delete_load_balancer_service(_client: &Client, _node: &StellarNode) -> Result<()> {
    Ok(())
}

#[instrument(skip(_client, _node), fields(name = %_node.name_any(), namespace = _node.namespace()))]
pub async fn delete_metallb_config(_client: &Client, _node: &StellarNode) -> Result<()> {
    Ok(())
}

/// Delete the Service for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_service(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => info!("Deleted Service {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("Service {} not found", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}
