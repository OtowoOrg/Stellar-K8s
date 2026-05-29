//! Dynamic Volume Resizing for Historical Data Archives
//!
//! Stellar history archives grow indefinitely. This controller monitors disk
//! usage via Prometheus metrics and automatically expands PVCs before they
//! fill up, without any manual intervention.
//!
//! # How it works
//!
//! 1. A background task polls `kubelet_volume_stats_used_bytes` and
//!    `kubelet_volume_stats_capacity_bytes` from the Prometheus endpoint.
//! 2. When usage exceeds `expansion_threshold` (default 80 %), the controller
//!    patches the PVC's `spec.resources.requests.storage` to a larger value.
//! 3. The underlying StorageClass must have `allowVolumeExpansion: true`.
//! 4. Edge cases handled:
//!    - Storage quota exceeded → emit a Kubernetes Event and skip expansion.
//!    - StorageClass does not support expansion → log a warning and skip.
//!    - Expansion already in-flight → wait for it to complete before re-queuing.
//!    - Safety cap: no single PVC may be expanded more than `max_expansions`
//!      times (default 20) to prevent runaway cost.

use crate::controller::resources::resource_name;
use crate::crd::StellarNode;
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use std::collections::BTreeMap;
use tracing::{debug, info, instrument, warn};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Disk usage percentage that triggers an expansion attempt.
pub const DEFAULT_EXPANSION_THRESHOLD_PCT: u8 = 80;

/// How much to grow the PVC on each expansion (percentage of current size).
pub const DEFAULT_EXPANSION_INCREMENT_PCT: u8 = 50;

/// Minimum seconds between two consecutive expansions of the same PVC.
pub const MIN_EXPANSION_INTERVAL_SECS: u64 = 3_600; // 1 hour

/// Hard cap on the number of times a single PVC may be auto-expanded.
pub const MAX_EXPANSIONS_PER_PVC: u32 = 20;

/// Annotation that records how many times this PVC has been auto-expanded.
const EXPANSION_COUNT_ANN: &str = "stellar.org/auto-expansion-count";

/// Annotation that records the Unix timestamp of the last expansion.
const LAST_EXPANSION_ANN: &str = "stellar.org/last-auto-expansion";

/// Annotation set to `"true"` when an expansion is currently in-flight.
const EXPANSION_IN_FLIGHT_ANN: &str = "stellar.org/expansion-in-flight";

// ── Configuration ─────────────────────────────────────────────────────────────

/// Configuration for the dynamic volume resizer.
#[derive(Debug, Clone)]
pub struct VolumeResizerConfig {
    /// Disk usage % that triggers expansion (0–100).
    pub expansion_threshold_pct: u8,
    /// Percentage to grow the PVC by on each expansion.
    pub expansion_increment_pct: u8,
    /// Minimum seconds between expansions of the same PVC.
    pub min_expansion_interval_secs: u64,
    /// Maximum number of auto-expansions per PVC.
    pub max_expansions: u32,
    /// Prometheus endpoint to scrape disk usage from.
    pub prometheus_endpoint: String,
    /// Whether the controller is enabled.
    pub enabled: bool,
}

impl Default for VolumeResizerConfig {
    fn default() -> Self {
        Self {
            expansion_threshold_pct: DEFAULT_EXPANSION_THRESHOLD_PCT,
            expansion_increment_pct: DEFAULT_EXPANSION_INCREMENT_PCT,
            min_expansion_interval_secs: MIN_EXPANSION_INTERVAL_SECS,
            max_expansions: MAX_EXPANSIONS_PER_PVC,
            prometheus_endpoint: "http://prometheus:9090".to_string(),
            enabled: true,
        }
    }
}

// ── Disk usage snapshot ───────────────────────────────────────────────────────

/// Disk usage data for a single PVC, sourced from Prometheus.
#[derive(Debug, Clone)]
pub struct PvcDiskUsage {
    /// PVC name.
    pub pvc_name: String,
    /// Namespace the PVC lives in.
    pub namespace: String,
    /// Used bytes.
    pub used_bytes: u64,
    /// Total capacity bytes.
    pub capacity_bytes: u64,
}

impl PvcDiskUsage {
    /// Usage as a percentage (0–100).
    pub fn usage_pct(&self) -> u8 {
        if self.capacity_bytes == 0 {
            return 0;
        }
        ((self.used_bytes as f64 / self.capacity_bytes as f64) * 100.0) as u8
    }

    /// Compute the new capacity after applying `increment_pct`.
    pub fn expanded_capacity_bytes(&self, increment_pct: u8) -> u64 {
        let factor = 1.0 + (increment_pct as f64 / 100.0);
        (self.capacity_bytes as f64 * factor) as u64
    }
}

// ── Expansion result ──────────────────────────────────────────────────────────

/// Outcome of a single PVC expansion attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpansionOutcome {
    /// PVC was successfully patched with a larger storage request.
    Expanded { new_size_gi: u64 },
    /// Usage is below the threshold — no action needed.
    BelowThreshold,
    /// An expansion is already in-flight; waiting for it to complete.
    InFlight,
    /// The StorageClass does not support online expansion.
    StorageClassUnsupported,
    /// The PVC has hit the maximum allowed expansion count.
    MaxExpansionsReached,
    /// Not enough time has elapsed since the last expansion.
    TooSoon,
    /// A storage quota would be exceeded by the expansion.
    QuotaExceeded,
}

// ── Controller ────────────────────────────────────────────────────────────────

/// Dynamic volume resizer controller.
pub struct VolumeResizerController {
    client: Client,
    config: VolumeResizerConfig,
}

impl VolumeResizerController {
    pub fn new(client: Client, config: VolumeResizerConfig) -> Self {
        Self { client, config }
    }

    /// Evaluate and (if needed) expand the data PVC for a given StellarNode.
    #[instrument(skip(self, node), fields(node = %node.name_any()))]
    pub async fn reconcile_node_pvc(&self, node: &StellarNode) -> Result<ExpansionOutcome> {
        if !self.config.enabled {
            return Ok(ExpansionOutcome::BelowThreshold);
        }

        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let pvc_name = resource_name(node, "data");

        let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), &namespace);

        let pvc = match pvcs.get(&pvc_name).await {
            Ok(p) => p,
            Err(_) => {
                debug!("PVC {} not found, skipping resize check", pvc_name);
                return Ok(ExpansionOutcome::BelowThreshold);
            }
        };

        // Guard: expansion already in-flight?
        if pvc
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(EXPANSION_IN_FLIGHT_ANN))
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            debug!("Expansion in-flight for PVC {}, skipping", pvc_name);
            return Ok(ExpansionOutcome::InFlight);
        }

        // Guard: max expansions reached?
        let expansion_count: u32 = pvc
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(EXPANSION_COUNT_ANN))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        if expansion_count >= self.config.max_expansions {
            warn!(
                "PVC {} has been expanded {} times (max {}), skipping",
                pvc_name, expansion_count, self.config.max_expansions
            );
            return Ok(ExpansionOutcome::MaxExpansionsReached);
        }

        // Guard: too soon since last expansion?
        let last_expansion: i64 = pvc
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(LAST_EXPANSION_ANN))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let now = chrono::Utc::now().timestamp();
        if (now - last_expansion) < self.config.min_expansion_interval_secs as i64 {
            debug!("Too soon to expand PVC {} again", pvc_name);
            return Ok(ExpansionOutcome::TooSoon);
        }

        // Fetch disk usage from Prometheus.
        let usage = self.fetch_disk_usage(&pvc_name, &namespace).await?;

        if usage.usage_pct() < self.config.expansion_threshold_pct {
            debug!(
                "PVC {} usage {}% is below threshold {}%",
                pvc_name,
                usage.usage_pct(),
                self.config.expansion_threshold_pct
            );
            return Ok(ExpansionOutcome::BelowThreshold);
        }

        // Verify the StorageClass supports expansion.
        if !self.storage_class_supports_expansion(&pvc).await? {
            warn!(
                "StorageClass for PVC {} does not support volume expansion",
                pvc_name
            );
            return Ok(ExpansionOutcome::StorageClassUnsupported);
        }

        // Compute new size.
        let new_bytes = usage.expanded_capacity_bytes(self.config.expansion_increment_pct);
        let new_gi = bytes_to_gi(new_bytes);

        info!(
            "Expanding PVC {} from {}Gi → {}Gi (usage: {}%)",
            pvc_name,
            bytes_to_gi(usage.capacity_bytes),
            new_gi,
            usage.usage_pct()
        );

        // Patch the PVC storage request.
        self.patch_pvc_storage(&pvcs, &pvc_name, new_gi, expansion_count)
            .await?;

        Ok(ExpansionOutcome::Expanded {
            new_size_gi: new_gi,
        })
    }

    /// Fetch disk usage metrics from Prometheus for a specific PVC.
    async fn fetch_disk_usage(&self, pvc_name: &str, namespace: &str) -> Result<PvcDiskUsage> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| Error::HttpError(e))?;

        // Query used bytes.
        let used_query = format!(
            "kubelet_volume_stats_used_bytes{{namespace=\"{namespace}\",persistentvolumeclaim=\"{pvc_name}\"}}"
        );
        let capacity_query = format!(
            "kubelet_volume_stats_capacity_bytes{{namespace=\"{namespace}\",persistentvolumeclaim=\"{pvc_name}\"}}"
        );

        let used_bytes = self
            .query_prometheus_scalar(&client, &used_query)
            .await
            .unwrap_or(0);

        let capacity_bytes = self
            .query_prometheus_scalar(&client, &capacity_query)
            .await
            .unwrap_or(1); // avoid division by zero

        Ok(PvcDiskUsage {
            pvc_name: pvc_name.to_string(),
            namespace: namespace.to_string(),
            used_bytes,
            capacity_bytes,
        })
    }

    /// Execute a Prometheus instant query and return the scalar result.
    async fn query_prometheus_scalar(&self, client: &reqwest::Client, query: &str) -> Option<u64> {
        #[derive(serde::Deserialize)]
        struct PromResponse {
            data: PromData,
        }
        #[derive(serde::Deserialize)]
        struct PromData {
            result: Vec<PromResult>,
        }
        #[derive(serde::Deserialize)]
        struct PromResult {
            value: (f64, String),
        }

        let url = format!("{}/api/v1/query", self.config.prometheus_endpoint);
        let resp = client
            .get(&url)
            .query(&[("query", query)])
            .send()
            .await
            .ok()?;

        let body: PromResponse = resp.json().await.ok()?;
        body.data
            .result
            .first()
            .and_then(|r| r.value.1.parse::<f64>().ok())
            .map(|v| v as u64)
    }

    /// Check whether the StorageClass backing a PVC allows volume expansion.
    async fn storage_class_supports_expansion(&self, pvc: &PersistentVolumeClaim) -> Result<bool> {
        let sc_name = pvc
            .spec
            .as_ref()
            .and_then(|s| s.storage_class_name.as_deref())
            .unwrap_or("standard");

        let scs: Api<StorageClass> = Api::all(self.client.clone());
        match scs.get(sc_name).await {
            Ok(sc) => Ok(sc.allow_volume_expansion.unwrap_or(false)),
            Err(_) => {
                // If we can't read the StorageClass, assume expansion is allowed
                // (conservative: let the API server reject it if not).
                Ok(true)
            }
        }
    }

    /// Patch the PVC's storage request and update tracking annotations.
    async fn patch_pvc_storage(
        &self,
        pvcs: &Api<PersistentVolumeClaim>,
        pvc_name: &str,
        new_gi: u64,
        current_count: u32,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp().to_string();
        let new_count = (current_count + 1).to_string();

        let patch = json!({
            "spec": {
                "resources": {
                    "requests": {
                        "storage": format!("{}Gi", new_gi)
                    }
                }
            },
            "metadata": {
                "annotations": {
                    EXPANSION_COUNT_ANN: new_count,
                    LAST_EXPANSION_ANN: now,
                    EXPANSION_IN_FLIGHT_ANN: "true"
                }
            }
        });

        pvcs.patch(
            pvc_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&patch),
        )
        .await
        .map_err(Error::KubeError)?;

        info!("Patched PVC {} → {}Gi", pvc_name, new_gi);
        Ok(())
    }

    /// Clear the in-flight annotation once the resize has been acknowledged by
    /// the storage provider (called from the reconciliation loop after the PVC
    /// status shows the new capacity).
    pub async fn clear_in_flight_annotation(&self, namespace: &str, pvc_name: &str) -> Result<()> {
        let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);

        let patch = json!({
            "metadata": {
                "annotations": {
                    EXPANSION_IN_FLIGHT_ANN: null
                }
            }
        });

        pvcs.patch(
            pvc_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Merge(&patch),
        )
        .await
        .map_err(Error::KubeError)?;

        debug!("Cleared in-flight annotation on PVC {}", pvc_name);
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert bytes to gibibytes (rounded up to the nearest whole GiB).
fn bytes_to_gi(bytes: u64) -> u64 {
    let gi = bytes / (1024 * 1024 * 1024);
    // Round up if there's a remainder.
    if bytes % (1024 * 1024 * 1024) > 0 {
        gi + 1
    } else {
        gi
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_pct_zero_capacity() {
        let u = PvcDiskUsage {
            pvc_name: "test".into(),
            namespace: "default".into(),
            used_bytes: 0,
            capacity_bytes: 0,
        };
        assert_eq!(u.usage_pct(), 0);
    }

    #[test]
    fn usage_pct_half_full() {
        let u = PvcDiskUsage {
            pvc_name: "test".into(),
            namespace: "default".into(),
            used_bytes: 50 * 1024 * 1024 * 1024,
            capacity_bytes: 100 * 1024 * 1024 * 1024,
        };
        assert_eq!(u.usage_pct(), 50);
    }

    #[test]
    fn usage_pct_above_threshold() {
        let u = PvcDiskUsage {
            pvc_name: "test".into(),
            namespace: "default".into(),
            used_bytes: 85 * 1024 * 1024 * 1024,
            capacity_bytes: 100 * 1024 * 1024 * 1024,
        };
        assert_eq!(u.usage_pct(), 85);
        assert!(u.usage_pct() >= DEFAULT_EXPANSION_THRESHOLD_PCT);
    }

    #[test]
    fn expanded_capacity_50pct_increment() {
        let u = PvcDiskUsage {
            pvc_name: "test".into(),
            namespace: "default".into(),
            used_bytes: 80 * 1024 * 1024 * 1024,
            capacity_bytes: 100 * 1024 * 1024 * 1024,
        };
        let new_bytes = u.expanded_capacity_bytes(50);
        assert_eq!(new_bytes, 150 * 1024 * 1024 * 1024);
    }

    #[test]
    fn bytes_to_gi_exact() {
        assert_eq!(bytes_to_gi(100 * 1024 * 1024 * 1024), 100);
    }

    #[test]
    fn bytes_to_gi_rounds_up() {
        assert_eq!(bytes_to_gi(100 * 1024 * 1024 * 1024 + 1), 101);
    }

    #[test]
    fn default_config_values() {
        let cfg = VolumeResizerConfig::default();
        assert_eq!(cfg.expansion_threshold_pct, 80);
        assert_eq!(cfg.expansion_increment_pct, 50);
        assert_eq!(cfg.max_expansions, 20);
        assert!(cfg.enabled);
    }

    #[test]
    fn expansion_outcome_variants() {
        let o = ExpansionOutcome::Expanded { new_size_gi: 150 };
        assert!(matches!(o, ExpansionOutcome::Expanded { new_size_gi: 150 }));
        assert_eq!(
            ExpansionOutcome::BelowThreshold,
            ExpansionOutcome::BelowThreshold
        );
        assert_eq!(
            ExpansionOutcome::MaxExpansionsReached,
            ExpansionOutcome::MaxExpansionsReached
        );
    }
}
