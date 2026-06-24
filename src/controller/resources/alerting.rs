//! Alerting ConfigMap management.

use super::helpers::*;
use super::prelude::*;

// ============================================================================
// Alerting — unchanged
// ============================================================================

pub async fn ensure_alerting(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    if !node.spec.alerting {
        return delete_alerting(client, node, dry_run).await;
    }

    let labels = standard_labels(node);
    let mut data = BTreeMap::new();

    let rules = format!(
        r#"groups:
- name: {instance}.rules
  rules:
  - alert: StellarNodeDown
    expr: up{{app_kubernetes_io_instance="{instance}"}} == 0
    for: 5m
    labels:
      severity: critical
    annotations:
      summary: "Stellar node {instance} is down"
      description: "The Stellar node {instance} has been down for more than 5 minutes."
  - alert: StellarNodeHighMemory
    expr: container_memory_usage_bytes{{pod=~"{instance}.*"}} / container_spec_memory_limit_bytes > 0.8
    for: 10m
    labels:
      severity: warning
    annotations:
      summary: "Stellar node {instance} high memory usage"
      description: "The Stellar node {instance} is using more than 80% of its memory limit."
  - alert: StellarNodeSyncIssue
    expr: stellar_core_sync_status{{app_kubernetes_io_instance="{instance}"}} != 1
    for: 15m
    labels:
      severity: warning
    annotations:
      summary: "Stellar node {instance} sync issue"
      description: "The Stellar node {instance} has not been in sync for more than 15 minutes."
"#,
        instance = node.name_any()
    );

    data.insert("alerts.yaml".to_string(), rules);

    let cm = ConfigMap {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.clone()),
                namespace: Some(namespace.clone()),
                labels: Some(labels),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        data: Some(data),
        ..Default::default()
    };

    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let patch = Patch::Apply(&cm);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    info!(
        "Alerting ConfigMap {} ensured for {}/{}",
        name,
        namespace,
        node.name_any()
    );
    Ok(())
}

pub(crate) fn build_hpa(node: &StellarNode) -> Result<HorizontalPodAutoscaler> {
    let autoscaling = node
        .spec
        .autoscaling
        .as_ref()
        .ok_or_else(|| Error::ValidationError("Autoscaling config not found".to_string()))?;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "hpa");
    let deployment_name = node.name_any();

    let mut metrics = Vec::new();

    if let Some(target_cpu) = autoscaling.target_cpu_utilization_percentage {
        metrics.push(MetricSpec {
            type_: "Resource".to_string(),
            resource: Some(k8s_openapi::api::autoscaling::v2::ResourceMetricSource {
                name: "cpu".to_string(),
                target: MetricTarget {
                    type_: "Utilization".to_string(),
                    average_utilization: Some(target_cpu),
                    ..Default::default()
                },
            }),
            ..Default::default()
        });
    }

    if let Some(target_mem) = autoscaling.target_memory_utilization_percentage {
        metrics.push(MetricSpec {
            type_: "Resource".to_string(),
            resource: Some(k8s_openapi::api::autoscaling::v2::ResourceMetricSource {
                name: "memory".to_string(),
                target: MetricTarget {
                    type_: "Utilization".to_string(),
                    average_utilization: Some(target_mem),
                    ..Default::default()
                },
            }),
            ..Default::default()
        });
    }

    for metric_name in &autoscaling.custom_metrics {
        let metric = match metric_name.as_str() {
            "ledger_ingestion_lag" => Some((
                "stellar_node_ingestion_lag".to_string(),
                "Value".to_string(),
                Quantity("5".to_string()),
            )),
            "stellar_horizon_tps" | "requests_per_second" => Some((
                "stellar_horizon_tps".to_string(),
                "Value".to_string(),
                Quantity("1000".to_string()),
            )),
            "stellar_queue_length" | "queue_length" | "horizon_queue_length" => Some((
                "stellar_horizon_queue_length".to_string(),
                "Value".to_string(),
                Quantity("50".to_string()),
            )),
            _ => None,
        };

        if let Some((metric_name, target_type, target_value)) = metric {
            metrics.push(MetricSpec {
                type_: "Object".to_string(),
                object: Some(ObjectMetricSource {
                    described_object: CrossVersionObjectReference {
                        api_version: Some("stellar.org/v1alpha1".to_string()),
                        kind: "StellarNode".to_string(),
                        name: node.name_any(),
                    },
                    metric: MetricIdentifier {
                        name: metric_name,
                        selector: None,
                    },
                    target: MetricTarget {
                        type_: target_type,
                        value: Some(target_value),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            });
        } else {
            warn!(
                "Unrecognized custom metric '{}' configured for node {}; skipping.",
                metric_name,
                node.name_any()
            );
        }
    }

    let behavior = autoscaling
        .behavior
        .as_ref()
        .map(|b| HorizontalPodAutoscalerBehavior {
            scale_up: b.scale_up.as_ref().map(|s| HPAScalingRules {
                stabilization_window_seconds: s.stabilization_window_seconds,
                policies: Some(
                    s.policies
                        .iter()
                        .map(|p| HPAScalingPolicy {
                            type_: p.policy_type.clone(),
                            value: p.value,
                            period_seconds: p.period_seconds,
                        })
                        .collect(),
                ),
                select_policy: Some("Max".to_string()),
            }),
            scale_down: b.scale_down.as_ref().map(|s| HPAScalingRules {
                stabilization_window_seconds: s.stabilization_window_seconds,
                policies: Some(
                    s.policies
                        .iter()
                        .map(|p| HPAScalingPolicy {
                            type_: p.policy_type.clone(),
                            value: p.value,
                            period_seconds: p.period_seconds,
                        })
                        .collect(),
                ),
                select_policy: Some("Min".to_string()),
            }),
        });

    let hpa = HorizontalPodAutoscaler {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: Some(namespace),
                labels: Some(standard_labels(node)),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        spec: Some(HorizontalPodAutoscalerSpec {
            scale_target_ref: CrossVersionObjectReference {
                api_version: Some("apps/v1".to_string()),
                kind: "Deployment".to_string(),
                name: deployment_name,
            },
            min_replicas: Some(autoscaling.min_replicas),
            max_replicas: autoscaling.max_replicas,
            metrics: if metrics.is_empty() {
                None
            } else {
                Some(metrics)
            },
            behavior,
        }),
        status: None,
    };

    Ok(hpa)
}

pub async fn delete_hpa(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    if node.spec.autoscaling.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "hpa");

    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => {
            info!("HPA deleted for {}/{}", namespace, name);
        }
        Err(kube::Error::Api(api_err)) if api_err.code == 404 => {
            info!("HPA {}/{} not found (already deleted)", namespace, name);
        }
        Err(e) => {
            warn!("Failed to delete HPA {}/{}: {:?}", namespace, name, e);
        }
    }

    Ok(())
}
