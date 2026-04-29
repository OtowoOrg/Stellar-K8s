//! Proactive disk scaling for PersistentVolumeClaims
//!
//! Monitors disk usage on managed PVCs and automatically expands volumes
//! when usage exceeds configured thresholds, preventing 'Disk Full' outages.
//!
//! # Features
//! - Monitors disk usage percentage on PVCs attached to Stellar nodes
//! - Automatically triggers PVC expansion when usage exceeds threshold (default: 80%)
//! - Respects storage provider expansion limits and capabilities
//! - Logs all expansion events for cost auditing
//! - Emits Kubernetes events and Prometheus metrics
//!
//! # Storage Provider Support
//! - AWS EBS: Supports online expansion (no pod restart required)
//! - GCP Persistent Disks: Supports online expansion
//! - Azure Disks: Supports online expansion
//! - Local storage: Expansion not supported (requires manual intervention)

use crate::controller::resources::resource_name;
use crate::crd::StellarNode;
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use std::collections::BTreeMap;
use tracing::{debug, info, instrument, warn};

/// Default disk usage threshold (percentage) that triggers expansion
pub const DEFAULT_EXPANSION_THRESHOLD: u8 = 80;

/// Default expansion increment (percentage of current size)
pub const DEFAULT_EXPANSION_INCREMENT: u8 = 50;

/// Minimum time between expansion attempts (seconds)
pub const MIN_EXPANSION_INTERVAL_SECS: u64 = 3600; // 1 hour

/// Maximum number of expansions per PVC (safety limit)
pub const MAX_EXPANSIONS_PER_PVC: u32 = 10;

/// Annotation key for tracking expansion count
const EXPANSION_COUNT_ANNOTATION: &str = "stellar.org/disk-expansion-count";

/// Annotation key for tracking last expansion timestamp
const LAST_EXPANSION_ANNOTATION: &str = "stellar.org/last-disk-expansion";

/// Configuration for disk scaling behavior
#[derive(Debug, Clone)]
pub struct DiskScalerConfig {
    /// Disk usage percentage that triggers expansion (0-100)
    pub expansion_threshold: u8,
    /// Percentage to increase disk size by (e.g., 50 = increase by 50%)
    pub expansion_increment: u8,
    /// Minimum time between expansions (seconds)
    pub min_expansion_interval_secs: u64,
    /// Maximum number of expansions allowed per PVC
    pub max_expansions: u32,
    /// Enable automatic disk scaling
    pub enabled: bool,
}

impl Default for DiskScalerConfig {
    fn default() -> Self {
        Self {
            expansion_threshold: DEFAULT_EXPANSION_THRESHOLD,
            expansion_increment: DEFAULT_EXPANSION_INCREMENT,
            min_expansion_interval_secs: MIN_EXPANSION_INTERVAL_SECS,
            max_expansions: MAX_EXPANSIONS_PER_PVC,
            enabled: true,
        }
    }
}

/// Disk usage information for a PVC
#[derive(Debug, Clone)]
pub struct DiskUsage {
    /// Total capacity in bytes
    pub capacity_bytes: u64,
    /// Used space in bytes
    pub used_bytes: u64,
    /// Usage percentage (0-100)
    pub usage_percent: u8,
}

impl DiskUsage {
    /// Calculate usage percentage
    pub fn calculate_percent(used: u64, capacity: u64) -> u8 {
        if capacity == 0 {
            return 0;
        }
        ((used as f64 / capacity as f64) * 100.0).min(100.0) as u8
    }
}

/// Result of a disk scaling operation
#[derive(Debug, Clone)]
pub enum ScalingResult {
    /// No action needed (usage below threshold)
    NoActionNeeded,
    /// Expansion triggered successfully
    Expanded {
        old_size: String,
        new_size: String,
        expansion_count: u32,
    },
    /// Expansion skipped due to rate limiting
    RateLimited {
        last_expansion: String,
        next_allowed: String,
    },
    /// Expansion skipped due to max expansions reached
    MaxExpansionsReached { count: u32 },
    /// Expansion not supported by storage class
    NotSupported { storage_class: String },
    /// Expansion failed
    Failed { reason: String },
}

/// Check if a storage class supports volume expansion
#[instrument(skip(client), fields(storage_class = %storage_class_name))]
pub async fn supports_expansion(client: &Client, storage_class_name: &str) -> Result<bool> {
    use k8s_openapi::api::storage::v1::StorageClass;

    let api: Api<StorageClass> = Api::all(client.clone());

    match api.get(storage_class_name).await {
        Ok(sc) => {
            let supports = sc.allow_volume_expansion.unwrap_or(false);
            debug!(
                "StorageClass {} supports expansion: {}",
                storage_class_name, supports
            );
            Ok(supports)
        }
        Err(e) => {
            warn!("Failed to get StorageClass {}: {}", storage_class_name, e);
            // Assume expansion is not supported if we can't verify
            Ok(false)
        }
    }
}

/// Get disk usage for a PVC by querying the pod's metrics
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn get_disk_usage(client: &Client, node: &StellarNode) -> Result<Option<DiskUsage>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    // Find the pod for this node
    let label_selector = format!("app.kubernetes.io/name={}", node.name_any());
    let pods = pod_api
        .list(&kube::api::ListParams::default().labels(&label_selector))
        .await?;

    if pods.items.is_empty() {
        debug!("No pods found for node {}", node.name_any());
        return Ok(None);
    }

    let pod = &pods.items[0];
    let pod_name = pod.name_any();

    // Execute df command in the pod to get disk usage
    // The Stellar data is typically mounted at /data
    let exec_result = execute_df_command(client, &namespace, &pod_name).await?;

    if let Some(usage) = exec_result {
        debug!(
            "Disk usage for {}: {}% ({}/{} bytes)",
            node.name_any(),
            usage.usage_percent,
            usage.used_bytes,
            usage.capacity_bytes
        );
        Ok(Some(usage))
    } else {
        Ok(None)
    }
}

/// Execute df command in a pod to get disk usage
async fn execute_df_command(
    client: &Client,
    namespace: &str,
    pod_name: &str,
) -> Result<Option<DiskUsage>> {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);

    // Get pod to find container name
    let pod = pod_api.get(pod_name).await?;
    let container_name = pod
        .spec
        .as_ref()
        .and_then(|spec| spec.containers.first())
        .map(|c| c.name.as_str())
        .unwrap_or("stellar-core");

    // Execute df command to get disk usage for /data mount
    let exec_params = kube::api::AttachParams::default()
        .container(container_name)
        .stdin(false)
        .stdout(true)
        .stderr(true);

    let command = vec!["df", "-B1", "/data"];

    match pod_api.exec(pod_name, command, &exec_params).await {
        Ok(mut attached) => {
            let stdout = tokio_util::io::ReaderStream::new(attached.stdout().unwrap());
            use futures::StreamExt;
            let output: Vec<_> = stdout.collect().await;

            // Parse df output
            let output_bytes: Vec<u8> = output.into_iter().flatten().flatten().collect();
            let output_str = String::from_utf8_lossy(&output_bytes);

            parse_df_output(&output_str)
        }
        Err(e) => {
            warn!("Failed to execute df command in pod {}: {}", pod_name, e);
            Ok(None)
        }
    }
}

/// Parse df command output to extract disk usage
pub(crate) fn parse_df_output(output: &str) -> Result<Option<DiskUsage>> {
    // df output format:
    // Filesystem     1B-blocks      Used Available Use% Mounted on
    // /dev/xvda1   1610612736000 644245094400 966367641600  40% /data

    for line in output.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 6 && parts[5] == "/data" {
            let capacity_bytes: u64 = parts[1].parse().unwrap_or(0);
            let used_bytes: u64 = parts[2].parse().unwrap_or(0);
            let usage_percent = DiskUsage::calculate_percent(used_bytes, capacity_bytes);

            return Ok(Some(DiskUsage {
                capacity_bytes,
                used_bytes,
                usage_percent,
            }));
        }
    }

    Ok(None)
}

/// Check if expansion is allowed based on rate limiting and max expansions
fn check_expansion_allowed(pvc: &PersistentVolumeClaim, config: &DiskScalerConfig) -> Result<bool> {
    let annotations = pvc.metadata.annotations.as_ref();

    // Check expansion count
    if let Some(annotations) = annotations {
        if let Some(count_str) = annotations.get(EXPANSION_COUNT_ANNOTATION) {
            let count: u32 = count_str.parse().unwrap_or(0);
            if count >= config.max_expansions {
                return Ok(false);
            }
        }

        // Check last expansion time
        if let Some(last_expansion_str) = annotations.get(LAST_EXPANSION_ANNOTATION) {
            if let Ok(last_expansion) = chrono::DateTime::parse_from_rfc3339(last_expansion_str) {
                let now = chrono::Utc::now();
                let elapsed = now.signed_duration_since(last_expansion.with_timezone(&chrono::Utc));

                if elapsed.num_seconds() < config.min_expansion_interval_secs as i64 {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}

/// Calculate new size for PVC expansion
pub(crate) fn calculate_new_size(current_size: &str, increment_percent: u8) -> Result<String> {
    // Parse current size (e.g., "100Gi", "1500Gi")
    let current_bytes = parse_quantity_to_bytes(current_size)?;

    // Calculate new size
    let increment_bytes = (current_bytes as f64 * (increment_percent as f64 / 100.0)) as u64;
    let new_bytes = current_bytes + increment_bytes;

    // Convert back to human-readable format
    Ok(format_bytes_to_quantity(new_bytes))
}

/// Parse Kubernetes quantity string to bytes
pub(crate) fn parse_quantity_to_bytes(quantity: &str) -> Result<u64> {
    let q = Quantity(quantity.to_string());

    // Extract numeric value and unit
    let s = q.0.as_str();
    let (num_str, unit) = if let Some(pos) = s.find(|c: char| c.is_alphabetic()) {
        (&s[..pos], &s[pos..])
    } else {
        (s, "")
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| Error::ValidationError(format!("Invalid quantity format: {}", quantity)))?;

    let multiplier: u64 = match unit {
        "" => 1,
        "Ki" => 1024,
        "Mi" => 1024 * 1024,
        "Gi" => 1024 * 1024 * 1024,
        "Ti" => 1024 * 1024 * 1024 * 1024,
        "k" => 1000,
        "M" => 1000 * 1000,
        "G" => 1000 * 1000 * 1000,
        "T" => 1000 * 1000 * 1000 * 1000,
        _ => return Err(Error::ValidationError(format!("Unknown unit: {}", unit))),
    };

    Ok((num * multiplier as f64) as u64)
}

/// Format bytes to Kubernetes quantity string
pub(crate) fn format_bytes_to_quantity(bytes: u64) -> String {
    const GI: u64 = 1024 * 1024 * 1024;
    const TI: u64 = 1024 * 1024 * 1024 * 1024;

    if bytes >= TI && bytes.is_multiple_of(TI) {
        format!("{}Ti", bytes / TI)
    } else if bytes >= GI && bytes.is_multiple_of(GI) {
        format!("{}Gi", bytes / GI)
    } else {
        // Round up to nearest Gi
        format!("{}Gi", bytes.div_ceil(GI))
    }
}

/// Expand a PVC to a new size
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn expand_pvc(
    client: &Client,
    node: &StellarNode,
    new_size: &str,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let pvc_name = resource_name(node, "data");

    // Get current PVC
    let pvc = api.get(&pvc_name).await?;

    // Update expansion count and timestamp
    let mut annotations = pvc.metadata.annotations.clone().unwrap_or_default();
    let current_count: u32 = annotations
        .get(EXPANSION_COUNT_ANNOTATION)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    annotations.insert(
        EXPANSION_COUNT_ANNOTATION.to_string(),
        (current_count + 1).to_string(),
    );
    annotations.insert(
        LAST_EXPANSION_ANNOTATION.to_string(),
        chrono::Utc::now().to_rfc3339(),
    );

    // Create patch to update PVC size and annotations
    let mut requests = BTreeMap::new();
    requests.insert("storage".to_string(), Quantity(new_size.to_string()));

    let patch = json!({
        "metadata": {
            "annotations": annotations
        },
        "spec": {
            "resources": {
                "requests": requests
            }
        }
    });

    let patch_params = if dry_run {
        PatchParams::apply("stellar-operator").dry_run()
    } else {
        PatchParams::apply("stellar-operator")
    };

    api.patch(&pvc_name, &patch_params, &Patch::Merge(&patch))
        .await?;

    info!(
        "Expanded PVC {} to {} (expansion #{}, dry_run={})",
        pvc_name,
        new_size,
        current_count + 1,
        dry_run
    );

    Ok(())
}

/// Check and potentially expand a PVC based on disk usage
#[instrument(skip(client, node, config), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn check_and_expand(
    client: &Client,
    node: &StellarNode,
    config: &DiskScalerConfig,
    dry_run: bool,
) -> Result<ScalingResult> {
    if !config.enabled {
        return Ok(ScalingResult::NoActionNeeded);
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let pvc_name = resource_name(node, "data");

    // Get PVC
    let pvc = match pvc_api.get(&pvc_name).await {
        Ok(pvc) => pvc,
        Err(e) => {
            warn!("Failed to get PVC {}: {}", pvc_name, e);
            return Ok(ScalingResult::NoActionNeeded);
        }
    };

    // Check if storage class supports expansion
    let storage_class = pvc
        .spec
        .as_ref()
        .and_then(|spec| spec.storage_class_name.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("");

    if !storage_class.is_empty() && !supports_expansion(client, storage_class).await? {
        return Ok(ScalingResult::NotSupported {
            storage_class: storage_class.to_string(),
        });
    }

    // Get disk usage
    let usage = match get_disk_usage(client, node).await? {
        Some(usage) => usage,
        None => {
            debug!("Could not determine disk usage for {}", node.name_any());
            return Ok(ScalingResult::NoActionNeeded);
        }
    };

    // Check if expansion is needed
    if usage.usage_percent < config.expansion_threshold {
        debug!(
            "Disk usage {}% is below threshold {}%",
            usage.usage_percent, config.expansion_threshold
        );
        return Ok(ScalingResult::NoActionNeeded);
    }

    info!(
        "Disk usage {}% exceeds threshold {}% for {}",
        usage.usage_percent,
        config.expansion_threshold,
        node.name_any()
    );

    // Check if expansion is allowed (rate limiting, max expansions)
    if !check_expansion_allowed(&pvc, config)? {
        let annotations = pvc.metadata.annotations.as_ref();
        let count: u32 = annotations
            .and_then(|a| a.get(EXPANSION_COUNT_ANNOTATION))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        if count >= config.max_expansions {
            return Ok(ScalingResult::MaxExpansionsReached { count });
        }

        if let Some(last_expansion_str) = annotations.and_then(|a| a.get(LAST_EXPANSION_ANNOTATION))
        {
            return Ok(ScalingResult::RateLimited {
                last_expansion: last_expansion_str.clone(),
                next_allowed: "See min_expansion_interval_secs".to_string(),
            });
        }
    }

    // Get current size
    let current_size = pvc
        .spec
        .as_ref()
        .and_then(|spec| spec.resources.as_ref())
        .and_then(|res| res.requests.as_ref())
        .and_then(|req| req.get("storage"))
        .map(|q| q.0.clone())
        .unwrap_or_else(|| "100Gi".to_string());

    // Calculate new size
    let new_size = calculate_new_size(&current_size, config.expansion_increment)?;

    // Perform expansion
    match expand_pvc(client, node, &new_size, dry_run).await {
        Ok(()) => {
            let count: u32 = pvc
                .metadata
                .annotations
                .as_ref()
                .and_then(|a| a.get(EXPANSION_COUNT_ANNOTATION))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            Ok(ScalingResult::Expanded {
                old_size: current_size,
                new_size,
                expansion_count: count + 1,
            })
        }
        Err(e) => Ok(ScalingResult::Failed {
            reason: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_percent() {
        assert_eq!(DiskUsage::calculate_percent(50, 100), 50);
        assert_eq!(DiskUsage::calculate_percent(80, 100), 80);
        assert_eq!(DiskUsage::calculate_percent(100, 100), 100);
        assert_eq!(DiskUsage::calculate_percent(0, 100), 0);
        assert_eq!(DiskUsage::calculate_percent(50, 0), 0);
    }

    #[test]
    fn test_parse_quantity_to_bytes() {
        assert_eq!(
            parse_quantity_to_bytes("100Gi").unwrap(),
            100 * 1024 * 1024 * 1024
        );
        assert_eq!(
            parse_quantity_to_bytes("1Ti").unwrap(),
            1024 * 1024 * 1024 * 1024
        );
        assert_eq!(parse_quantity_to_bytes("500Mi").unwrap(), 500 * 1024 * 1024);
    }

    #[test]
    fn test_format_bytes_to_quantity() {
        assert_eq!(format_bytes_to_quantity(100 * 1024 * 1024 * 1024), "100Gi");
        assert_eq!(format_bytes_to_quantity(1024 * 1024 * 1024 * 1024), "1Ti");
    }

    #[test]
    fn test_calculate_new_size() {
        assert_eq!(calculate_new_size("100Gi", 50).unwrap(), "150Gi");
        assert_eq!(calculate_new_size("1Ti", 50).unwrap(), "1536Gi");
    }

    #[test]
    fn test_parse_df_output() {
        let output = "Filesystem     1B-blocks      Used Available Use% Mounted on\n\
                      /dev/xvda1   1610612736000 644245094400 966367641600  40% /data";

        let usage = parse_df_output(output).unwrap().unwrap();
        assert_eq!(usage.capacity_bytes, 1610612736000);
        assert_eq!(usage.used_bytes, 644245094400);
        assert_eq!(usage.usage_percent, 40);
    }
}
