use anyhow::{Context, Result};
use futures::StreamExt;
use k8s_openapi::api::core::v1::{Event, ObjectReference, Pod};
use kube::{
    api::{Api, LogParams, ObjectMeta, Patch, PatchParams, PostParams},
    Client,
};
use serde_json::json;
use std::env;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    // Init logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    info!("Starting Stellar-K8s Crash Loop Analysis sidecar");

    let namespace = env::var("NAMESPACE").context("NAMESPACE env var not set")?;
    let pod_name = env::var("POD_NAME").context("POD_NAME env var not set")?;
    let container_name =
        env::var("CONTAINER_NAME").unwrap_or_else(|_| "stellar-operator".to_string());

    let client = Client::try_default().await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let events: Api<Event> = Api::namespaced(client.clone(), &namespace);

    // Initial check for previous logs if the pod is already crashing
    if let Err(e) =
        analyze_previous_logs(&pods, &events, &pod_name, &namespace, &container_name).await
    {
        warn!("Failed to analyze previous logs: {}", e);
    }

    loop {
        info!("Watching logs for container: {}", container_name);

        let log_params = LogParams {
            container: Some(container_name.clone()),
            follow: true,
            tail_lines: Some(10),
            ..LogParams::default()
        };

        match pods.log_stream(&pod_name, &log_params).await {
            Ok(stream) => {
                use futures::AsyncBufReadExt;
                let mut lines = stream.lines();
                while let Some(line) = lines.next().await {
                    match line {
                        Ok(log_line) => {
                            if let Some(recommendation) = analyze_log(&log_line) {
                                info!(
                                    "Found issue: {}. Recommendation: {}",
                                    log_line.trim(),
                                    recommendation
                                );
                                if let Err(e) = report_recommendation(
                                    &pods,
                                    &events,
                                    &pod_name,
                                    &namespace,
                                    recommendation,
                                )
                                .await
                                {
                                    error!("Failed to report recommendation: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Error reading log stream: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to open log stream: {}. Retrying in 5s...", e);
            }
        }

        // If the stream broke, check if we crashed and if there are logs from the previous run
        if let Err(e) =
            analyze_previous_logs(&pods, &events, &pod_name, &namespace, &container_name).await
        {
            debug!("No previous logs to analyze or failed: {}", e);
        }

        sleep(Duration::from_secs(5)).await;
    }
}

async fn analyze_previous_logs(
    pods: &Api<Pod>,
    events: &Api<Event>,
    pod_name: &str,
    namespace: &str,
    container_name: &str,
) -> Result<()> {
    let log_params = LogParams {
        container: Some(container_name.to_string()),
        previous: true,
        tail_lines: Some(50),
        ..LogParams::default()
    };

    match pods.logs(pod_name, &log_params).await {
        Ok(logs) => {
            for line in logs.lines() {
                if let Some(recommendation) = analyze_log(line) {
                    info!(
                        "Found issue in previous logs: {}. Recommendation: {}",
                        line.trim(),
                        recommendation
                    );
                    report_recommendation(pods, events, pod_name, namespace, recommendation)
                        .await?;
                    break; // Just report the first one found in previous logs
                }
            }
        }
        Err(_) => {
            // This is expected if there's no previous container
        }
    }
    Ok(())
}

fn analyze_log(line: &str) -> Option<&'static str> {
    let line_lower = line.to_lowercase();
    if line_lower.contains("connection refused") {
        Some("Check your NetworkPolicies or service reachability.")
    } else if line_lower.contains("forbidden")
        || line_lower.contains("rbac")
        || line_lower.contains("permission denied")
    {
        Some("Check your RBAC permissions (ClusterRole/RoleBinding).")
    } else if line_lower.contains("timeout")
        || line_lower.contains("timed out")
        || line_lower.contains("deadline exceeded")
    {
        Some("Check API server latency or network connectivity.")
    } else if line_lower.contains("database")
        || line_lower.contains("postgresql")
        || line_lower.contains("sqlx")
    {
        Some("Check database connectivity and credentials.")
    } else if line_lower.contains("configmap") && line_lower.contains("not found") {
        Some("Ensure the required ConfigMap exists.")
    } else if line_lower.contains("secret") && line_lower.contains("not found") {
        Some("Ensure the required Secret exists.")
    } else {
        None
    }
}

async fn report_recommendation(
    pods: &Api<Pod>,
    events: &Api<Event>,
    pod_name: &str,
    namespace: &str,
    recommendation: &str,
) -> Result<()> {
    // 1. Update annotation
    let patch = json!({
        "metadata": {
            "annotations": {
                "stellar.io/fix-recommendation": recommendation
            }
        }
    });
    pods.patch(pod_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;

    // 2. Create event
    let now = chrono::Utc::now();
    let event = Event {
        metadata: ObjectMeta {
            generate_name: Some(format!("{}-fix-", pod_name)),
            namespace: Some(namespace.to_string()),
            ..ObjectMeta::default()
        },
        involved_object: ObjectReference {
            kind: Some("Pod".to_string()),
            name: Some(pod_name.to_string()),
            namespace: Some(namespace.to_string()),
            ..ObjectReference::default()
        },
        reason: Some("CrashLoopAnalysis".to_string()),
        message: Some(recommendation.to_string()),
        type_: Some("Warning".to_string()),
        first_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
        last_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
        ..Event::default()
    };
    events.create(&PostParams::default(), &event).await?;

    Ok(())
}
