//! Carbon-aware scheduler integration

use crate::carbon_aware::api::CarbonIntensityAPI;
use crate::carbon_aware::types::{CarbonAwareConfig, RegionCarbonData};
use crate::error::Result;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Client, ResourceExt};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Carbon-aware scheduler that enhances node scoring with carbon intensity
pub struct CarbonAwareScheduler {
    /// Carbon intensity API client
    api: CarbonIntensityAPI,
    /// Configuration
    config: CarbonAwareConfig,
    /// Cached carbon intensity data
    carbon_data: Arc<RwLock<RegionCarbonData>>,
    /// Kubernetes client
    _kube_client: Client,
}

impl CarbonAwareScheduler {
    /// Create new carbon-aware scheduler
    pub fn new(api: CarbonIntensityAPI, config: CarbonAwareConfig, kube_client: Client) -> Self {
        Self {
            api,
            config,
            carbon_data: Arc::new(RwLock::new(RegionCarbonData::new())),
            _kube_client: kube_client,
        }
    }

    /// Start background carbon data refresh
    pub async fn start_refresh_loop(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Carbon-aware scheduling disabled");
            return Ok(());
        }

        let api = self.api.clone();
        let _config = self.config.clone();
        let carbon_data = self.carbon_data.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                tokio::time::Duration::from_secs(60), // Refresh every minute
            );

            loop {
                interval.tick().await;

                match api.fetch_all_regions().await {
                    Ok(data) => {
                        let mut guard = carbon_data.write().await;
                        *guard = data;
                        debug!("Refreshed carbon intensity data");
                    }
                    Err(e) => {
                        warn!("Failed to refresh carbon intensity data: {}", e);
                    }
                }
            }
        });

        info!("Started carbon data refresh loop");
        Ok(())
    }

    /// Score nodes based on carbon intensity
    pub async fn score_nodes_carbon_aware<'a>(
        &self,
        _pod: &Pod,
        candidates: &[&'a Node],
    ) -> Result<Vec<(f64, &'a Node)>> {
        if !self.config.enabled {
            // Return equal scores if carbon-aware scheduling is disabled
            return Ok(candidates.iter().map(|n| (1.0, *n)).collect());
        }

        let carbon_data = self.carbon_data.read().await;

        // Check if data is stale
        if carbon_data.is_stale(self.config.max_data_age_minutes) {
            warn!("Carbon intensity data is stale, using fallback scoring");
            return Ok(candidates.iter().map(|n| (1.0, *n)).collect());
        }

        let mut scored_nodes = Vec::new();

        for node in candidates {
            let carbon_score = self.calculate_carbon_score(node, &carbon_data).await;
            scored_nodes.push((carbon_score, *node));
        }

        // Sort by score (higher is better for lower carbon intensity)
        scored_nodes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored_nodes)
    }

    /// Calculate carbon score for a node
    async fn calculate_carbon_score(&self, node: &Node, carbon_data: &RegionCarbonData) -> f64 {
        // Extract region from node labels
        let region = self.extract_node_region(node);

        if let Some(region) = region {
            if let Some(carbon_info) = carbon_data.get_region(&region) {
                // Convert carbon intensity to score (lower intensity = higher score)
                // Normalize to 0-1 range where 1 is best (lowest carbon)
                let max_intensity = 1000.0; // gCO2/kWh - reasonable upper bound
                let normalized_score =
                    1.0 - (carbon_info.carbon_intensity / max_intensity).min(1.0);

                debug!(
                    "Node {} in region {} has carbon intensity {} gCO2/kWh, score: {}",
                    node.name_any(),
                    region,
                    carbon_info.carbon_intensity,
                    normalized_score
                );

                return normalized_score;
            }
        }

        // Default score if region data not available
        debug!(
            "No carbon data available for node {} region, using default score",
            node.name_any()
        );
        0.5
    }

    /// Extract region from node labels
    fn extract_node_region(&self, node: &Node) -> Option<String> {
        if let Some(labels) = &node.metadata.labels {
            // Try different label keys for region
            let region_keys = [
                "topology.kubernetes.io/region",
                "failure-domain.beta.kubernetes.io/region",
                "region.kubernetes.io",
                "cloud.google.com/location",
                "eks.amazonaws.com/region",
            ];

            for key in &region_keys {
                if let Some(region) = labels.get(*key) {
                    return Some(region.clone());
                }
            }
        }

        // Try to extract from node name or provider-specific metadata
        let node_name = node.name_any();

        // AWS: parse from names like "ip-10-0-1-123.us-west-2.compute.internal"
        if node_name.contains("compute.amazonaws.com") || node_name.contains("ec2.internal") {
            if let Some(region) = self.extract_aws_region(&node_name) {
                return Some(region);
            }
        }

        // GCP: parse from names like "gke-cluster-default-pool-123-us-west1-a"
        if node_name.contains("gke-") {
            if let Some(region) = self.extract_gcp_region(&node_name) {
                return Some(region);
            }
        }

        // Azure: parse from names like "aks-agentpool-123-vmss000000"
        if node_name.contains("aks-") {
            // For Azure, we might need to query the node's provider ID
            if let Some(provider_id) = &node.spec.as_ref().and_then(|s| s.provider_id.as_ref()) {
                if let Some(region) = self.extract_azure_region(provider_id) {
                    return Some(region);
                }
            }
        }

        None
    }

    /// Extract AWS region from node name
    fn extract_aws_region(&self, node_name: &str) -> Option<String> {
        // AWS regions in node names like "us-west-2"
        let aws_regions = [
            "us-east-1",
            "us-east-2",
            "us-west-1",
            "us-west-2",
            "ca-central-1",
            "eu-west-1",
            "eu-west-2",
            "eu-central-1",
            "eu-north-1",
            "ap-southeast-1",
            "ap-southeast-2",
            "ap-northeast-1",
            "ap-northeast-2",
            "ap-south-1",
            "sa-east-1",
        ];

        for region in &aws_regions {
            if node_name.contains(region) {
                return Some(region.to_string());
            }
        }
        None
    }

    /// Extract GCP region from node name
    fn extract_gcp_region(&self, node_name: &str) -> Option<String> {
        // GCP regions like "us-west1", "europe-west1"
        let gcp_regions = [
            "us-central1",
            "us-east1",
            "us-west1",
            "us-west2",
            "europe-west1",
            "europe-west2",
            "europe-west3",
            "europe-west4",
            "asia-east1",
            "asia-southeast1",
            "asia-northeast1",
            "asia-northeast2",
        ];

        for region in &gcp_regions {
            if node_name.contains(region) {
                return Some(format!("{}-{}", &region[..2], &region[2..]).to_uppercase());
            }
        }
        None
    }

    /// Extract Azure region from provider ID
    fn extract_azure_region(&self, provider_id: &str) -> Option<String> {
        // Azure provider IDs like "/subscriptions/.../resourceGroups/.../providers/Microsoft.Compute/virtualMachineScaleSets/.../virtualMachines/..."
        // Extract region from resource group or location
        if let Some(rg_start) = provider_id.find("resourceGroups/") {
            let rg_part = &provider_id[rg_start + 15..];
            if let Some(rg_end) = rg_part.find('/') {
                let resource_group = &rg_part[..rg_end];
                // Azure resource groups often contain region info
                if let Some(region) = self.parse_azure_region_from_rg(resource_group) {
                    return Some(region);
                }
            }
        }
        None
    }

    /// Parse Azure region from resource group name
    fn parse_azure_region_from_rg(&self, resource_group: &str) -> Option<String> {
        let azure_regions = [
            "eastus",
            "eastus2",
            "westus",
            "westus2",
            "centralus",
            "northeurope",
            "westeurope",
            "southeastasia",
            "eastasia",
        ];

        for region in &azure_regions {
            if resource_group.contains(region) {
                return Some(region.to_uppercase());
            }
        }
        None
    }

    /// Check if a pod should be scheduled carbon-awar
    pub fn should_schedule_carbon_aware(&self, pod: &Pod) -> bool {
        if !self.config.enabled {
            return false;
        }

        // Check for carbon-aware scheduling annotation
        if let Some(annotations) = &pod.metadata.annotations {
            if let Some(value) = annotations.get("stellar.org/carbon-aware") {
                return value == "true" || value == "enabled";
            }
        }

        // Check for read pool pods (they are non-critical)
        if let Some(labels) = &pod.metadata.labels {
            if labels.get("stellar.org/role").map(|s| s.as_str()) == Some("read-replica") {
                return true;
            }
        }

        false
    }

    /// Get current carbon statistics
    pub async fn get_carbon_stats(&self) -> Result<CarbonStats> {
        let carbon_data = self.carbon_data.read().await;

        let regions_count = carbon_data.regions.len();
        let avg_intensity = if regions_count > 0 {
            carbon_data
                .regions
                .values()
                .map(|d| d.carbon_intensity)
                .sum::<f64>()
                / regions_count as f64
        } else {
            0.0
        };

        let best_region = carbon_data
            .regions
            .values()
            .min_by(|a, b| a.carbon_intensity.partial_cmp(&b.carbon_intensity).unwrap())
            .map(|d| d.region.clone());

        Ok(CarbonStats {
            regions_count,
            average_intensity: avg_intensity,
            best_region,
            last_updated: carbon_data.last_updated,
            is_stale: carbon_data.is_stale(self.config.max_data_age_minutes),
        })
    }
}

/// Carbon statistics for dashboard
#[derive(Clone, Debug)]
pub struct CarbonStats {
    pub regions_count: usize,
    pub average_intensity: f64,
    pub best_region: Option<String>,
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub is_stale: bool,
}
