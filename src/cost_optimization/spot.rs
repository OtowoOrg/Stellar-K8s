// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Spot Instance Integration for Cost Optimization
//!
//! Provides spot instance scheduling, interruption handling, and cost analysis
//! for non-critical workloads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Spot instance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotConfig {
    pub enabled: bool,
    pub max_price_per_hour_usd: f64,
    pub instance_types: Vec<String>,
    pub availability_zones: Vec<String>,
    pub interruption_handling: InterruptionHandling,
    pub fallback_to_ondemand: bool,
}

impl Default for SpotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_price_per_hour_usd: 0.50,
            instance_types: vec![
                "t3.medium".to_string(),
                "t3.large".to_string(),
                "m5.large".to_string(),
                "c5.large".to_string(),
            ],
            availability_zones: vec![
                "us-east-1a".to_string(),
                "us-east-1b".to_string(),
                "us-east-1c".to_string(),
            ],
            interruption_handling: InterruptionHandling::default(),
            fallback_to_ondemand: true,
        }
    }
}

/// Spot interruption handling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionHandling {
    pub notice_period_seconds: u32,
    pub graceful_shutdown_timeout_seconds: u32,
    pub preemption_strategy: PreemptionStrategy,
    pub notification_webhook: Option<String>,
}

impl Default for InterruptionHandling {
    fn default() -> Self {
        Self {
            notice_period_seconds: 120,
            graceful_shutdown_timeout_seconds: 60,
            preemption_strategy: PreemptionStrategy::GracefulDrain,
            notification_webhook: None,
        }
    }
}

/// Preemption strategy for spot instances
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreemptionStrategy {
    /// Gracefully drain and migrate workloads
    GracefulDrain,
    /// Save state and terminate immediately
    SnapshotAndTerminate,
    /// Wait for deadline then force terminate
    WaitForDeadline,
}

/// Spot instance request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotRequest {
    pub id: String,
    pub instance_type: String,
    pub availability_zone: String,
    pub max_price_usd: f64,
    pub status: SpotRequestStatus,
    pub created_at: DateTime<Utc>,
    pub fulfilled_at: Option<DateTime<Utc>>,
    pub interruption_time: Option<DateTime<Utc>>,
    pub current_price_usd: f64,
}

/// Spot request status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpotRequestStatus {
    Pending,
    Active,
    Interrupted,
    Fulfilled,
    Failed,
}

/// Spot instance cost analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotCostAnalysis {
    pub total_spot_cost_usd: f64,
    pub total_ondemand_cost_usd: f64,
    pub savings_usd: f64,
    pub savings_percent: f64,
    pub interruption_count: usize,
    pub avg_instance_lifetime_hours: f64,
    pub utilization_percent: f64,
}

/// Spot instance manager
pub struct SpotManager {
    config: SpotConfig,
    active_requests: HashMap<String, SpotRequest>,
    cost_history: Vec<SpotCostRecord>,
}

/// Spot cost record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotCostRecord {
    pub timestamp: DateTime<Utc>,
    pub instance_type: String,
    pub spot_price_usd: f64,
    pub ondemand_price_usd: f64,
    pub instance_id: Option<String>,
}

impl SpotManager {
    pub fn new(config: SpotConfig) -> Self {
        Self {
            config,
            active_requests: HashMap::new(),
            cost_history: Vec::new(),
        }
    }

    /// Request a spot instance
    pub async fn request_spot_instance(
        &mut self,
        workload_id: &str,
        instance_type: &str,
    ) -> Result<SpotRequest, String> {
        if !self.config.enabled {
            return Err("Spot instances disabled".to_string());
        }

        if !self
            .config
            .instance_types
            .contains(&instance_type.to_string())
        {
            return Err(format!(
                "Instance type {} not in allowed list",
                instance_type
            ));
        }

        let request = SpotRequest {
            id: format!("sir-{:016x}", rand::random::<u64>()),
            instance_type: instance_type.to_string(),
            availability_zone: self.config.availability_zones[0].clone(),
            max_price_usd: self.config.max_price_per_hour_usd,
            status: SpotRequestStatus::Pending,
            created_at: Utc::now(),
            fulfilled_at: None,
            interruption_time: None,
            current_price_usd: 0.0,
        };

        self.active_requests
            .insert(request.id.clone(), request.clone());

        tracing::info!(
            "Spot instance requested: {} for workload {}",
            request.id,
            workload_id
        );

        Ok(request)
    }

    /// Handle spot interruption notice
    pub async fn handle_interruption(&mut self, request_id: &str) -> Result<(), String> {
        let request = self
            .active_requests
            .get_mut(request_id)
            .ok_or_else(|| format!("Request {} not found", request_id))?;

        request.status = SpotRequestStatus::Interrupted;
        request.interruption_time = Some(Utc::now());

        match &self.config.interruption_handling.preemption_strategy {
            PreemptionStrategy::GracefulDrain => {
                tracing::info!("Graceful drain initiated for spot instance {}", request_id);
                // Would trigger workload migration
            }
            PreemptionStrategy::SnapshotAndTerminate => {
                tracing::info!(
                    "Snapshot and terminate initiated for spot instance {}",
                    request_id
                );
                // Would save state and terminate
            }
            PreemptionStrategy::WaitForDeadline => {
                tracing::info!("Waiting for deadline on spot instance {}", request_id);
                // Would wait then force terminate
            }
        }

        Ok(())
    }

    /// Calculate cost analysis
    pub fn calculate_cost_analysis(&self) -> SpotCostAnalysis {
        let total_spot_cost: f64 = self.cost_history.iter().map(|r| r.spot_price_usd).sum();
        let total_ondemand_cost: f64 = self.cost_history.iter().map(|r| r.ondemand_price_usd).sum();
        let savings = total_ondemand_cost - total_spot_cost;
        let interruption_count = self
            .active_requests
            .values()
            .filter(|r| matches!(r.status, SpotRequestStatus::Interrupted))
            .count();

        SpotCostAnalysis {
            total_spot_cost_usd: total_spot_cost,
            total_ondemand_cost_usd: total_ondemand_cost,
            savings_usd: savings,
            savings_percent: if total_ondemand_cost > 0.0 {
                (savings / total_ondemand_cost) * 100.0
            } else {
                0.0
            },
            interruption_count,
            avg_instance_lifetime_hours: 24.0, // Would calculate from history
            utilization_percent: 85.0,         // Would calculate from metrics
        }
    }

    /// Get recommendations for spot usage
    pub fn get_recommendations(&self) -> Vec<SpotRecommendation> {
        let mut recommendations = Vec::new();

        if self.config.enabled {
            let analysis = self.calculate_cost_analysis();

            if analysis.savings_percent < 50.0 {
                recommendations.push(SpotRecommendation {
                    recommendation_type: SpotRecommendationType::IncreaseSpotUsage,
                    description:
                        "Consider increasing spot instance usage for non-critical workloads"
                            .to_string(),
                    estimated_savings_usd: analysis.total_ondemand_cost_usd * 0.3,
                    priority: RecommendationPriority::High,
                });
            }

            if analysis.interruption_count > 10 {
                recommendations.push(SpotRecommendation {
                    recommendation_type: SpotRecommendationType::DiversifyInstanceTypes,
                    description: "Diversify instance types to reduce interruption frequency"
                        .to_string(),
                    estimated_savings_usd: 0.0,
                    priority: RecommendationPriority::Medium,
                });
            }
        }

        recommendations
    }
}

/// Spot instance recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotRecommendation {
    pub recommendation_type: SpotRecommendationType,
    pub description: String,
    pub estimated_savings_usd: f64,
    pub priority: RecommendationPriority,
}

/// Spot recommendation types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpotRecommendationType {
    IncreaseSpotUsage,
    DiversifyInstanceTypes,
    AdjustMaxPrice,
    EnableSpotForWorkload,
}

/// Recommendation priority
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecommendationPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spot_config_default() {
        let config = SpotConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_price_per_hour_usd, 0.50);
        assert!(config.instance_types.contains(&"t3.medium".to_string()));
    }

    #[test]
    fn test_spot_manager_creation() {
        let manager = SpotManager::new(SpotConfig::default());
        assert!(manager.active_requests.is_empty());
    }

    #[tokio::test]
    async fn test_request_spot_instance() {
        let mut manager = SpotManager::new(SpotConfig::default());
        let result = manager
            .request_spot_instance("workload-1", "t3.medium")
            .await;
        assert!(result.is_ok());
        assert_eq!(manager.active_requests.len(), 1);
    }

    #[test]
    fn test_cost_analysis() {
        let manager = SpotManager::new(SpotConfig::default());
        let analysis = manager.calculate_cost_analysis();
        assert_eq!(analysis.total_spot_cost_usd, 0.0);
        assert_eq!(analysis.interruption_count, 0);
    }
}
