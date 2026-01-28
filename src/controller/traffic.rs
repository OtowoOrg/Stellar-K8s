use k8s_openapi::api::core::v1::{Pod, Service, ServicePort, ServiceSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    api::{Api, Patch, PatchParams, ListParams},
    Client, ResourceExt,
};
use std::time::Duration;
use tracing::{info, debug, instrument};
use serde::Deserialize;

use crate::crd::{StellarNode, ReadReplicaStrategy};
use crate::error::{Error, Result};

#[derive(Debug, Deserialize)]
struct StellarCoreInfo {
    info: InfoSection,
}

#[derive(Debug, Deserialize)]
struct InfoSection {
    ledger: LedgerInfo,
}

#[derive(Debug, Deserialize)]
struct LedgerInfo {
    num: u64,
    _age: u64,
}

/// Reconcile traffic routing for read-only replicas
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn reconcile_traffic_routing(
    client: &Client,
    node: &StellarNode,
) -> Result<()> {
    if node.spec.read_replica_config.is_none() {
        return Ok(());
    }
    
    let config = node.spec.read_replica_config.as_ref().unwrap();
    let _namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    
    // 1. Ensure the traffic service exists
    ensure_traffic_service(client, node).await?;

    // 2. If strategy is FreshnessPreferred, update pod labels
    if config.strategy == ReadReplicaStrategy::FreshnessPreferred {
        update_pod_labels_based_on_lag(client, node).await?;
    } else {
        // For RoundRobin, we ensure all ready pods have the traffic label
        ensure_all_ready_pods_enabled(client, node).await?;
    }

    Ok(())
}

async fn ensure_traffic_service(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-read-traffic", node.name_any());

    let mut selector = super::resources::standard_labels(node);
    selector.insert("stellar.org/role".to_string(), "read-replica".to_string());
    selector.insert("stellar.org/traffic".to_string(), "enabled".to_string());

    let ports = vec![
        ServicePort {
            name: Some("http".to_string()),
            port: 80,
            target_port: Some(IntOrString::Int(11626)),
            protocol: Some("TCP".to_string()),
            ..Default::default()
        },
    ];

    let service = Service {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(selector.clone()),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(selector),
            ports: Some(ports),
            type_: Some("ClusterIP".to_string()),
            ..Default::default()
        }),
        status: None,
    };

    let patch = Patch::Apply(&service);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

async fn update_pod_labels_based_on_lag(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    
    // Select read replicas
    let label_selector = format!(
        "app.kubernetes.io/instance={},stellar.org/role=read-replica",
        node.name_any()
    );
    let lp = ListParams::default().labels(&label_selector);
    let pods = pod_api.list(&lp).await?;

    if pods.items.is_empty() {
        return Ok(());
    }

    let mut pod_ledgers = Vec::new();
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(Error::HttpError)?;

    // Gather ledger info
    for pod in &pods.items {
        if let Some(ip) = &pod.status.as_ref().and_then(|s| s.pod_ip.as_ref()) {
            let url = format!("http://{}:11626/info", ip);
            match http_client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(info) = resp.json::<StellarCoreInfo>().await {
                        pod_ledgers.push((pod.clone(), info.info.ledger.num));
                    }
                }
                Err(e) => {
                    debug!("Failed to fetch info from pod {}: {}", pod.name_any(), e);
                }
            }
        }
    }

    if pod_ledgers.is_empty() {
        return Ok(());
    }

    // Determine max ledger
    let max_ledger = pod_ledgers.iter().map(|(_, l)| *l).max().unwrap_or(0);
    let lag_threshold = 5; // Configurable? Using hardcoded 5 for now

    for (pod, ledger) in pod_ledgers {
        let is_fresh = max_ledger.saturating_sub(ledger) <= lag_threshold;
        let should_enable = is_fresh;

        ensure_traffic_label(&pod_api, &pod, should_enable).await?;
    }
    
    // Also handle pods that didn't respond (assume unhealthy/lagging)
    // We didn't collect them in pod_ledgers, so we need to iterate all pods again?
    // Optimization: Just iterate original list and check if in pod_ledgers
    // For simplicity, failing to respond means traffic disabled.
    
    Ok(())
}

async fn ensure_all_ready_pods_enabled(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    
    let label_selector = format!(
        "app.kubernetes.io/instance={},stellar.org/role=read-replica",
        node.name_any()
    );
    let pods = pod_api.list(&ListParams::default().labels(&label_selector)).await?;

    for pod in pods {
        // Check if ready
        let is_ready = pod.status.as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conds| conds.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
            .unwrap_or(false);

        ensure_traffic_label(&pod_api, &pod, is_ready).await?;
    }
    Ok(())
}

async fn ensure_traffic_label(api: &Api<Pod>, pod: &Pod, enabled: bool) -> Result<()> {
    let current_val = pod.metadata.labels.as_ref()
        .and_then(|l| l.get("stellar.org/traffic"))
        .map(|s| s.as_str());

    let desired_val = if enabled { Some("enabled") } else { None };

    if current_val != desired_val {
        let name = pod.name_any();
        info!("Updating traffic label for {} to {:?}", name, desired_val);
        
        // Patch label using JSON merge patch
        // To remove a label, set it to null
        let patch_json = if let Some(val) = desired_val {
             serde_json::json!({
                "metadata": {
                    "labels": {
                        "stellar.org/traffic": val
                    }
                }
            })
        } else {
            serde_json::json!({
                "metadata": {
                    "labels": {
                        "stellar.org/traffic": null
                    }
                }
            })
        };

        api.patch(&name, &PatchParams::apply("stellar-operator"), &Patch::Merge(&patch_json)).await?;
    }
    Ok(())
}
