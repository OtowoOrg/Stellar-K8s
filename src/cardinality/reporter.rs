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
//! Cardinality budget reporter

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client,
};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::cardinality::budget::{BudgetPhase, BudgetSpec, BudgetStatus, CardinalityBudget};
use crate::cardinality::collector::CardinalityCollector;
use crate::cardinality::cost_allocation::{CardinalityCostAllocator, CostAllocationReport};
use crate::cardinality::quarantine::QuarantineManager;
use crate::error::{Error, Result};

/// Reporter configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReporterConfig {
    /// Report interval
    #[serde(with = "crate::cardinality::budget::duration_serde")]
    #[serde(default = "default_report_interval")]
    pub report_interval: Duration,
    /// Enable cost allocation reporting
    #[serde(default)]
    pub enable_cost_reporting: bool,
    /// Enable quarantine reporting
    #[serde(default)]
    pub enable_quarantine_reporting: bool,
    /// Alert on budget exceeded
    #[serde(default)]
    pub alert_on_exceeded: bool,
    /// Webhook URL for alerts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_webhook_url: Option<String>,
}

fn default_report_interval() -> Duration { Duration::minutes(5) }

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
            report_interval: default_report_interval(),
            enable_cost_reporting: true,
            enable_quarantine_reporting: true,
            alert_on_exceeded: true,
            alert_webhook_url: None,
        }
    }
}

/// Cardinality reporter for budget monitoring and alerting
pub struct CardinalityReporter {
    config: ReporterConfig,
    collector: Arc<CardinalityCollector>,
    quarantine_manager: Option<Arc<QuarantineManager>>,
    cost_allocator: Option<Arc<CardinalityCostAllocator>>,
    /// Prometheus metrics
    metrics: Arc<ReporterMetrics>,
    /// Alert state tracking
    alert_states: Arc<RwLock<BTreeMap<String, AlertState>>>,
}

/// Alert state for deduplication
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlertState {
    alert_key: String,
    first_fired: DateTime<Utc>,
    last_fired: DateTime<Utc>,
    fire_count: u32,
    acknowledged: bool,
}

/// Prometheus metrics for reporter
pub struct ReporterMetrics {
    pub budget_phase: Family<BudgetPhaseLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub budget_utilization: Family<TeamLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,
    pub alerts_fired_total: Family<AlertLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub reports_generated_total: Counter<u64, std::sync::atomic::AtomicU64>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct BudgetPhaseLabels {
    pub team: String,
    pub metric_name: String,
    pub phase: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TeamLabels {
    pub team: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AlertLabels {
    pub team: String,
    pub alert_type: String,
    pub severity: String,
}

impl ReporterMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            budget_phase: Family::default(),
            budget_utilization: Family::default(),
            alerts_fired_total: Family::default(),
            reports_generated_total: Counter::default(),
        };

        registry.register(
            "cardinality_budget_phase",
            "Current budget phase (0=healthy, 1=warning, 2=critical, 3=exceeded)",
            metrics.budget_phase.clone(),
        ).unwrap();

        registry.register(
            "cardinality_budget_utilization",
            "Budget utilization percentage per team",
            metrics.budget_utilization.clone(),
        ).unwrap();

        registry.register(
            "cardinality_alerts_fired_total",
            "Total alerts fired per team per alert type",
            metrics.alerts_fired_total.clone(),
        ).unwrap();

        registry.register(
            "cardinality_reports_generated_total",
            "Total reports generated",
            metrics.reports_generated_total.clone(),
        ).unwrap();

        metrics
    }
}

impl CardinalityReporter {
    /// Create a new reporter
    pub fn new(
        config: ReporterConfig,
        collector: Arc<CardinalityCollector>,
        registry: &mut Registry,
    ) -> Self {
        let metrics = Arc::new(ReporterMetrics::new(registry));
        Self {
            config,
            collector,
            quarantine_manager: None,
            cost_allocator: None,
            metrics,
            alert_states: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    /// Set quarantine manager
    pub fn with_quarantine_manager(mut self, manager: Arc<QuarantineManager>) -> Self {
        self.quarantine_manager = Some(manager);
        self
    }

    /// Set cost allocator
    pub fn with_cost_allocator(mut self, allocator: Arc<CardinalityCostAllocator>) -> Self {
        self.cost_allocator = Some(allocator);
        self
    }

    /// Run a single reporting cycle
    pub async fn report_cycle(&self) -> Result<ReportSummary> {
        let now = Utc::now();
        let counts = self.collector.get_all_counts().await;
        
        let mut summary = ReportSummary {
            timestamp: now,
            teams_checked: 0,
            budgets_warning: 0,
            budgets_critical: 0,
            budgets_exceeded: 0,
            total_quarantined: 0,
            alerts_fired: 0,
            cost_report: None,
        };

        // Check each team's budgets
        for (team, team_counts) in &counts {
            summary.teams_checked += 1;
            
            let team_total: u64 = team_counts.values().sum();
            
            // Get budget for team (simplified - would come from CR in real impl)
            let budget_limit = self.get_budget_limit(&team).await;
            
            if budget_limit > 0 {
                let utilization = team_total as f64 / budget_limit as f64 * 100.0;
                
                // Update utilization metric
                self.metrics.budget_utilization
                    .get_or_create(&TeamLabels { team: team.clone() })
                    .set(utilization);
                
                // Determine phase
                let phase = if utilization >= 100.0 {
                    BudgetPhase::Exceeded
                } else if utilization >= 90.0 {
                    BudgetPhase::Critical
                } else if utilization >= 70.0 {
                    BudgetPhase::Warning
                } else {
                    BudgetPhase::Healthy
                };

                // Update phase metric
                for (metric, &count) in team_counts {
                    let metric_utilization = if budget_limit > 0 {
                        count as f64 / budget_limit as f64 * 100.0
                    } else {
                        0.0
                    };
                    
                    let metric_phase = if metric_utilization >= 100.0 {
                        BudgetPhase::Exceeded
                    } else if metric_utilization >= 90.0 {
                        BudgetPhase::Critical
                    } else if metric_utilization >= 70.0 {
                        BudgetPhase::Warning
                    } else {
                        BudgetPhase::Healthy
                    };

                    self.metrics.budget_phase
                        .get_or_create(&BudgetPhaseLabels {
                            team: team.clone(),
                            metric_name: metric.clone(),
                            phase: format!("{:?}", metric_phase),
                        })
                        .set(1);

                    // Fire alerts on phase transitions
                    if self.config.alert_on_exceeded {
                        self.check_and_fire_alert(&team, metric, &metric_phase, metric_utilization).await;
                    }
                }

                match phase {
                    BudgetPhase::Warning => summary.budgets_warning += 1,
                    BudgetPhase::Critical => summary.budgets_critical += 1,
                    BudgetPhase::Exceeded => summary.budgets_exceeded += 1,
                    _ => {}
                }
            }
        }

        // Quarantine reporting
        if let Some(qm) = &self.quarantine_manager {
            if self.config.enable_quarantine_reporting {
                let quarantined = qm.get_all_quarantined().await;
                summary.total_quarantined = quarantined.len() as u64;
                
                for qs in &quarantined {
                    debug!(
                        "Quarantined: team={} metric={} labels={:?} reason={}",
                        qs.team, qs.metric_name, qs.labels, qs.reason
                    );
                }
            }
        }

        // Cost allocation reporting
        if let Some(ca) = &self.cost_allocator {
            if self.config.enable_cost_reporting {
                let period_start = now - self.config.report_interval;
                let report = ca.generate_report(period_start, now).await?;
                summary.cost_report = Some(report);
            }
        }

        self.metrics.reports_generated_total.inc();

        info!(
            "Cardinality report: teams={}, warning={}, critical={}, exceeded={}, quarantined={}",
            summary.teams_checked, summary.budgets_warning, summary.budgets_critical,
            summary.budgets_exceeded, summary.total_quarantined
        );

        Ok(summary)
    }

    /// Check and fire alert on phase transition
    async fn check_and_fire_alert(&self, team: &str, metric: &str, phase: &BudgetPhase, utilization: f64) {
        let alert_key = format!("{}:{}:{:?}", team, metric, phase);
        let mut alert_states = self.alert_states.write().await;
        
        let should_fire = match alert_states.get(&alert_key) {
            Some(state) => {
                // Only re-fire if utilization increased significantly or it's been a while
                let time_since_last = Utc::now().signed_duration_since(state.last_fired);
                time_since_last > Duration::minutes(15) || utilization > 100.0
            }
            None => true,
        };

        if should_fire {
            let severity = match phase {
                BudgetPhase::Exceeded => "critical",
                BudgetPhase::Critical => "high",
                BudgetPhase::Warning => "medium",
                _ => "low",
            };

            // Update alert state
            alert_states.insert(alert_key.clone(), AlertState {
                alert_key: alert_key.clone(),
                first_fired: alert_states.get(&alert_key).map(|s| s.first_fired).unwrap_or_else(Utc::now),
                last_fired: Utc::now(),
                fire_count: alert_states.get(&alert_key).map(|s| s.fire_count + 1).unwrap_or(1),
                acknowledged: false,
            });

            // Record metric
            self.metrics.alerts_fired_total
                .get_or_create(&AlertLabels {
                    team: team.to_string(),
                    alert_type: format!("budget_{:?}", phase).to_lowercase(),
                    severity: severity.to_string(),
                })
                .inc();

            // Send webhook alert if configured
            if let Some(webhook_url) = &self.config.alert_webhook_url {
                self.send_webhook_alert(webhook_url, team, metric, phase, utilization, severity).await;
            }

            warn!(
                "CARDINALITY ALERT: team={} metric={} phase={:?} utilization={:.1}%",
                team, metric, phase, utilization
            );
        }
    }

    /// Send alert via webhook
    async fn send_webhook_alert(
        &self,
        url: &str,
        team: &str,
        metric: &str,
        phase: &BudgetPhase,
        utilization: f64,
        severity: &str,
    ) {
        let payload = serde_json::json!({
            "alert": "CardinalityBudgetExceeded",
            "team": team,
            "metric": metric,
            "phase": format!("{:?}", phase),
            "utilization_percent": utilization,
            "severity": severity,
            "timestamp": Utc::now().to_rfc3339(),
        });

        let client = reqwest::Client::new();
        if let Err(e) = client.post(url).json(&payload).send().await {
            tracing::error!("Failed to send cardinality alert webhook: {}", e);
        }
    }

    /// Get budget limit for a team (placeholder - would query CRs in real impl)
    async fn get_budget_limit(&self, team: &str) -> u64 {
        // In real implementation, query CardinalityBudget CRs
        // For now, return a default
        10000
    }

    /// Start periodic reporting task
    pub async fn start_reporting_task(self: Arc<Self>) {
        let interval = self.config.report_interval;
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval.num_seconds() as u64));
        
        loop {
            ticker.tick().await;
            if let Err(e) = self.report_cycle().await {
                tracing::error!("Report cycle failed: {}", e);
            }
        }
    }

    /// Get current alert states
    pub async fn get_alert_states(&self) -> Vec<AlertState> {
        self.alert_states.read().await.values().cloned().collect()
    }

    /// Acknowledge an alert
    pub async fn acknowledge_alert(&self, team: &str, metric: &str, phase: &BudgetPhase) {
        let alert_key = format!("{}:{}:{:?}", team, metric, phase);
        let mut alert_states = self.alert_states.write().await;
        if let Some(state) = alert_states.get_mut(&alert_key) {
            state.acknowledged = true;
        }
    }
}

/// Report summary
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    pub timestamp: DateTime<Utc>,
    pub teams_checked: u32,
    pub budgets_warning: u32,
    pub budgets_critical: u32,
    pub budgets_exceeded: u32,
    pub total_quarantined: u64,
    pub alerts_fired: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_report: Option<CostAllocationReport>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_reporter_config_defaults() {
        let config = ReporterConfig::default();
        assert_eq!(config.report_interval, Duration::minutes(5));
        assert!(config.enable_cost_reporting);
        assert!(config.enable_quarantine_reporting);
        assert!(config.alert_on_exceeded);
    }

    #[test]
    fn test_budget_phase_determination() {
        assert_eq!(determine_phase(50.0), BudgetPhase::Healthy);
        assert_eq!(determine_phase(75.0), BudgetPhase::Warning);
        assert_eq!(determine_phase(95.0), BudgetPhase::Critical);
        assert_eq!(determine_phase(100.0), BudgetPhase::Exceeded);
        assert_eq!(determine_phase(150.0), BudgetPhase::Exceeded);
    }

    fn determine_phase(utilization: f64) -> BudgetPhase {
        if utilization >= 100.0 {
            BudgetPhase::Exceeded
        } else if utilization >= 90.0 {
            BudgetPhase::Critical
        } else if utilization >= 70.0 {
            BudgetPhase::Warning
        } else {
            BudgetPhase::Healthy
        }
    }
}
