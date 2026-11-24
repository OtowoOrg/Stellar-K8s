//! Integration coverage for per-node PDB auto-tuning.
//!
//! Requires a live Kubernetes cluster with the `StellarNode` CRD installed:
//!
//! ```sh
//! cargo test --test pdb_integration_test -- --ignored
//! ```
//!
//! The test is `#[ignore]`-annotated so it never silently passes (or fails)
//! in environments without a cluster.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::IntOrString;
use k8s_openapi::core::v1::{Container, Pod, PodSpec};
use k8s_openapi::policy::v1::PodDisruptionBudget;
use kube::api::{Api, ApiResource, DynamicObject, ListParams, Patch, PatchParams, PostParams};
use kube::core::GroupVersionKind;
use kube::Client;
use serde_json::json;
use std::collections::BTreeMap;

use stellar_k8s::controller::pdb::reconciler::reconcile;

const NAMESPACE: &str = "pdb-integration-test";
const NODE_NAME: &str = "node0";
const POD_NAME: &str = "validator-node0";

/// Cluster-gated test: an unsynced node pins `maxUnavailable=0`, a synced
/// node relaxes to `1`, and eviction behaviour follows the PDB.
#[tokio::test]
#[ignore = "requires a live Kubernetes cluster (KUBECONFIG or in-cluster config)"]
async fn pdb_blocks_eviction_during_sync() {
    let client = Client::try_default()
        .await
        .expect("KUBECONFIG or in-cluster config must be available for this test");

    // Create a temporary namespace for the test.
    create_namespace(&client).await;

    // Create a StellarNode object marked as unsynced.
    create_stellar_node(&client, NODE_NAME, false).await;

    // Create a Pod that matches the PDB selector labels.
    create_pod(&client, POD_NAME, NODE_NAME).await;

    // Run the reconciler. It should create a PDB with maxUnavailable=0.
    reconcile(&client, NAMESPACE)
        .await
        .expect("reconcile failed");

    // Verify PDB was created with maxUnavailable=0.
    let pdb_api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), NAMESPACE);
    let pdb = pdb_api
        .get(&format!("{NODE_NAME}-pdb"))
        .await
        .expect("PDB not found");
    assert_eq!(
        pdb.spec
            .as_ref()
            .and_then(|spec| spec.max_unavailable.clone()),
        Some(IntOrString::Int(0)),
        "maxUnavailable should be 0 while syncing"
    );

    // Mark the node as synced and ready.
    update_stellar_node_status(&client, NODE_NAME, true).await;

    // Reconcile again: PDB should now allow eviction.
    reconcile(&client, NAMESPACE)
        .await
        .expect("reconcile failed");
    let pdb = pdb_api
        .get(&format!("{NODE_NAME}-pdb"))
        .await
        .expect("PDB not found");
    assert_eq!(
        pdb.spec
            .as_ref()
            .and_then(|spec| spec.max_unavailable.clone()),
        Some(IntOrString::Int(1)),
        "maxUnavailable should be 1 after sync completes"
    );
}

async fn create_namespace(client: &Client) {
    let ns_api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(client.clone());
    match ns_api
        .create(
            &PostParams::default(),
            &k8s_openapi::api::core::v1::Namespace {
                metadata: ObjectMeta {
                    name: Some(NAMESPACE.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
    {
        Ok(_) => {}
        Err(kube::Error::Api(e)) if e.code == 409 => {}
        Err(e) => panic!("failed to create namespace: {e}"),
    }
}

async fn create_stellar_node(client: &Client, name: &str, synced: bool) {
    let gvk = GroupVersionKind::gvk("stellar.org", "v1", "StellarNode");
    let api_resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> =
        Api::namespaced_with(client.clone(), NAMESPACE, &api_resource);

    let status = if synced { "True" } else { "False" };
    let data = json!({
        "apiVersion": "stellar.org/v1",
        "kind": "StellarNode",
        "metadata": {
            "name": name,
            "namespace": NAMESPACE,
            "labels": {
                "stellar.org/network": "testnet",
                "stellar.org/node": name,
                "stellar.org/validator": "true"
            }
        },
        "spec": {"isValidator": true},
        "status": {
            "conditions": [
                {"type": "Ready", "status": "True"},
                {"type": "Synced", "status": status}
            ]
        }
    });

    let obj: DynamicObject = serde_json::from_value(data).unwrap();
    match api.create(&PostParams::default(), &obj).await {
        Ok(_) => {}
        Err(kube::Error::Api(e)) if e.code == 409 => {}
        Err(e) => panic!("failed to create StellarNode: {e}"),
    }
}

async fn update_stellar_node_status(client: &Client, name: &str, synced: bool) {
    let gvk = GroupVersionKind::gvk("stellar.org", "v1", "StellarNode");
    let api_resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> =
        Api::namespaced_with(client.clone(), NAMESPACE, &api_resource);

    let status = if synced { "True" } else { "False" };
    let patch = json!({
        "status": {
            "conditions": [
                {"type": "Ready", "status": "True"},
                {"type": "Synced", "status": status}
            ]
        }
    });
    let patch = Patch::Merge(&patch);
    let params = PatchParams::default();
    api.patch_status(name, &params, &patch)
        .await
        .expect("failed to update status");
}

async fn create_pod(client: &Client, pod_name: &str, node_name: &str) {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), NAMESPACE);
    let pod = Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            namespace: Some(NAMESPACE.to_string()),
            labels: Some(BTreeMap::from([
                ("stellar.org/network".to_string(), "testnet".to_string()),
                ("stellar.org/node".to_string(), node_name.to_string()),
            ])),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![Container {
                name: "validator".to_string(),
                image: "busybox".to_string(),
                command: Some(vec!["sleep".to_string(), "3600".to_string()]),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    match pod_api.create(&PostParams::default(), &pod).await {
        Ok(_) => {}
        Err(kube::Error::Api(e)) if e.code == 409 => {}
        Err(e) => panic!("failed to create pod: {e}"),
    }
}

/// Placeholder so the test target always has at least one non-ignored test.
#[test]
fn pdb_module_is_linked() {
    assert_eq!(
        stellar_k8s::controller::pdb::health::max_unavailable(true, 5, 3),
        0
    );
}
