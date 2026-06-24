//! Error retry policy.

use super::prelude::*;
use super::state::ControllerState;

/// Error policy determines how to handle reconciliation errors
pub(crate) fn error_policy(
    node: Arc<StellarNode>,
    error: &Error,
    ctx: Arc<ControllerState>,
) -> Action {
    let node_name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let reconcile_id = ctx.next_reconcile_id();

    let node_name_for_span = node_name.clone();
    let namespace_for_span = namespace.clone();
    let resource_version = node
        .metadata
        .resource_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let _error_span = info_span!(
        "reconcile_error",
        node_name = %node_name_for_span,
        namespace = %namespace_for_span,
        reconcile_id = %reconcile_id,
        resource_version = %resource_version
    );
    let _enter = _error_span.enter();

    error!("Reconciliation error for {}: {:?}", node_name, error);

    // Get retry count from annotations (default to 0)
    let retry_count = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("stellar.org/error-retry-count"))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    // Apply operator retry budget based on error retriability.
    let retry_duration = if error.is_retriable() {
        Duration::from_secs(ctx.retry_budget_retriable_secs)
    } else {
        Duration::from_secs(ctx.retry_budget_nonretriable_secs)
    };

    debug!(
        "Requeuing {} after {:?} (retry_count: {}, retriable: {})",
        node.name_any(),
        retry_duration,
        retry_count,
        error.is_retriable()
    );

    Action::requeue(retry_duration)
}

/// Perform quorum analysis for validator nodes
async fn perform_quorum_analysis(
    client: &Client,
    node: &StellarNode,
    max_attempts: u32,
) -> Result<()> {
    use super::quorum::QuorumAnalyzer;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Get pod IPs for all validator pods
    let pod_api: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(client.clone(), &namespace);
    let lp = kube::api::ListParams::default().labels(&format!("app.kubernetes.io/instance={name}"));

    let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;
    let pod_ips: Vec<String> = pods
        .items
        .iter()
        .filter_map(|pod| pod.status.as_ref()?.pod_ip.clone())
        .collect();

    if pod_ips.is_empty() {
        debug!(
            "No pod IPs found for quorum analysis of {}/{}",
            namespace, name
        );
        return Ok(());
    }

    // Create analyzer and run analysis with timeout
    let mut analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100, max_attempts);

    let analysis_future = analyzer.analyze_quorum(pod_ips);
    let result = tokio::time::timeout(Duration::from_secs(30), analysis_future)
        .await
        .map_err(|_| Error::ConfigError("Quorum analysis timeout".to_string()))?
        .map_err(|e| Error::ConfigError(format!("Quorum analysis failed: {e}")))?;

    // Update metrics
    #[cfg(feature = "metrics")]
    {
        let node_type = node.spec.node_type.to_string();
        let hardware_generation = hardware_generation_for_metrics(client, node).await;
        let network = match &node.spec.network {
            crate::crd::StellarNetwork::Mainnet => "mainnet",
            crate::crd::StellarNetwork::Testnet => "testnet",
            crate::crd::StellarNetwork::Futurenet => "futurenet",
            crate::crd::StellarNetwork::Custom(_) => "custom",
        };

        metrics::set_quorum_critical_nodes(
            &namespace,
            &name,
            &node_type,
            network,
            &hardware_generation,
            result.critical_nodes.len() as i64,
        );
        metrics::set_quorum_min_overlap(
            &namespace,
            &name,
            &node_type,
            network,
            &hardware_generation,
            result.min_overlap as i64,
        );
        metrics::set_quorum_fragility_score(
            &namespace,
            &name,
            &node_type,
            network,
            &hardware_generation,
            result.fragility_score,
        );
    }

    // Update status
    analyzer
        .update_node_status(client, node, &result)
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to update status: {e}")))?;

    info!(
        "Quorum analysis complete for {}/{}: fragility={:.3}, critical_nodes={}, min_overlap={}",
        namespace,
        name,
        result.fragility_score,
        result.critical_nodes.len(),
        result.min_overlap
    );

    Ok(())
}

#[cfg(feature = "metrics")]
async fn hardware_generation_for_metrics(client: &Client, node: &StellarNode) -> String {
    match infra::resolve_stellar_node_infra(client, node).await {
        Ok(summary) => summary.hardware_generation_label(),
        Err(err) => {
            warn!(
                "Failed to resolve hardware generation for metrics on {}/{}: {:?}",
                node.namespace().unwrap_or_else(|| "default".to_string()),
                node.name_any(),
                err
            );
            "unknown".to_string()
        }
    }
}
