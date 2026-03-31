use chrono::Utc;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use tracing::{debug, info, warn};

use crate::crd::StellarNode;
use crate::error::Result;

use super::cve::{
    create_canary_deployment, delete_canary_deployment, rollback_version, trigger_rolling_update,
    CVERolloutStatus, CanaryTestRunner, CanaryTestStatus, ConsensusHealthMonitor,
    RegistryScannerClient, CANARY_DEPLOYMENT_ANNOTATION, CANARY_TEST_STATUS_ANNOTATION,
    CVE_AUTO_PATCH_ANNOTATION, CVE_DETECTED_ANNOTATION, CVE_PATCHED_VERSION_ANNOTATION,
    CVE_ROLLBACK_REASON_ANNOTATION, CVE_ROLLOUT_STATUS_ANNOTATION, CVE_SCAN_TIME_ANNOTATION,
    CVE_VULNERABLE_IMAGE_ANNOTATION,
};
use crate::crd::CVEHandlingConfig;

/// Check if auto-patch is enabled via annotation (safety gate)
/// Defaults to true if annotation is not present
fn is_auto_patch_enabled(node: &StellarNode) -> bool {
    node.metadata
        .annotations
        .as_ref()
        .and_then(|ann| ann.get(CVE_AUTO_PATCH_ANNOTATION))
        .map(|v| v == "true" || v == "enabled")
        .unwrap_or(true) // Default to enabled if annotation not present
}

/// Handle CVE scanning and patching during reconciliation
pub async fn reconcile_cve_patches(
    client: &Client,
    node: &StellarNode,
    config: &CVEHandlingConfig,
) -> Result<()> {
    if !config.enabled {
        debug!(
            "CVE handling disabled for {}/{}",
            node.namespace().unwrap_or_else(|| "default".to_string()),
            node.name_any()
        );
        return Ok(());
    }

    // Safety gate: Check opt-in/out annotation
    if !is_auto_patch_enabled(node) {
        debug!(
            "CVE auto-patch disabled via annotation for {}/{}",
            node.namespace().unwrap_or_else(|| "default".to_string()),
            node.name_any()
        );
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Check if we should scan (based on last scan time)
    if should_scan(node, config) {
        info!("Starting CVE scan for {}/{}", namespace, name);
        scan_and_initiate_patch(client, node, config).await?;
    }

    // Check status of ongoing CVE patch operations
    check_cve_patch_status(client, node, config).await?;

    // Monitor consensus health if rolling out patched version
    if is_rolling_out_patch(node) {
        monitor_consensus_during_rollout(client, node, config).await?;
    }

    Ok(())
}

/// Check if image should be scanned for CVEs
fn should_scan(node: &StellarNode, config: &CVEHandlingConfig) -> bool {
    let annotations = node.metadata.annotations.as_ref();

    // Get last scan time
    let last_scan = annotations
        .and_then(|ann| ann.get(CVE_SCAN_TIME_ANNOTATION))
        .and_then(|ts| ts.parse::<i64>().ok())
        .and_then(|ts| {
            let nanos = ((ts % 1000) * 1_000_000) as u32;
            chrono::DateTime::<Utc>::from_timestamp(ts / 1000, nanos)
        });

    if let Some(last_scan_time) = last_scan {
        let elapsed = Utc::now()
            .signed_duration_since(last_scan_time)
            .num_seconds() as u64;

        if elapsed < config.scan_interval_secs {
            return false;
        }
    }

    true
}

/// Scan image and initiate patch if CVEs found
async fn scan_and_initiate_patch(
    client: &Client,
    node: &StellarNode,
    config: &CVEHandlingConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // In production, use actual registry scanner endpoint
    // For now, use a mock endpoint
    let scanner =
        RegistryScannerClient::new("http://trivy-api.security-scanning:8080".to_string(), None);

    let image = get_node_image(client, node).await?;
    debug!("Scanning image for CVEs: {}", image);

    let scan_result = scanner.scan_image(&image).await?;

    // Update scan timestamp
    let mut annotations = node.metadata.annotations.clone().unwrap_or_default();
    annotations.insert(
        CVE_SCAN_TIME_ANNOTATION.to_string(),
        Utc::now().timestamp().to_string(),
    );

    // Check if we should proceed with patching
    if scan_result.has_critical && config.critical_only {
        // Only patch critical vulnerabilities
        if !scan_result
            .vulnerabilities
            .iter()
            .any(|v| v.severity as i32 >= 4)
        {
            info!(
                "CVE scan for {}/{} found vulnerabilities but no critical ones",
                namespace, name
            );
            annotations.insert(CVE_DETECTED_ANNOTATION.to_string(), "false".to_string());
            update_node_annotations(client, node, annotations).await?;
            return Ok(());
        }
    }

    if !scan_result.can_patch() {
        info!(
            "CVE scan for {}/{} found vulnerabilities but no patched version available",
            namespace, name
        );
        annotations.insert(CVE_DETECTED_ANNOTATION.to_string(), "false".to_string());
        update_node_annotations(client, node, annotations).await?;
        return Ok(());
    }

    // Record CVE detection
    info!(
        "CVEs detected in {}/{}: {} critical, {} high, {} medium, {} low",
        namespace,
        name,
        scan_result.cve_count.critical,
        scan_result.cve_count.high,
        scan_result.cve_count.medium,
        scan_result.cve_count.low
    );

    annotations.insert(CVE_DETECTED_ANNOTATION.to_string(), "true".to_string());
    annotations.insert(
        CVE_VULNERABLE_IMAGE_ANNOTATION.to_string(),
        scan_result.current_image,
    );

    if let Some(ref patched_version) = scan_result.patched_version {
        annotations.insert(
            CVE_PATCHED_VERSION_ANNOTATION.to_string(),
            patched_version.clone(),
        );
    }

    annotations.insert(
        CVE_ROLLOUT_STATUS_ANNOTATION.to_string(),
        CVERolloutStatus::CanaryTesting.as_str().to_string(),
    );

    update_node_annotations(client, node, annotations).await?;

    // Create canary deployment for testing
    if let Some(patched_version) = &scan_result.patched_version {
        initiate_canary_deployment(client, node, patched_version, config).await?;
    }

    Ok(())
}

/// Initiate canary deployment for testing patched version
async fn initiate_canary_deployment(
    client: &Client,
    node: &StellarNode,
    patched_image: &str,
    _config: &CVEHandlingConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!(
        "Creating canary deployment for {}/{} with image {}",
        namespace, name, patched_image
    );

    // Create canary deployment
    let canary_name = create_canary_deployment(client, node, patched_image).await?;

    // Update node annotations to track canary deployment
    let mut annotations = node.metadata.annotations.clone().unwrap_or_default();
    annotations.insert(
        CANARY_DEPLOYMENT_ANNOTATION.to_string(),
        canary_name.clone(),
    );
    annotations.insert(
        CANARY_TEST_STATUS_ANNOTATION.to_string(),
        CanaryTestStatus::Running.as_str().to_string(),
    );

    update_node_annotations(client, node, annotations).await?;

    info!(
        "Canary deployment created: {}/{}/{}",
        namespace, name, canary_name
    );

    Ok(())
}

/// Check status of ongoing CVE patch operations
async fn check_cve_patch_status(
    client: &Client,
    node: &StellarNode,
    config: &CVEHandlingConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let default_annotations = Default::default();
    let annotations = node
        .metadata
        .annotations
        .as_ref()
        .unwrap_or(&default_annotations);

    let canary_status_str = annotations
        .get(CANARY_TEST_STATUS_ANNOTATION)
        .map(|s| s.as_str())
        .unwrap_or("Pending");

    let canary_status = match canary_status_str {
        "Passed" => CanaryTestStatus::Passed,
        "Failed" => CanaryTestStatus::Failed,
        "Timeout" => CanaryTestStatus::Timeout,
        "Running" => CanaryTestStatus::Running,
        _ => CanaryTestStatus::Pending,
    };

    match canary_status {
        CanaryTestStatus::Running => {
            // Check if tests are still running
            if let Some(canary_name) = annotations.get(CANARY_DEPLOYMENT_ANNOTATION) {
                debug!(
                    "Checking canary test status for {}/{}/{}",
                    namespace, name, canary_name
                );

                let result = run_canary_health_checks(client, node).await?;

                if result == CanaryTestStatus::Passed {
                    // Tests passed, initiate rolling update
                    on_canary_test_passed(client, node, config).await?;
                } else if result == CanaryTestStatus::Failed {
                    // Tests failed, mark as failed
                    on_canary_test_failed(client, node, config).await?;
                }
            }
        }

        CanaryTestStatus::Passed => {
            // Canary tests passed, check if rolling update is complete
            let rollout_status_str = annotations
                .get(CVE_ROLLOUT_STATUS_ANNOTATION)
                .map(|s| s.as_str())
                .unwrap_or("Idle");

            if rollout_status_str == "Complete" {
                // Rollout complete, clean up canary deployment
                if let Some(canary_name) = annotations.get(CANARY_DEPLOYMENT_ANNOTATION) {
                    delete_canary_deployment(client, node, canary_name).await?;
                }
            }
        }

        CanaryTestStatus::Failed => {
            // Canary tests failed, do not proceed with rollout
            warn!(
                "Canary tests failed for {}/{}, will not proceed with patched version rollout",
                namespace, name
            );
        }

        _ => {}
    }

    Ok(())
}

/// Run health checks on canary pod
async fn run_canary_health_checks(client: &Client, node: &StellarNode) -> Result<CanaryTestStatus> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // Get canary pod
    let pods_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let canary_label = format!("stellar.org/cve-canary={}", node.name_any());
    let list_params = ListParams::default().labels(&canary_label);

    let pods = pods_api.list(&list_params).await?;

    if let Some(canary_pod) = pods.items.first() {
        let test_status = CanaryTestRunner::run_tests(client, node, canary_pod).await?;
        return Ok(test_status);
    }

    Ok(CanaryTestStatus::Running)
}

/// Handle successful canary test
async fn on_canary_test_passed(
    client: &Client,
    node: &StellarNode,
    _config: &CVEHandlingConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let default_annotations = Default::default();
    let annotations = node
        .metadata
        .annotations
        .as_ref()
        .unwrap_or(&default_annotations);

    if let Some(patched_version) = annotations.get(CVE_PATCHED_VERSION_ANNOTATION) {
        info!(
            "Canary tests passed for {}/{}, initiating rolling update to {}",
            namespace, name, patched_version
        );

        trigger_rolling_update(client, node, patched_version).await?;

        let mut new_annotations = annotations.clone();
        new_annotations.insert(
            CANARY_TEST_STATUS_ANNOTATION.to_string(),
            CanaryTestStatus::Passed.as_str().to_string(),
        );
        new_annotations.insert(
            CVE_ROLLOUT_STATUS_ANNOTATION.to_string(),
            CVERolloutStatus::Rolling.as_str().to_string(),
        );

        update_node_annotations(client, node, new_annotations).await?;
    }

    Ok(())
}

/// Handle failed canary test
async fn on_canary_test_failed(
    client: &Client,
    node: &StellarNode,
    _config: &CVEHandlingConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    warn!(
        "Canary tests failed for {}/{}, aborting patched version rollout",
        namespace, name
    );

    let default_annotations = Default::default();
    let annotations = node
        .metadata
        .annotations
        .as_ref()
        .unwrap_or(&default_annotations);

    if let Some(canary_name) = annotations.get(CANARY_DEPLOYMENT_ANNOTATION) {
        delete_canary_deployment(client, node, canary_name).await?;
    }

    let mut new_annotations = annotations.clone();
    new_annotations.insert(
        CANARY_TEST_STATUS_ANNOTATION.to_string(),
        CanaryTestStatus::Failed.as_str().to_string(),
    );
    new_annotations.insert(
        CVE_ROLLOUT_STATUS_ANNOTATION.to_string(),
        CVERolloutStatus::Failed.as_str().to_string(),
    );

    update_node_annotations(client, node, new_annotations).await?;

    Ok(())
}

/// Check if node is rolling out a patched version
fn is_rolling_out_patch(node: &StellarNode) -> bool {
    let annotations = node.metadata.annotations.as_ref();

    annotations
        .and_then(|ann| ann.get(CVE_ROLLOUT_STATUS_ANNOTATION))
        .map(|status| status == CVERolloutStatus::Rolling.as_str())
        .unwrap_or(false)
}

/// Monitor consensus health during rollout and rollback if needed
async fn monitor_consensus_during_rollout(
    client: &Client,
    node: &StellarNode,
    config: &CVEHandlingConfig,
) -> Result<()> {
    if !config.enable_auto_rollback {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    debug!(
        "Monitoring consensus health during CVE patch rollout for {}/{}",
        namespace,
        node.name_any()
    );

    // Get baseline consensus health
    let baseline_health = ConsensusHealthMonitor::check_consensus_health(client, node).await?;

    // Check if health has degraded
    let degraded = ConsensusHealthMonitor::detect_degradation(
        client,
        node,
        baseline_health,
        config.consensus_health_threshold,
    )
    .await?;

    if degraded {
        warn!(
            "Consensus health degraded during CVE patch rollout for {}/{}, initiating rollback",
            namespace,
            node.name_any()
        );

        // Get the previous version (vulnerable version before patch)
        let default_annotations = Default::default();
        let annotations = node
            .metadata
            .annotations
            .as_ref()
            .unwrap_or(&default_annotations);
        let vulnerable_image = annotations
            .get(CVE_VULNERABLE_IMAGE_ANNOTATION)
            .cloned()
            .unwrap_or_else(|| node.spec.version.clone());

        // Rollback to previous version
        rollback_version(
            client,
            node,
            &vulnerable_image,
            "Consensus health degraded during patched version rollout",
        )
        .await?;

        // Update annotations
        let mut new_annotations = annotations.clone();
        new_annotations.insert(
            CVE_ROLLOUT_STATUS_ANNOTATION.to_string(),
            CVERolloutStatus::RolledBack.as_str().to_string(),
        );
        new_annotations.insert(
            CVE_ROLLBACK_REASON_ANNOTATION.to_string(),
            format!(
                "Health: {:.2}% < {:.2}%",
                baseline_health * 100.0,
                config.consensus_health_threshold * 100.0
            ),
        );

        update_node_annotations(client, node, new_annotations).await?;
    }

    Ok(())
}

/// Get the image being used by a StellarNode
async fn get_node_image(client: &Client, node: &StellarNode) -> Result<String> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // Get the deployment to extract image info
    let deployments_api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let label_selector = format!("app.kubernetes.io/instance={}", node.name_any());

    match deployments_api
        .list(&ListParams::default().labels(&label_selector))
        .await
    {
        Ok(deployments) => {
            if let Some(deployment) = deployments.items.first() {
                if let Some(spec) = &deployment.spec {
                    if let Some(_metadata) = &spec.template.metadata {
                        if let Some(containers) = &spec
                            .template
                            .spec
                            .as_ref()
                            .and_then(|ps| ps.containers.first())
                        {
                            if let Some(image) = &containers.image {
                                return Ok(image.clone());
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            debug!("Could not fetch deployment image: {}", e);
        }
    }

    // Fallback to using version from spec
    Ok(format!("stellar/core:{}", node.spec.version))
}

/// Update node annotations
async fn update_node_annotations(
    client: &Client,
    node: &StellarNode,
    annotations: std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let nodes_api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let patch = serde_json::json!({
        "metadata": {
            "annotations": annotations
        }
    });

    nodes_api
        .patch(
            &name,
            &PatchParams::apply("cve-handler"),
            &Patch::Merge(patch),
        )
        .await?;

    Ok(())
}
