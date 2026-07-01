//! HPA management.

use super::alerting::build_hpa;
use super::helpers::*;
use super::prelude::*;

// ============================================================================
// HorizontalPodAutoscaler — unchanged
// ============================================================================

pub async fn ensure_hpa(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    if !matches!(
        node.spec.node_type,
        NodeType::Horizon | NodeType::SorobanRpc
    ) || node.spec.autoscaling.is_none()
    {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "hpa");

    let hpa = build_hpa(node)?;

    let patch = Patch::Apply(&hpa);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    info!("HPA ensured for {}/{}", namespace, name);
    Ok(())
}
