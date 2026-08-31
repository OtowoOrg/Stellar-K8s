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
//! Error Budget Tracking and SLO Management
//!
//! This module provides comprehensive error budget tracking, SLO definitions,
//! burn rate alerting, and automated incident creation when budgets are consumed.

use chrono::{DateTime, Duration, Utc};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{family::Family, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::incident::{IncidentManager, IncidentSeverity as IncidentManagerSeverity};

/// SLO (Service Level Objective) definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloDefinition {
    /// Unique identifier for the SLO
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this SLO measures
    pub description: String,
    /// Target service (e.g., "api-gateway", "horizon", "stellar-node")
    pub service: String,
    /// SLI (Service Level Indicator) type
    pub sli_type: SliType,
    /// Target threshold (e.g., 99.9% for availability)
    pub target: f64,
    /// Time window for evaluation (e.g., 30d, 7d, 24h)
    pub window: String,
    /// Labels for metric selection
    pub labels: HashMap<String, String>,
}

/// Types of Service Level Indicators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SliType {
    /// Availability: successful requests / total requests
    Availability,
    /// Latency: percentage of requests under threshold
    Latency,
    /// Quality: percentage of successful operations
    Quality,
    /// Throughput: minimum throughput maintained
    Throughput,
}

/// Error budget status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBudgetStatus {
    /// SLO ID this budget belongs to
    pub slo_id: String,
    /// Current error budget remaining (0.0 - 1.0)
    pub budget_remaining: f64,
    /// Total error budget for the window
    pub total_budget: f64,
    /// Budget consumed so far
    pub budget_consumed: f64,
    /// Current burn rate (budget consumed per hour)
    pub burn_rate: f64,
    /// Time until budget exhaustion at current rate
    pub time_to_exhaustion: Option<Duration>,
    /// Current SLI value
    pub current_sli_value: f64,
    /// Current SLO compliance status
    pub status: SloStatus,
    /// Last updated timestamp
    pub last_updated: DateTime<Utc>,
}

/// SLO compliance status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SloStatus {
    /// Within budget, healthy
    Healthy,
    /// Budget being consumed faster than expected
    Warning,
    /// Budget nearly exhausted
    Critical,
    /// Budget exhausted
    Exhausted,
}

/// Error budget alert configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBudgetAlertConfig {
    /// SLO ID to alert on
    pub slo_id: String,
    /// Warning threshold (percentage of budget consumed)
    pub warning_threshold: f64,
    /// Critical threshold (percentage of budget consumed)
    pub critical_threshold: f64,
    /// Burn rate threshold for fast burn alert (multiplier of normal rate)
    pub fast_burn_multiplier: f64,
    /// Burn rate threshold for slow burn alert
    pub slow_burn_multiplier: f64,
    /// Enable automated incident creation
    pub auto_create_incident: bool,
    /// Incident severity for auto-created incidents
    pub incident_severity: IncidentSeverity,
}

/// Incident severity for auto-created incidents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncidentSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Error budget tracker with Prometheus metrics
pub struct ErrorBudgetTracker {
    /// Registry for metrics
    registry: Arc<Registry>,
    /// SLO definitions
    slos: Arc<RwLock<HashMap<String, SloDefinition>>>,
    /// Alert configurations
    alert_configs: Arc<RwLock<HashMap<String, ErrorBudgetAlertConfig>>>,
    /// Current budget statuses
    budgets: Arc<RwLock<HashMap<String, ErrorBudgetStatus>>>,
    /// Optional incident manager used to open incidents when budgets burn.
    incident_manager: Arc<RwLock<Option<Arc<IncidentManager>>>>,
    /// Prometheus metrics
    metrics: Arc<ErrorBudgetMetrics>,
}

/// Prometheus metrics for error budget tracking
pub struct ErrorBudgetMetrics {
    /// Current error budget remaining (0.0 - 1.0)
    pub budget_remaining: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// Total error budget for the window
    pub total_budget: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// Budget consumed so far
    pub budget_consumed: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// Current burn rate (budget consumed per hour)
    pub burn_rate: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// Time until budget exhaustion in hours
    pub time_to_exhaustion_hours: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// SLO compliance status (0=healthy, 1=warning, 2=critical, 3=exhausted)
    pub slo_status: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// SLO target
    pub slo_target: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// Current SLI value
    pub current_sli_value: Family<SloLabels, Gauge<f64, AtomicU64>>,
    /// Alert firing status
    pub alert_firing: Family<AlertLabels, Gauge<f64, AtomicU64>>,
}

/// Labels for SLO metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SloLabels {
    pub slo_id: String,
    pub service: String,
    pub sli_type: String,
    pub window: String,
}

/// Labels for alert metrics
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AlertLabels {
    pub slo_id: String,
    pub alert_type: String, // "fast_burn", "slow_burn", "warning", "critical"
    pub severity: String,
}

impl ErrorBudgetTracker {
    /// Create a new error budget tracker
    pub fn new(registry: &mut Registry) -> Arc<Self> {
        let metrics = Arc::new(ErrorBudgetMetrics::new(registry));

        Arc::new(Self {
            registry: Arc::new(Registry::default()),
            slos: Arc::new(RwLock::new(HashMap::new())),
            alert_configs: Arc::new(RwLock::new(HashMap::new())),
            budgets: Arc::new(RwLock::new(HashMap::new())),
            incident_manager: Arc::new(RwLock::new(None)),
            metrics,
        })
    }

    pub async fn set_incident_manager(&self, manager: Arc<IncidentManager>) {
        *self.incident_manager.write().await = Some(manager);
    }

    /// Register an SLO definition
    pub async fn register_slo(&self, slo: SloDefinition) {
        let mut slos = self.slos.write().await;
        slos.insert(slo.id.clone(), slo.clone());

        // Initialize budget status
        let mut budgets = self.budgets.write().await;
        budgets.insert(
            slo.id.clone(),
            ErrorBudgetStatus {
                slo_id: slo.id.clone(),
                budget_remaining: 1.0,
                total_budget: 1.0,
                budget_consumed: 0.0,
                burn_rate: 0.0,
                time_to_exhaustion: None,
                current_sli_value: slo.target,
                status: SloStatus::Healthy,
                last_updated: Utc::now(),
            },
        );

        // Initialize metrics
        let labels = SloLabels {
            slo_id: slo.id.clone(),
            service: slo.service.clone(),
            sli_type: format!("{:?}", slo.sli_type),
            window: slo.window.clone(),
        };

        self.metrics
            .slo_target
            .get_or_create(&labels)
            .set(slo.target);

        info!("Registered SLO: {} ({})", slo.name, slo.id);
    }

    /// Register alert configuration
    pub async fn register_alert_config(&self, config: ErrorBudgetAlertConfig) {
        let mut configs = self.alert_configs.write().await;
        configs.insert(config.slo_id.clone(), config.clone());
        info!("Registered alert config for SLO: {}", config.slo_id);
    }

    /// Update SLI value and recalculate error budget
    pub async fn update_sli(&self, slo_id: &str, current_sli: f64) {
        let mut budgets = self.budgets.write().await;

        if let Some(budget) = budgets.get_mut(slo_id) {
            let slos = self.slos.read().await;
            if let Some(slo) = slos.get(slo_id) {
                // Calculate error budget consumed based on SLI vs target
                let error_rate = 1.0 - current_sli / slo.target.max(0.001);
                let error_rate = error_rate.clamp(0.0, 1.0);

                budget.current_sli_value = current_sli;
                budget.budget_consumed = error_rate;
                budget.budget_remaining = 1.0 - error_rate;
                budget.total_budget = 1.0;
                budget.last_updated = Utc::now();

                // Calculate burn rate (errors per hour)
                // Simplified: assume 1 hour window for rate calculation
                budget.burn_rate = error_rate;

                // Calculate time to exhaustion
                if budget.burn_rate > 0.0 {
                    let hours_remaining = budget.budget_remaining / budget.burn_rate;
                    budget.time_to_exhaustion = Some(Duration::hours(hours_remaining as i64));
                } else {
                    budget.time_to_exhaustion = None;
                }

                // Determine status
                budget.status = self.calculate_status(budget.budget_remaining);

                // Update metrics
                let labels = SloLabels {
                    slo_id: slo.id.clone(),
                    service: slo.service.clone(),
                    sli_type: format!("{:?}", slo.sli_type),
                    window: slo.window.clone(),
                };

                self.metrics
                    .budget_remaining
                    .get_or_create(&labels)
                    .set(budget.budget_remaining);
                self.metrics
                    .budget_consumed
                    .get_or_create(&labels)
                    .set(budget.budget_consumed);
                self.metrics
                    .burn_rate
                    .get_or_create(&labels)
                    .set(budget.burn_rate);
                self.metrics
                    .current_sli_value
                    .get_or_create(&labels)
                    .set(current_sli);

                if let Some(ttx) = budget.time_to_exhaustion {
                    self.metrics
                        .time_to_exhaustion_hours
                        .get_or_create(&labels)
                        .set(ttx.num_hours() as f64);
                }

                self.metrics
                    .slo_status
                    .get_or_create(&labels)
                    .set(budget.status as i64 as f64);

                // Check alerts
                self.check_alerts(slo_id, budget).await;
            }
        }
    }

    /// Calculate SLO status based on budget remaining
    fn calculate_status(&self, budget_remaining: f64) -> SloStatus {
        if budget_remaining <= 0.0 {
            SloStatus::Exhausted
        } else if budget_remaining < 0.1 {
            SloStatus::Critical
        } else if budget_remaining < 0.5 {
            SloStatus::Warning
        } else {
            SloStatus::Healthy
        }
    }

    /// Check and fire alerts based on budget status
    async fn check_alerts(&self, slo_id: &str, budget: &ErrorBudgetStatus) {
        let configs = self.alert_configs.read().await;
        let config = configs.get(slo_id);

        if let Some(config) = config {
            let budget_consumed_pct = budget.budget_consumed;

            // Check warning threshold
            if budget_consumed_pct >= config.warning_threshold
                && budget_consumed_pct < config.critical_threshold
            {
                self.fire_alert(slo_id, "warning", config.incident_severity, budget)
                    .await;
            }

            // Check critical threshold
            if budget_consumed_pct >= config.critical_threshold {
                self.fire_alert(slo_id, "critical", config.incident_severity, budget)
                    .await;
            }

            // Check fast burn
            if budget.burn_rate > 0.0 {
                let normal_rate = budget.total_budget / 24.0; // Assuming 24h window
                if budget.burn_rate >= normal_rate * config.fast_burn_multiplier {
                    self.fire_alert(slo_id, "fast_burn", IncidentSeverity::High, budget)
                        .await;
                } else if budget.burn_rate >= normal_rate * config.slow_burn_multiplier {
                    self.fire_alert(slo_id, "slow_burn", IncidentSeverity::Medium, budget)
                        .await;
                }
            }
        }
    }

    /// Fire an alert and optionally create an incident
    async fn fire_alert(
        &self,
        slo_id: &str,
        alert_type: &str,
        severity: IncidentSeverity,
        budget: &ErrorBudgetStatus,
    ) {
        let alert_labels = AlertLabels {
            slo_id: slo_id.to_string(),
            alert_type: alert_type.to_string(),
            severity: format!("{:?}", severity),
        };

        self.metrics
            .alert_firing
            .get_or_create(&alert_labels)
            .set(1.0);

        warn!(
            "Error budget alert fired: slo={}, type={}, severity={:?}, budget_remaining={:.2}%",
            slo_id,
            alert_type,
            severity,
            budget.budget_remaining * 100.0
        );

        let auto_incident = self
            .alert_configs
            .read()
            .await
            .get(slo_id)
            .map(|c| c.auto_create_incident)
            .unwrap_or(false);

        if auto_incident {
            let incident_manager = self.incident_manager.read().await.clone();
            if let Some(manager) = incident_manager {
                let issue_name = match alert_type {
                    "warning" => "Error budget warning",
                    "critical" => "Error budget critical",
                    "fast_burn" => "Fast burn rate budget exhaustion",
                    "slow_burn" => "Slow burn rate budget exhaustion",
                    _ => "Error budget alert",
                };
                let title = format!("{} for {} ({})", issue_name, slo_id, alert_type);
                let description = format!(
                    "Error budget for {} is burning at {:.2}% remaining. Alert type: {}. Severity: {:?}.",
                    slo_id,
                    budget.budget_remaining * 100.0,
                    alert_type,
                    severity
                );
                let incident_severity = match severity {
                    IncidentSeverity::Low => IncidentManagerSeverity::Low,
                    IncidentSeverity::Medium => IncidentManagerSeverity::Medium,
                    IncidentSeverity::High => IncidentManagerSeverity::High,
                    IncidentSeverity::Critical => IncidentManagerSeverity::Critical,
                };
                manager
                    .create_incident(title, description, incident_severity, vec![])
                    .await;
            }
        }
    }

    /// Get current budget status for an SLO
    pub async fn get_budget_status(&self, slo_id: &str) -> Option<ErrorBudgetStatus> {
        self.budgets.read().await.get(slo_id).cloned()
    }

    /// Get all budget statuses
    pub async fn get_all_budget_statuses(&self) -> Vec<ErrorBudgetStatus> {
        self.budgets.read().await.values().cloned().collect()
    }

    /// Get Prometheus metrics for scraping
    pub fn metrics(&self) -> &ErrorBudgetMetrics {
        &self.metrics
    }
}

impl ErrorBudgetMetrics {
    /// Create new error budget metrics
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            budget_remaining: Family::default(),
            total_budget: Family::default(),
            budget_consumed: Family::default(),
            burn_rate: Family::default(),
            time_to_exhaustion_hours: Family::default(),
            slo_status: Family::default(),
            slo_target: Family::default(),
            current_sli_value: Family::default(),
            alert_firing: Family::default(),
        };

        // Register metrics
        registry.register(
            "error_budget_remaining",
            "Current error budget remaining (0.0 - 1.0)",
            metrics.budget_remaining.clone(),
        );

        registry.register(
            "error_budget_total",
            "Total error budget for the window",
            metrics.total_budget.clone(),
        );

        registry.register(
            "error_budget_consumed",
            "Error budget consumed so far",
            metrics.budget_consumed.clone(),
        );

        registry.register(
            "error_budget_burn_rate",
            "Current burn rate (budget consumed per hour)",
            metrics.burn_rate.clone(),
        );

        registry.register(
            "error_budget_time_to_exhaustion_hours",
            "Estimated hours until budget exhaustion",
            metrics.time_to_exhaustion_hours.clone(),
        );

        registry.register(
            "slo_status",
            "SLO compliance status (0=healthy, 1=warning, 2=critical, 3=exhausted)",
            metrics.slo_status.clone(),
        );

        registry.register("slo_target", "SLO target value", metrics.slo_target.clone());

        registry.register(
            "current_sli_value",
            "Current SLI measured value",
            metrics.current_sli_value.clone(),
        );

        registry.register(
            "error_budget_alert_firing",
            "Whether an error budget alert is firing",
            metrics.alert_firing.clone(),
        );

        Self {
            budget_remaining: metrics.budget_remaining,
            total_budget: metrics.total_budget,
            budget_consumed: metrics.budget_consumed,
            burn_rate: metrics.burn_rate,
            time_to_exhaustion_hours: metrics.time_to_exhaustion_hours,
            slo_status: metrics.slo_status,
            slo_target: metrics.slo_target,
            current_sli_value: metrics.current_sli_value,
            alert_firing: metrics.alert_firing,
        }
    }
}

pub fn default_slos() -> Vec<SloDefinition> {
    vec![
        SloDefinition {
            id: "api-availability".to_string(),
            name: "API availability".to_string(),
            description: "Percentage of successful API requests over total requests.".to_string(),
            service: "stellar-api".to_string(),
            sli_type: SliType::Availability,
            target: 99.9,
            window: "30d".to_string(),
            labels: HashMap::from([("route".to_string(), "all".to_string())]),
        },
        SloDefinition {
            id: "api-latency".to_string(),
            name: "API latency".to_string(),
            description: "Percentage of API requests completing under 500ms p95.".to_string(),
            service: "stellar-api".to_string(),
            sli_type: SliType::Latency,
            target: 99.5,
            window: "7d".to_string(),
            labels: HashMap::from([("percentile".to_string(), "p95".to_string())]),
        },
        SloDefinition {
            id: "api-error-rate".to_string(),
            name: "API error rate".to_string(),
            description: "Percentage of requests returning 5xx or transport failures.".to_string(),
            service: "stellar-api".to_string(),
            sli_type: SliType::Quality,
            target: 99.9,
            window: "24h".to_string(),
            labels: HashMap::from([("status_class".to_string(), "5xx".to_string())]),
        },
    ]
}

// Default error budget alert configurations
pub fn default_alert_configs() -> Vec<ErrorBudgetAlertConfig> {
    vec![
        ErrorBudgetAlertConfig {
            slo_id: "api-availability".to_string(),
            warning_threshold: 0.5,
            critical_threshold: 0.9,
            fast_burn_multiplier: 2.0,
            slow_burn_multiplier: 1.5,
            auto_create_incident: true,
            incident_severity: IncidentSeverity::High,
        },
        ErrorBudgetAlertConfig {
            slo_id: "api-latency".to_string(),
            warning_threshold: 0.3,
            critical_threshold: 0.7,
            fast_burn_multiplier: 3.0,
            slow_burn_multiplier: 2.0,
            auto_create_incident: true,
            incident_severity: IncidentSeverity::Medium,
        },
        ErrorBudgetAlertConfig {
            slo_id: "api-error-rate".to_string(),
            warning_threshold: 0.4,
            critical_threshold: 0.8,
            fast_burn_multiplier: 2.5,
            slow_burn_multiplier: 1.5,
            auto_create_incident: true,
            incident_severity: IncidentSeverity::Critical,
        },
        ErrorBudgetAlertConfig {
            slo_id: "stellar-ledger-close".to_string(),
            warning_threshold: 0.4,
            critical_threshold: 0.8,
            fast_burn_multiplier: 2.0,
            slow_burn_multiplier: 1.5,
            auto_create_incident: true,
            incident_severity: IncidentSeverity::High,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::incident::{IncidentManager, IncidentSeverity as IncidentMgmtSeverity};

    #[test]
    fn test_slo_status_calculation() {
        let tracker = ErrorBudgetTracker::new(&mut Registry::default());

        assert_eq!(tracker.calculate_status(1.0), SloStatus::Healthy);
        assert_eq!(tracker.calculate_status(0.6), SloStatus::Healthy);
        assert_eq!(tracker.calculate_status(0.4), SloStatus::Warning);
        assert_eq!(tracker.calculate_status(0.05), SloStatus::Critical);
        assert_eq!(tracker.calculate_status(0.0), SloStatus::Exhausted);
    }

    #[test]
    fn test_sli_type_serialization() {
        let sli = SliType::Availability;
        let json = serde_json::to_string(&sli).unwrap();
        assert_eq!(json, "\"Availability\"");

        let sli = SliType::Latency;
        let json = serde_json::to_string(&sli).unwrap();
        assert_eq!(json, "\"Latency\"");
    }

    #[tokio::test]
    async fn test_default_slos_include_api_availability_latency_and_error_budget_alerts() {
        let slos = default_slos();

        assert!(slos.iter().any(|s| s.id == "api-availability"));
        assert!(slos.iter().any(|s| s.id == "api-latency"));
        assert!(slos.iter().any(|s| s.id == "api-error-rate"));

        let alerts = default_alert_configs();
        assert!(alerts.iter().any(|a| a.slo_id == "api-availability"));
        assert!(alerts.iter().any(|a| a.slo_id == "api-latency"));
        assert!(alerts.iter().any(|a| a.slo_id == "api-error-rate"));
    }

    #[tokio::test]
    async fn test_error_budget_exhaustion_auto_creates_incident() {
        let mut registry = Registry::default();
        let tracker = ErrorBudgetTracker::new(&mut registry);
        let incident_manager = Arc::new(IncidentManager::new());
        tracker.set_incident_manager(incident_manager.clone()).await;

        tracker
            .register_slo(SloDefinition {
                id: "api-availability".to_string(),
                name: "API availability".to_string(),
                description: "Successful API requests over total requests".to_string(),
                service: "api".to_string(),
                sli_type: SliType::Availability,
                target: 99.9,
                window: "30d".to_string(),
                labels: HashMap::new(),
            })
            .await;

        tracker
            .register_alert_config(ErrorBudgetAlertConfig {
                slo_id: "api-availability".to_string(),
                warning_threshold: 0.5,
                critical_threshold: 0.9,
                fast_burn_multiplier: 2.0,
                slow_burn_multiplier: 1.5,
                auto_create_incident: true,
                incident_severity: IncidentSeverity::High,
            })
            .await;

        tracker.update_sli("api-availability", 0.0).await;

        let incidents = incident_manager.list_all().await;
        assert!(!incidents.is_empty());
        assert!(incidents.iter().any(|i| i.title.contains("Error budget")));
    }
}
