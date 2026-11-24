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
//! Cost optimization recommendation engine

use serde::{Deserialize, Serialize};

use super::calculator::ResourceCost;
use super::model::CostRecord;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecommendationType {
    RightSize,
    UseReservedInstance,
    UseSpotInstance,
    DeleteUnused,
    MoveToLowerCostRegion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OptimizationRecommendation {
    pub resource_id: String,
    pub recommendation_type: RecommendationType,
    pub description: String,
    pub current_monthly_cost: f64,
    pub estimated_monthly_savings: f64,
    pub savings_pct: f64,
    pub effort: ImplementationEffort,
    pub risk: RiskLevel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ImplementationEffort {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

pub struct RecommendationEngine {
    min_savings_usd: f64,
}

impl RecommendationEngine {
    pub fn new(min_savings_usd: f64) -> Self {
        Self { min_savings_usd }
    }

    pub fn generate(
        &self,
        record: &CostRecord,
        cost: &ResourceCost,
    ) -> Vec<OptimizationRecommendation> {
        let mut recs = Vec::new();

        // Reserved instance recommendation
        if !record.is_reserved && cost.potential_savings_reserved >= self.min_savings_usd {
            let savings_pct = cost.potential_savings_reserved / cost.monthly_estimate_usd * 100.0;
            recs.push(OptimizationRecommendation {
                resource_id: record.resource_id.clone(),
                recommendation_type: RecommendationType::UseReservedInstance,
                description: format!(
                    "Switch {} to a 1-year reserved instance to save {:.0}%",
                    record.instance_type, savings_pct
                ),
                current_monthly_cost: cost.monthly_estimate_usd,
                estimated_monthly_savings: cost.potential_savings_reserved,
                savings_pct,
                effort: ImplementationEffort::Low,
                risk: RiskLevel::Low,
            });
        }

        // Spot instance recommendation (for non-critical workloads)
        if !record.is_spot && cost.potential_savings_spot >= self.min_savings_usd {
            let savings_pct = cost.potential_savings_spot / cost.monthly_estimate_usd * 100.0;
            recs.push(OptimizationRecommendation {
                resource_id: record.resource_id.clone(),
                recommendation_type: RecommendationType::UseSpotInstance,
                description: format!(
                    "Use spot/preemptible instances for {} to save {:.0}% (fault-tolerant workloads only)",
                    record.instance_type, savings_pct
                ),
                current_monthly_cost: cost.monthly_estimate_usd,
                estimated_monthly_savings: cost.potential_savings_spot,
                savings_pct,
                effort: ImplementationEffort::Medium,
                risk: RiskLevel::Medium,
            });
        }

        recs
    }
}
