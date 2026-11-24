use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{Api, ApiResource, DynamicObject, ListParams, Patch, PatchParams, PostParams};
use kube::core::GroupVersionKind;
use kube::Client;
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// The label selector for identifying Stellar validator pods.
const STELLAR_LABEL: &str = "app=stellar";
/// The name of the PodDisruptionBudget resource managed by this controller.
const PDB_NAME: &str = "stellar-pdb";
/// Default interval between reconcile loops if not configured.
const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 30;

/// A reconciler that dynamically adjusts the PodDisruptionBudget based on
/// real-time health of the Stellar nodes.
pub struct PdbReconciler {
    client: Client,
    namespace: String,
}

impl PdbReconciler {
    /// Create a new reconciler using the given kubernetes client.
    pub fn new(client: Client) -> Self {
        let namespace = env::var("NAMESPACE").unwrap_or_else(|_| "default".to_string());
        Self { client, namespace }
    }

    /// Run the controller loop. This function blocks forever, periodically
    /// reconciling the PodDisruptionBudget.
    pub async fn run(&self) {
        let interval = env::var("RECONCILE_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_RECONCILE_INTERVAL_SECS);
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        loop {
            ticker.tick().await;
            info!("Reconciling PDB");
            if let Err(e) = self.reconcile().await {
                error!("Reconcile failed: {:?}", e);
            }
        }
    }

    /// Perform a single reconciliation: inspect Stellar pods, compute the
    /// desired maxUnavailable, and create/update the PDB accordingly.
    async fn reconcile(&self) -> Result<(), kube::Error> {
        debug!("Listing Stellar pods with label {}", STELLAR_LABEL);
        let pods_api: Api<Pod> = Api::all(self.client.clone());
        let lp = ListParams::default().labels(STELLAR_LABEL);
        let pods = pods_api.list(&lp).await?;

        let mut any_syncing = false;
        let mut ready_count = 0;
        for pod in pods {
            // Check sync status annotation
            if let Some(annotations) = pod.metadata.annotations.as_ref() {
                if annotations.get("stellar-sync-status").map(|v| v.as_str()) == Some("syncing") {
                    any_syncing = true;
                    warn!("Pod {} is currently syncing; PDB will block evictions", pod.metadata.name.as_deref().unwrap_or("<unknown>"));
                }
            }
            // Count ready pods (phase = Running and ready condition true)
            if let Some(status) = pod.status.as_ref() {
                if status.phase.as_deref() == Some("Running") {
                    if let Some(conditions) = status.conditions.as_ref() {
                        let ready = conditions.iter().any(|c| {
                            c.type_.as_str() == "Ready"
                                && c.status.as_str() == "True"
                        });
                        if ready {
                            ready_count += 1;
                        }
                    } else {
                        // If no conditions, assume ready if Running
                        ready_count += 1;
                    }
                }
            }
        }

        // Determine desired maxUnavailable based on quorum safety.
        // If any node is syncing, we must not allow evictions.
        let desired_max_unavailable = if any_syncing {
            0
        } else {
            // TODO: Implement full quorum calculation. For now, allow at
            // most one unreplicated pod to be removed when we have at least 2 ready.
            if ready_count >= 2 {
                1
            } else {
                0
            }
        };

        debug!(
            "Computed desired maxUnavailable: {} (sync: {}, ready: {})",
            desired_max_unavailable, any_syncing, ready_count
        );

        let pdb_api: Api<PodDisruptionBudget> = Api::namespaced(self.client.clone(), &self.namespace);

        // Build the selector used for the PDB always targets the stellar labels
        let selector = LabelSelector {
            match_labels: Some(BTreeMap::from([("app".to_string(), "stellar".to_string())])),
            ..Default::default()
        };

        match pdb_api.get(PDB_NAME).await {
            Ok(_pdb) => {
                // Update existing PDB with server-side apply
                info!("Updating existing PDB {} with maxUnavailable={}", PDB_NAME, desired_max_unavailable);
                let patch = json!({
                    "spec": {
                        "maxUnavailable": desired_max_unavailable
                    }
                });
                let pp = PatchParams::apply("pdb-auto-tuner").force();
                let apply = Patch::Apply(&patch);
                pdb_api.patch(PDB_NAME, &pp, &apply).await?;
            }
            Err(kube::Error::Api(api_err)) if api_err.code == 404 => {
                // Create new PDB
                info!("Creating PDB {} with maxUnavailable={}", PDB_NAME, desired_max_unavailable);
                let pdb = PodDisruptionBudget {
                    metadata: ObjectMeta {
                        name: Some(PDB_NAME.to_string()),
                        namespace: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    spec: Some(k8s_openapi::api::policy::v1::PodDisruptionBudgetSpec {
                        min_available: None,
                        max_unavailable: Some(IntOrString::Int(desired_max_unavailable)),
                        unhealthy_pod_eviction_policy: None,
                        selector: Some(selector),
                    }),
                    status: None,
                };
                pdb_api.create(&PostParams::default(), &pdb).await?;
            }
            Err(e) => {
                warn!("Failed to fetch PDB {}: {:?}", PDB_NAME, e);
                return Err(e);
            }
        }

        Ok(())
    }
}

/// Returns true when the `Synced` status condition of a `StellarNode`
/// object is `"True"`.
fn is_node_synced(node: &DynamicObject) -> bool {
    node.data
        .pointer("/status/conditions")
        .and_then(|conditions| conditions.as_array())
        .map(|conditions| {
            conditions.iter().any(|condition| {
                condition.pointer("/type").and_then(|t| t.as_str()) == Some("Synced")
                    && condition.pointer("/status").and_then(|s| s.as_str()) == Some("True")
            })
        })
        .unwrap_or(false)
}

/// Ensures the per-node `PodDisruptionBudget` (`{node}-pdb`) carries the
/// given `maxUnavailable` budget, creating or patching it as needed.
async fn ensure_node_pdb(
    client: &Client,
    namespace: &str,
    node_name: &str,
    max_unavailable: i32,
) -> Result<(), kube::Error> {
    let pdb_api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), namespace);
    let pdb_name = format!("{node_name}-pdb");
    let mut selector_labels = BTreeMap::new();
    selector_labels.insert("stellar.org/node".to_string(), node_name.to_string());

    match pdb_api.get(&pdb_name).await {
        Ok(_) => {
            let patch = json!({ "spec": { "maxUnavailable": max_unavailable } });
            let pp = PatchParams::apply("pdb-auto-tuner").force();
            pdb_api.patch(&pdb_name, &pp, &Patch::Apply(&patch)).await?;
        }
        Err(kube::Error::Api(api_err)) if api_err.code == 404 => {
            let pdb = PodDisruptionBudget {
                metadata: ObjectMeta {
                    name: Some(pdb_name),
                    namespace: Some(namespace.to_string()),
                    ..Default::default()
                },
                spec: Some(k8s_openapi::api::policy::v1::PodDisruptionBudgetSpec {
                        min_available: None,
                        max_unavailable: Some(IntOrString::Int(max_unavailable)),
                        unhealthy_pod_eviction_policy: None,
                        selector: Some(LabelSelector {
                        match_labels: Some(selector_labels),
                        ..Default::default()
                    }),
                }),
                status: None,
            };
            pdb_api.create(&PostParams::default(), &pdb).await?;
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Reconcile every `StellarNode` in `namespace`: nodes that are still
/// syncing get a `maxUnavailable` budget of `0` so drains block until
/// catch-up completes, while synced nodes tolerate a single eviction as
/// long as quorum capacity allows it.
pub async fn reconcile(client: &Client, namespace: &str) -> Result<(), kube::Error> {
    let gvk = GroupVersionKind::gvk("stellar.org", "v1", "StellarNode");
    let api_resource = ApiResource::from_gvk(&gvk);
    let nodes: Api<DynamicObject> =
        Api::namespaced_with(client.clone(), namespace, &api_resource);
    let node_list = nodes.list(&ListParams::default()).await?;
    let node_count = node_list.items.len();

    for node in node_list {
        let Some(node_name) = node.metadata.name.clone() else {
            continue;
        };
        let syncing = !is_node_synced(&node);
        let max_unavailable =
            super::health::max_unavailable(syncing, node_count, 1);
        debug!(
            "Node {} syncing={} maxUnavailable={}",
            node_name, syncing, max_unavailable
        );
        ensure_node_pdb(client, namespace, &node_name, max_unavailable).await?;
    }
    Ok(())
}
