use crate::cli::InfoArgs;
use kube::ResourceExt;
use stellar_k8s::crd::StellarNode;
use stellar_k8s::infra;
use stellar_k8s::Error;

pub async fn run_info(args: InfoArgs) -> Result<(), Error> {
    use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
    use k8s_openapi::api::core::v1::Service;

    // Initialize Kubernetes client
    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    let api: kube::Api<StellarNode> = kube::Api::namespaced(client.clone(), &args.namespace);
    let nodes = api
        .list(&Default::default())
        .await
        .map_err(Error::KubeError)?;

    println!("Managed Stellar Nodes: {}", nodes.items.len());
    println!();

    // Display detailed information for each node
    for node in &nodes.items {
        let name = node.name_any();
        let node_type = format!("{:?}", node.spec.node_type);
        let network = format!("{:?}", node.spec.network);
        let replicas = node.spec.replicas;

        println!("StellarNode: {name}");
        println!("  Type: {node_type}");
        println!("  Network: {network}");
        println!("  Replicas: {replicas}");

        // Find owned Deployments
        let deployment_api: kube::Api<Deployment> =
            kube::Api::namespaced(client.clone(), &args.namespace);
        let label_selector =
            format!("app.kubernetes.io/instance={name},app.kubernetes.io/name=stellar-node");
        let deployments = deployment_api
            .list(&kube::api::ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        if !deployments.items.is_empty() {
            println!("  Deployments:");
            for deployment in &deployments.items {
                let dep_name = deployment.metadata.name.as_deref().unwrap_or("unknown");
                let ready = deployment
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                let desired = deployment
                    .spec
                    .as_ref()
                    .and_then(|s| s.replicas)
                    .unwrap_or(0);
                println!("    - {dep_name} ({ready}/{desired} ready)");
            }
        }

        // Find owned StatefulSets
        let statefulset_api: kube::Api<StatefulSet> =
            kube::Api::namespaced(client.clone(), &args.namespace);
        let statefulsets = statefulset_api
            .list(&kube::api::ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        if !statefulsets.items.is_empty() {
            println!("  StatefulSets:");
            for sts in &statefulsets.items {
                let sts_name = sts.metadata.name.as_deref().unwrap_or("unknown");
                let ready = sts
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                let desired = sts.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
                println!("    - {sts_name} ({ready}/{desired} ready)");
            }
        }

        // Find owned Services
        let service_api: kube::Api<Service> =
            kube::Api::namespaced(client.clone(), &args.namespace);
        let services = service_api
            .list(&kube::api::ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        if !services.items.is_empty() {
            println!("  Services:");
            for service in &services.items {
                let svc_name = service.metadata.name.as_deref().unwrap_or("unknown");
                let svc_type = service
                    .spec
                    .as_ref()
                    .and_then(|s| s.type_.as_deref())
                    .unwrap_or("ClusterIP");
                let cluster_ip = service
                    .spec
                    .as_ref()
                    .and_then(|s| s.cluster_ip.as_deref())
                    .unwrap_or("None");
                println!("    - {svc_name} (type: {svc_type}, IP: {cluster_ip})");
            }
        }

        match infra::resolve_stellar_node_infra(&client, node).await {
            Ok(summary) if !summary.is_empty() => {
                println!("  Infra Details:");
                println!(
                    "    Hardware Generation: {}",
                    summary.hardware_generation_label()
                );

                for assignment in &summary.assignments {
                    let kube_node = assignment.kubernetes_node.as_deref().unwrap_or("pending");
                    println!(
                        "    - Pod {} on {} ({})",
                        assignment.pod_name, kube_node, assignment.hardware_generation
                    );

                    if assignment.feature_labels.is_empty() {
                        println!("      feature.node.kubernetes.io/* labels: none found");
                    } else {
                        println!("      feature.node.kubernetes.io/* labels:");
                        for (key, value) in &assignment.feature_labels {
                            println!("        - {key}={value}");
                        }
                    }
                }
            }
            Ok(_) => {
                println!("  Infra Details:");
                println!("    Hardware Generation: unknown");
                println!("    Pods are not scheduled yet, so node feature labels are unavailable.");
            }
            Err(err) => {
                println!("  Infra Details:");
                println!("    Hardware Generation: unknown");
                println!("    Failed to inspect Kubernetes node labels: {err}");
            }
        }

        println!();
    }

    Ok(())
}
