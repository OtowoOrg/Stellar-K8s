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
//! Cardinality collector for tracking metric series at ingestion time

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::cardinality::budget::{BudgetSpec, EnforcementAction, MetricSelector};
use crate::error::{Error, Result};

/// Configuration for the cardinality collector
#[derive(Clone, Debug)]
pub struct CollectorConfig {
    /// Cardinality budgets to enforce
    pub budgets: Vec<BudgetSpec>,
    /// Sample rate for cardinality tracking (0.0-1.0)
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    /// Maximum series to track in memory per metric
    #[serde(default = "default_max_series")]
    pub max_series_per_metric: usize,
    /// Flush interval for metrics
    #[serde(with = "crate::cardinality::budget::duration_serde")]
    #[serde(default = "default_flush_interval")]
    pub flush_interval: chrono::Duration,
    /// Enable quarantine enforcement
    #[serde(default)]
    pub enable_quarantine: bool,
}

fn default_sample_rate() -> f64 { 1.0 }
fn default_max_series() -> usize { 100000 }
fn default_flush_interval() -> chrono::Duration { chrono::Duration::seconds(30) }

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            budgets: vec![],
            sample_rate: default_sample_rate(),
            max_series_per_metric: default_max_series(),
            flush_interval: default_flush_interval(),
            enable_quarantine: true,
        }
    }
}

/// A unique metric series identified by its label set
#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricSeries {
    pub metric_name: String,
    pub labels: BTreeMap<String, String>,
    pub team: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub sample_count: u64,
    pub is_quarantined: bool,
    pub quarantine_reason: Option<String>,
}

/// Team identifier extractor from labels
#[derive(Clone, Debug)]
pub struct TeamExtractor {
    /// Label keys to try in order for team identification
    team_label_keys: Vec<String>,
    /// Default team if no label found
    default_team: String,
}

impl Default for TeamExtractor {
    fn default() -> Self {
        Self {
            team_label_keys: vec!["team".to_string(), "owner".to_string(), "namespace".to_string()],
            default_team: "unknown".to_string(),
        }
    }
}

impl TeamExtractor {
    pub fn new(team_label_keys: Vec<String>, default_team: String) -> Self {
        Self { team_label_keys, default_team }
    }

    /// Extract team from metric labels
    pub fn extract_team(&self, labels: &BTreeMap<String, String>) -> String {
        for key in &self.team_label_keys {
            if let Some(team) = labels.get(key) {
                return team.clone();
            }
        }
        self.default_team.clone()
    }
}

/// Cardinality collector - tracks unique metric series at ingestion
pub struct CardinalityCollector {
    config: CollectorConfig,
    team_extractor: TeamExtractor,
    /// Active series per metric: metric_name -> (label_hash -> MetricSeries)
    series: Arc<RwLock<HashMap<String, HashMap<u64, MetricSeries>>>>,
    /// Cardinality counts per team per metric
    cardinality_counts: Arc<RwLock<HashMap<String, HashMap<String, u64>>>>, // team -> metric -> count
    /// Prometheus metrics
    metrics: Arc<CollectorMetrics>,
    /// Quarantine manager reference
    quarantine_manager: Option<Arc<crate::cardinality::quarantine::QuarantineManager>>,
}

/// Prometheus metrics for cardinality collection
pub struct CollectorMetrics {
    /// Total series seen per metric
    pub series_total: Family<MetricLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    /// Current cardinality per metric per team
    pub current_cardinality: Family<TeamMetricLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    /// Quarantined series count
    pub quarantined_series: Family<TeamMetricLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    /// Dropped series count
    pub dropped_series: Family<TeamMetricLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    /// Budget utilization
    pub budget_utilization: Family<TeamLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,
    /// Series sample rate histogram
    pub sample_rate: Family<MetricLabels, Histogram>,
}

/// Labels for metric-level metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct MetricLabels {
    pub metric_name: String,
}

/// Labels for team + metric metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TeamMetricLabels {
    pub team: String,
    pub metric_name: String,
}

/// Labels for team-level metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TeamLabels {
    pub team: String,
}

impl CollectorMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            series_total: Family::default(),
            current_cardinality: Family::default(),
            quarantined_series: Family::default(),
            dropped_series: Family::default(),
            budget_utilization: Family::default(),
            sample_rate: Family::default(),
        };

        registry.register(
            "cardinality_series_total",
            "Total unique series seen per metric",
            metrics.series_total.clone(),
        ).unwrap();

        registry.register(
            "cardinality_current",
            "Current cardinality per metric per team",
            metrics.current_cardinality.clone(),
        ).unwrap();

        registry.register(
            "cardinality_quarantined_total",
            "Total quarantined series per metric per team",
            metrics.quarantined_series.clone(),
        ).unwrap();

        registry.register(
            "cardinality_dropped_total",
            "Total dropped series per metric per team",
            metrics.dropped_series.clone(),
        ).unwrap();

        registry.register(
            "cardinality_budget_utilization",
            "Budget utilization percentage per team",
            metrics.budget_utilization.clone(),
        ).unwrap();

        registry.register(
            "cardinality_sample_rate",
            "Sample rate histogram",
            metrics.sample_rate.clone(),
        ).unwrap();

        metrics
    }
}

impl CardinalityCollector {
    /// Create a new cardinality collector
    pub fn new(config: CollectorConfig, registry: &mut Registry) -> Self {
        let metrics = Arc::new(CollectorMetrics::new(registry));
        Self {
            config,
            team_extractor: TeamExtractor::default(),
            series: Arc::new(RwLock::new(HashMap::new())),
            cardinality_counts: Arc::new(RwLock::new(HashMap::new())),
            metrics,
            quarantine_manager: None,
        }
    }

    /// Set quarantine manager
    pub fn with_quarantine_manager(mut self, manager: Arc<crate::cardinality::quarantine::QuarantineManager>) -> Self {
        self.quarantine_manager = Some(manager);
        self
    }

    /// Set custom team extractor
    pub fn with_team_extractor(mut self, extractor: TeamExtractor) -> Self {
        self.team_extractor = extractor;
        self
    }

    /// Process a metric sample and track cardinality
    pub async fn process_sample(
        &self,
        metric_name: &str,
        labels: &BTreeMap<String, String>,
        value: f64,
    ) -> Result<ProcessResult> {
        let team = self.team_extractor.extract_team(labels);
        let label_hash = self.hash_labels(labels);
        let now = Utc::now();

        // Check if this series is quarantined
        if self.is_quarantined(metric_name, label_hash).await {
            self.metrics.dropped_series
                .get_or_create(&TeamMetricLabels {
                    team: team.clone(),
                    metric_name: metric_name.to_string(),
                })
                .inc();
            return Ok(ProcessResult::Dropped { reason: "Series is quarantined".to_string() });
        }

        // Check budget
        let budget_check = self.check_budget(&team, metric_name).await;
        if budget_check.exceeded {
            match budget_check.enforcement_action {
                EnforcementAction::Warn => {
                    warn!(
                        "Cardinality budget exceeded for team {} metric {}: {}/{} ({:.1}%)",
                        team, metric_name, budget_check.current, budget_check.limit,
                        budget_check.current as f64 / budget_check.limit as f64 * 100.0
                    );
                }
                EnforcementAction::Quarantine => {
                    if self.config.enable_quarantine {
                        if let Some(qm) = &self.quarantine_manager {
                            qm.quarantine_series(metric_name, labels.clone(), team.clone(), 
                                format!("Budget exceeded: {}/{}", budget_check.current, budget_check.limit)).await?;
                        }
                        self.metrics.quarantined_series
                            .get_or_create(&TeamMetricLabels {
                                team: team.clone(),
                                metric_name: metric_name.to_string(),
                            })
                            .inc();
                        return Ok(ProcessResult::Quarantined { 
                            reason: format!("Budget exceeded: {}/{}", budget_check.current, budget_check.limit) 
                        });
                    }
                }
                EnforcementAction::Drop => {
                    self.metrics.dropped_series
                        .get_or_create(&TeamMetricLabels {
                            team: team.clone(),
                            metric_name: metric_name.to_string(),
                        })
                        .inc();
                    return Ok(ProcessResult::Dropped { 
                        reason: format!("Budget exceeded: {}/{}", budget_check.current, budget_check.limit) 
                    });
                }
            }
        }

        // Track the series
        let mut series_map = self.series.write().await;
        let metric_series = series_map.entry(metric_name.to_string()).or_default();
        
        let is_new = !metric_series.contains_key(&label_hash);
        if is_new {
            // Check per-metric series limit
            if metric_series.len() >= self.config.max_series_per_metric {
                warn!("Max series per metric reached for {}, dropping sample", metric_name);
                return Ok(ProcessResult::Dropped { reason: "Max series per metric reached".to_string() });
            }
            
            let series = MetricSeries {
                metric_name: metric_name.to_string(),
                labels: labels.clone(),
                team: team.clone(),
                first_seen: now,
                last_seen: now,
                sample_count: 1,
                is_quarantined: false,
                quarantine_reason: None,
            };
            metric_series.insert(label_hash, series);
            
            // Update cardinality count
            let mut counts = self.cardinality_counts.write().await;
            let team_counts = counts.entry(team.clone()).or_default();
            *team_counts.entry(metric_name.to_string()).or_insert(0) += 1;
            
            // Update metrics
            self.metrics.series_total
                .get_or_create(&MetricLabels { metric_name: metric_name.to_string() })
                .inc();
        } else {
            // Update existing series
            if let Some(series) = metric_series.get_mut(&label_hash) {
                series.last_seen = now;
                series.sample_count += 1;
            }
        }

        // Update current cardinality gauge
        let counts = self.cardinality_counts.read().await;
        if let Some(team_counts) = counts.get(&team) {
            if let Some(&count) = team_counts.get(metric_name) {
                self.metrics.current_cardinality
                    .get_or_create(&TeamMetricLabels {
                        team: team.clone(),
                        metric_name: metric_name.to_string(),
                    })
                    .set(count as i64);
            }
        }

        Ok(ProcessResult::Accepted)
    }

    /// Check budget for a team/metric
    async fn check_budget(&self, team: &str, metric_name: &str) -> BudgetCheckResult {
        let counts = self.cardinality_counts.read().await;
        let current = counts.get(team)
            .and_then(|tc| tc.get(metric_name))
            .copied()
            .unwrap_or(0);

        for budget in &self.config.budgets {
            if budget.team == team {
                for selector in &budget.metric_selectors {
                    if self.metric_matches_selector(metric_name, selector) {
                        return BudgetCheckResult {
                            exceeded: current >= budget.limit,
                            current,
                            limit: budget.limit,
                            enforcement_action: budget.enforcement_action.clone(),
                        };
                    }
                }
                // If no selectors, apply to all metrics
                if budget.metric_selectors.is_empty() {
                    return BudgetCheckResult {
                        exceeded: current >= budget.limit,
                        current,
                        limit: budget.limit,
                        enforcement_action: budget.enforcement_action.clone(),
                    };
                }
            }
        }

        BudgetCheckResult {
            exceeded: false,
            current,
            limit: u64::MAX,
            enforcement_action: EnforcementAction::Warn,
        }
    }

    /// Check if metric matches selector
    fn metric_matches_selector(&self, metric_name: &str, selector: &MetricSelector) -> bool {
        // Simple glob matching
        let pattern = &selector.metric_pattern;
        if pattern == "*" || pattern == metric_name {
            return true;
        }
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len()-1];
            return metric_name.starts_with(prefix);
        }
        false
    }

    /// Check if a series is quarantined
    async fn is_quarantined(&self, metric_name: &str, label_hash: u64) -> bool {
        let series = self.series.read().await;
        series.get(metric_name)
            .and_then(|m| m.get(&label_hash))
            .map(|s| s.is_quarantined)
            .unwrap_or(false)
    }

    /// Hash labels for efficient lookup
    fn hash_labels(&self, labels: &BTreeMap<String, String>) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        for (k, v) in labels {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Get current cardinality for a team/metric
    pub async fn get_cardinality(&self, team: &str, metric_name: &str) -> u64 {
        let counts = self.cardinality_counts.read().await;
        counts.get(team)
            .and_then(|tc| tc.get(metric_name))
            .copied()
            .unwrap_or(0)
    }

    /// Get all cardinality counts
    pub async fn get_all_counts(&self) -> HashMap<String, HashMap<String, u64>> {
        self.cardinality_counts.read().await.clone()
    }

    /// Flush metrics to Prometheus
    pub async fn flush_metrics(&self) {
        let counts = self.cardinality_counts.read().await;
        for (team, team_counts) in counts.iter() {
            for (metric, &count) in team_counts {
                self.metrics.current_cardinality
                    .get_or_create(&TeamMetricLabels {
                        team: team.clone(),
                        metric_name: metric.clone(),
                    })
                    .set(count as i64);
            }
            
            // Calculate budget utilization
            for budget in &self.config.budgets {
                if budget.team == team {
                    let total: u64 = team_counts.values().sum();
                    let utilization = if budget.limit > 0 {
                        total as f64 / budget.limit as f64 * 100.0
                    } else {
                        0.0
                    };
                    self.metrics.budget_utilization
                        .get_or_create(&TeamLabels { team: team.clone() })
                        .set(utilization);
                }
            }
        }
    }

    /// Start background flush task
    pub async fn start_flush_task(self: Arc<Self>) {
        let interval = self.config.flush_interval;
        let mut ticker = tokio::time::interval(Duration::from_secs(interval.num_seconds() as u64));
        
        loop {
            ticker.tick().await;
            self.flush_metrics().await;
        }
    }
}

/// Result of processing a metric sample
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessResult {
    Accepted,
    Quarantined { reason: String },
    Dropped { reason: String },
}

/// Budget check result
#[derive(Debug, Clone)]
struct BudgetCheckResult {
    exceeded: bool,
    current: u64,
    limit: u64,
    enforcement_action: EnforcementAction,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn test_collector_new_series() {
        let mut registry = Registry::default();
        let config = CollectorConfig::default();
        let collector = CardinalityCollector::new(config, &mut registry);
        
        let mut labels = BTreeMap::new();
        labels.insert("team".to_string(), "test-team".to_string());
        labels.insert("service".to_string(), "test-service".to_string());
        
        let result = collector.process_sample("test_metric", &labels, 1.0).await.unwrap();
        assert_eq!(result, ProcessResult::Accepted);
    }

    #[tokio::test]
    async fn test_team_extractor() {
        let extractor = TeamExtractor::default();
        
        let mut labels = BTreeMap::new();
        labels.insert("team".to_string(), "my-team".to_string());
        assert_eq!(extractor.extract_team(&labels), "my-team");
        
        let mut labels2 = BTreeMap::new();
        labels2.insert("owner".to_string(), "other-team".to_string());
        assert_eq!(extractor.extract_team(&labels2), "other-team");
        
        let empty = BTreeMap::new();
        assert_eq!(extractor.extract_team(&empty), "unknown");
    }
}