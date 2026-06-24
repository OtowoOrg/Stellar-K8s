//! Cleanup on deletion.

use super::events::ActionType;
use super::events::{publish_object_event, recorder_for};
use super::prelude::*;
use super::state::ControllerState;

pub(crate) fn cleanup_stellar_node(
    client: Client,
    node: Arc<StellarNode>,
    ctx: Arc<ControllerState>,
) -> BoxFuture<'static, Result<Action>> {
    async move {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        info!("Cleaning up StellarNode: {}/{}", namespace, name);

        let recorder = recorder_for(&client, &ctx.event_reporter, &node);
        if let Err(e) = publish_object_event(
            &recorder,
            EventType::Normal,
            "FinalizerCleanupStarted",
            "Finalize",
            "Finalizer cleanup started; removing managed Kubernetes resources for this StellarNode.",
        )
        .await
        {
            warn!("Failed to publish FinalizerCleanupStarted event: {e}");
        }

        // Delete resources in reverse order of creation

        // 0a. Delete Managed Database Resources
        apply_or_emit!(&ctx, &node, ActionType::Delete, "Managed Database", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_cnpg_resources(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete CNPG resources: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 0. Delete Alerting
        apply_or_emit!(&ctx, &node, ActionType::Delete, "Alerting", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_alerting(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete alerting: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 0b. Delete VPA (if vpaConfig was configured)
        apply_or_emit!(&ctx, &node, ActionType::Delete, "VPA", move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = vpa_controller::delete_vpa(&client, &node).await {
                warn!("Failed to delete VPA: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 1. Delete HPA (if autoscaling was configured)
        apply_or_emit!(&ctx, &node, ActionType::Delete, "HPA", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_hpa(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete HPA: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 2. Delete ServiceMonitor (if autoscaling was configured)
        apply_or_emit!(&ctx, &node, ActionType::Delete, "ServiceMonitor", move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_service_monitor(&client, &node).await {
                warn!("Failed to delete ServiceMonitor: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 3. Delete Ingress
        apply_or_emit!(&ctx, &node, ActionType::Delete, "Ingress", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_ingress(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete Ingress: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 3a. Delete NetworkPolicy
        apply_or_emit!(&ctx, &node, ActionType::Delete, "NetworkPolicy", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_network_policy(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete NetworkPolicy: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 3b. Delete MetalLB LoadBalancer Service
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Delete,
            "MetalLB LoadBalancer",
            move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                if let Err(e) = resources::delete_load_balancer_service(&client, &node).await {
                    warn!("Failed to delete MetalLB LoadBalancer service: {:?}", e);
                }
                if let Err(e) = resources::delete_metallb_config(&client, &node).await {
                    warn!("Failed to delete MetalLB configuration: {:?}", e);
                }
                Ok(())
            }
        )
        .await?;

        // 3c. Delete Service Mesh Resources (Istio/Linkerd)
        apply_or_emit!(&ctx, &node, ActionType::Delete, "Service Mesh", move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = service_mesh::delete_service_mesh_resources(&client, &node).await {
                warn!("Failed to delete service mesh resources: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 3d. Delete PDB
        apply_or_emit!(&ctx, &node, ActionType::Delete, "PDB", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_pdb(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete PodDisruptionBudget: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 4. Delete Service
        apply_or_emit!(&ctx, &node, ActionType::Delete, "Service", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_service(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete Service: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 5. Delete Deployment/StatefulSet
        apply_or_emit!(&ctx, &node, ActionType::Delete, "Workload", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_workload(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete workload: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 6. Delete ConfigMap
        apply_or_emit!(&ctx, &node, ActionType::Delete, "ConfigMap", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            if let Err(e) = resources::delete_config_map(&client, &node, ctx.dry_run).await {
                warn!("Failed to delete ConfigMap: {:?}", e);
            }
            Ok(())
        })
        .await?;

        // 7. Delete PVC based on retention policy
        if node.spec.should_delete_pvc() {
            info!(
                "Deleting PVC for node: {}/{} (retention policy: Delete)",
                namespace, name
            );
            apply_or_emit!(&ctx, &node, ActionType::Delete, "PVC", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                if let Err(e) = resources::delete_pvc(&client, &node, ctx.dry_run).await {
                    warn!("Failed to delete PVC: {:?}", e);
                }
                Ok(())
            })
            .await?;
        } else {
            info!(
                "Retaining PVC for node: {}/{} (retention policy: Retain)",
                namespace, name
            );
        }

        info!("Cleanup complete for StellarNode: {}/{}", namespace, name);

        // Return await_change to signal finalizer completion
        Ok(Action::await_change())
    }
    .boxed()
}
