//! Reconcile dispatch.

use super::prelude::*;
use super::apply::apply_stellar_node;
use super::cleanup::cleanup_stellar_node;

pub(crate) fn reconcile(
    obj: Arc<StellarNode>,
    ctx: Arc<ControllerState>,
) -> BoxFuture<'static, Result<Action>> {
    async move {
        let node_name = obj.name_any();
        let namespace = obj.namespace().unwrap_or_else(|| "default".to_string());

        #[cfg(feature = "metrics")]
        let reconcile_start = std::time::Instant::now();

        if !ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
            debug!("Not the leader, skipping reconciliation");
            return Ok(Action::requeue(Duration::from_secs(5)));
        }

        let res = {
            let client = ctx.client.clone();
            let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

            info!(
                "Reconciling StellarNode {}/{} (type: {:?})",
                namespace, node_name, obj.spec.node_type
            );

            // 1. Advanced Configuration Validation
            let validation_errors = crate::config_mgmt::validation::Validator::validate(&obj.spec);
            if !validation_errors.is_empty() {
                warn!("Configuration validation failed for {}/{}: {:?}", namespace, node_name, validation_errors);
                // In a real implementation, we would update status with these errors and return Action::requeue
            }

            // 2. Automatic Rollback Check
            if let Some(status) = &obj.status {
                if crate::config_mgmt::rollback::RollbackManager::should_rollback(&status.conditions) {
                    warn!("Critical failure detected for {}/{}, checking for rollback target...", namespace, node_name);
                    // Rollback logic would go here: fetch history, find stable version, patch CRD back
                }
            }

            // 3. Security Policy Enforcement
            let security_violations = crate::security::policy::PolicyEnforcer::enforce_policy(&obj.spec);
            if !security_violations.is_empty() {
                warn!("Security policy violations detected for {}/{}: {:?}", namespace, node_name, security_violations);
                // In a real implementation, we would block reconciliation or fire critical alerts
            }

            // Manual finalizer logic to avoid HRTB Send issues with the helper closure
            if obj.metadata.deletion_timestamp.is_some() {
                if obj.finalizers().iter().any(|f| f == STELLAR_NODE_FINALIZER) {
                    cleanup_stellar_node(client.clone(), obj.clone(), ctx.clone()).await?;

                    let patch = serde_json::json!({
                        "metadata": {
                            "finalizers": obj.finalizers().iter().filter(|f| f != &STELLAR_NODE_FINALIZER).collect::<Vec<_>>()
                        }
                    });
                    api.patch(&node_name, &PatchParams::default(), &Patch::Merge(patch)).await?;
                }
                Ok(Action::await_change())
            } else {
                if !obj.finalizers().iter().any(|f| f == STELLAR_NODE_FINALIZER) {
                    let mut finalizers = obj.finalizers().to_vec();
                    finalizers.push(STELLAR_NODE_FINALIZER.to_string());
                    let patch = serde_json::json!({
                        "metadata": {
                            "finalizers": finalizers
                        }
                    });
                    api.patch(&node_name, &PatchParams::default(), &Patch::Merge(patch)).await?;
                }
                apply_stellar_node(client.clone(), obj.clone(), ctx.clone()).await
            }
        };

        #[cfg(feature = "metrics")]
        {
            let seconds = reconcile_start.elapsed().as_secs_f64();
            metrics::observe_reconcile_duration_seconds("stellarnode", seconds);
            if let Err(err) = &res {
                // Keep the label cardinality low: a few broad error kinds.
                let kind = match err {
                    Error::KubeError(_) => "kube",
                    Error::ValidationError(_) => "validation",
                    Error::ConfigError(_) => "config",
                    _ => "unknown",
                };
                metrics::inc_reconcile_error("stellarnode", kind);
                metrics::inc_operator_reconcile_error("stellarnode", kind);
            } else {
                // Record successful reconciliation timestamp
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                ctx.last_reconcile_success
                    .store(now, std::sync::atomic::Ordering::Relaxed);
            }
        }

        res
    }
    .boxed()
}

