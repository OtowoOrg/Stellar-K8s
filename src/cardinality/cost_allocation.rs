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
//! Cost allocation for metric cardinality

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;

use crate::cardinality::budget::BudgetSpec;
use crate::cardinality::collector::CardinalityCollector;
use crate::error::{Error, Result};

/// Cost allocation configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostAllocationConfig {
    /// Cost per metric series per month (USD)
    #[serde(default = "default_cost_per_series")]
    pub cost_per_series_per_month: f64,
    /// Cost per sample ingested (USD)
    #[serde(default = "default_cost_per_sample")]
    pub cost_per_sample: f64,
    /// Currency
    #[serde(default = "default_currency")]
    pub currency: String,
    /// Allocation method
    #[serde(default)]
    pub allocation_method: AllocationMethod,
}

fn default_cost_per_series() -> f64 { 0.10 }
fn default_cost_per_sample() -> f64 { 0.000001 }
fn default_currency() -> String { "USD".to_string() }

/// Cost allocation method
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum AllocationMethod {
    /// Proportional to cardinality usage
    #[default]
    Proportional,
    /// Equal share per team
    EqualShare,
    /// Based on budget limits
    BudgetBased,
}

/// Team cost breakdown
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCostBreakdown {
    pub team: String,
    pub series_cost: f64,
    pub ingestion_cost: f64,
    pub total_cost: f64,
    pub series_count: u64,
    pub sample_count: u64,
    pub budget_limit: u64,
    pub budget_utilization: f64,
    #[serde(default)]
    pub metric_breakdown: BTreeMap<String, MetricCostBreakdown>,
}

/// Per-metric cost breakdown
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricCostBreakdown {
    pub metric_name: String,
    pub series_count: u64,
    pub sample_count: u64,
    pub series_cost: f64,
    pub ingestion_cost: f64,
    pub total_cost: f64,
}

/// Cost allocation report
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostAllocationReport {
    pub generated_at: DateTime<Utc>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub currency: String,
    pub total_cost: f64,
    pub total_series: u64,
    pub total_samples: u64,
    pub team_breakdowns: Vec<TeamCostBreakdown>,
    pub unallocated_cost: f64,
}

/// Cardinality cost allocator
pub struct CardinalityCostAllocator {
    config: CostAllocationConfig,
    collector: Arc<CardinalityCollector>,
    /// Historical reports
    reports: Arc<RwLock<Vec<CostAllocationReport>>>,
    max_reports: usize,
}

impl CardinalityCostAllocator {
    /// Create a new cost allocator
    pub fn new(config: CostAllocationConfig, collector: Arc<CardinalityCollector>) -> Self {
        Self {
            config,
            collector,
            reports: Arc::new(RwLock::new(Vec::new())),
            max_reports: 100,
        }
    }

    /// Generate cost allocation report
    pub async fn generate_report(&self, period_start: DateTime<Utc>, period_end: DateTime<Utc>) -> Result<CostAllocationReport> {
        let counts = self.collector.get_all_counts().await;
        
        let mut team_breakdowns = Vec::new();
        let mut total_cost = 0.0;
        let mut total_series = 0u64;
        let mut total_samples = 0u64;

        // Get budget specs for limit lookup
        let budgets = self.get_budget_map().await;

        for (team, team_counts) in counts {
            let mut metric_breakdowns = BTreeMap::new();
            let mut team_series = 0u64;
            let mut team_samples = 0u64;
            let mut team_series_cost = 0.0;
            let mut team_ingestion_cost = 0.0;

            for (metric, &series_count) in team_counts {
                // In a real implementation, we'd track samples per metric
                // For now, estimate based on series count
                let estimated_samples = series_count * 1000; // Rough estimate
                
                let series_cost = series_count as f64 * self.config.cost_per_series_per_month;
                let ingestion_cost = estimated_samples as f64 * self.config.cost_per_sample;
                let total_metric_cost = series_cost + ingestion_cost;

                metric_breakdowns.insert(metric.clone(), MetricCostBreakdown {
                    metric_name: metric.clone(),
                    series_count,
                    sample_count: estimated_samples,
                    series_cost,
                    ingestion_cost,
                    total_cost: total_metric_cost,
                });

                team_series += series_count;
                team_samples += estimated_samples;
                team_series_cost += series_cost;
                team_ingestion_cost += ingestion_cost;
            }

            let team_total = team_series_cost + team_ingestion_cost;
            let budget_limit = budgets.get(&team).map(|b| b.limit).unwrap_or(0);
            let budget_utilization = if budget_limit > 0 {
                team_series as f64 / budget_limit as f64 * 100.0
            } else {
                0.0
            };

            team_breakdowns.push(TeamCostBreakdown {
                team: team.clone(),
                series_cost: team_series_cost,
                ingestion_cost: team_ingestion_cost,
                total_cost: team_total,
                series_count: team_series,
                sample_count: team_samples,
                budget_limit,
                budget_utilization,
                metric_breakdown: metric_breakdowns,
            });

            total_cost += team_total;
            total_series += team_series;
            total_samples += team_samples;
        }

        // Sort by cost descending
        team_breakdowns.sort_by(|a, b| b.total_cost.partial_cmp(&a.total_cost).unwrap());

        let report = CostAllocationReport {
            generated_at: Utc::now(),
            period_start,
            period_end,
            currency: self.config.currency.clone(),
            total_cost,
            total_series,
            total_samples,
            team_breakdowns,
            unallocated_cost: 0.0,
        };

        // Store report
        let mut reports = self.reports.write().await;
        reports.push(report.clone());
        if reports.len() > self.max_reports {
            reports.remove(0);
        }

        info!(
            "Generated cost allocation report: ${:.2} total, {} teams, {} series",
            total_cost, reports.len(), total_series
        );

        Ok(report)
    }

    /// Get budget map for limit lookups
    async fn get_budget_map(&self) -> BTreeMap<String, BudgetSpec> {
        // In a real implementation, this would fetch from the collector's config
        // For now, return empty map
        BTreeMap::new()
    }

    /// Get historical reports
    pub async fn get_reports(&self) -> Vec<CostAllocationReport> {
        self.reports.read().await.clone()
    }

    /// Get latest report
    pub async fn get_latest_report(&self) -> Option<CostAllocationReport> {
        self.reports.read().await.last().cloned()
    }

    /// Export report as CSV
    pub async fn export_csv(&self, report: &CostAllocationReport) -> Result<String> {
        let mut csv = String::from("team,series_count,sample_count,series_cost,ingestion_cost,total_cost,budget_limit,budget_utilization_percent\n");
        
        for team in &report.team_breakdowns {
            csv.push_str(&format!(
                "{},{},{},{:.4},{:.4},{:.4},{},{:.2}\n",
                team.team,
                team.series_count,
                team.sample_count,
                team.series_cost,
                team.ingestion_cost,
                team.total_cost,
                team.budget_limit,
                team.budget_utilization
            ));
        }
        
        Ok(csv)
    }

    /// Export report as JSON
    pub async fn export_json(&self, report: &CostAllocationReport) -> Result<String> {
        serde_json::to_string_pretty(report).map_err(Error::SerializationError)
    }

    /// Start periodic report generation
    pub async fn start_report_task(self: Arc<Self>, interval: chrono::Duration) {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval.num_seconds() as u64));
        
        loop {
            ticker.tick().await;
            let now = Utc::now();
            let period_start = now - interval;
            if let Err(e) = self.generate_report(period_start, now).await {
                tracing::error!("Failed to generate cost allocation report: {}", e);
            }
        }
    }
}

impl Default for CostAllocationConfig {
    fn default() -> Self {
        Self {
            cost_per_series_per_month: default_cost_per_series(),
            cost_per_sample: default_cost_per_sample(),
            currency: default_currency(),
            allocation_method: AllocationMethod::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn test_cost_allocation_config_defaults() {
        let config = CostAllocationConfig::default();
        assert_eq!(config.cost_per_series_per_month, 0.10);
        assert_eq!(config.cost_per_sample, 0.000001);
        assert_eq!(config.currency, "USD");
        assert_eq!(config.allocation_method, AllocationMethod::Proportional);
    }

    #[test]
    fn test_team_cost_breakdown_serialization() {
        let breakdown = TeamCostBreakdown {
            team: "test-team".to_string(),
            series_cost: 10.0,
            ingestion_cost: 5.0,
            total_cost: 15.0,
            series_count: 100,
            sample_count: 100000,
            budget_limit: 1000,
            budget_utilization: 10.0,
            metric_breakdown: BTreeMap::new(),
        };
        
        let json = serde_json::to_string(&breakdown).unwrap();
        assert!(json.contains("test-team"));
        assert!(json.contains("15.0"));
    }
}
