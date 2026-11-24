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
//! Canary analysis for latency regressions

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::latency::deploy_marker::DeployMarker;
use crate::latency::sli_tracker::{LatencySLITracker, SLIDelta, SLIConfig};

/// Canary analysis configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryConfig {
    /// Canary name
    pub name: String,
    /// Service being analyzed
    pub service: String,
    /// SLIs to monitor during canary
    pub sli_names: Vec<String>,
    /// Canary duration
    #[serde(default = "default_duration")]
    pub duration: String,
    /// Traffic percentage for canary (0-100)
    #[serde(default = "default_traffic_percentage")]
    pub traffic_percentage: u8,
    /// Regression threshold (percentage points)
    #[serde(default = "default_regression_threshold")]
    pub regression_threshold: f64,
    /// Error rate threshold
    #[serde(default = "default_error_rate_threshold")]
    pub error_rate_threshold: f64,
    /// Minimum samples required
    #[serde(default = "default_min_samples")]
    pub min_samples: u64,
    /// Auto-promote on success
    #[serde(default)]
    pub auto_promote: bool,
    /// Auto-rollback on failure
    #[serde(default)]
    pub auto_rollback: bool,
}

fn default_duration() -> String { "30m".to_string() }
fn default_traffic_percentage() -> u8 { 10 }
fn default_regression_threshold() -> f64 { 1.0 }
fn default_error_rate_threshold() -> f64 { 0.01 }
fn default_min_samples() -> u64 { 100 }

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            service: String::new(),
            sli_names: vec![],
            duration: Duration::minutes(30),
            traffic_percentage: default_traffic_percentage(),
            regression_threshold: default_regression_threshold(),
            error_rate_threshold: default_error_rate_threshold(),
            min_samples: default_min_samples(),
            auto_promote: false,
            auto_rollback: false,
        }
    }
}

/// Canary phase
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum CanaryPhase {
    #[default]
    NotStarted,
    Running,
    Analyzing,
    Promoted,
    RolledBack,
    Failed,
}

/// Canary analysis result
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryResult {
    pub canary_name: String,
    pub deploy_marker_id: String,
    pub phase: CanaryPhase,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub sli_deltas: Vec<SLIDelta>,
    pub overall_assessment: CanaryAssessment,
    pub regression_detected: bool,
    pub error_rate: f64,
    pub sample_count: u64,
    pub recommendation: CanaryRecommendation,
}

/// Overall canary assessment
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CanaryAssessment {
    Healthy,
    Degraded,
    Critical,
}

/// Canary recommendation
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CanaryRecommendation {
    Promote,
    Rollback,
    Extend,
    ManualReview,
}

/// Canary analyzer for automated latency regression detection
pub struct CanaryAnalyzer {
    config: CanaryConfig,
    sli_tracker: Arc<LatencySLITracker>,
    /// Active canary deployments
    active_canaries: Arc<RwLock<BTreeMap<String, CanaryState>>>,
    /// Prometheus metrics
    metrics: Arc<CanaryMetrics>,
}

/// Internal canary state
#[derive(Clone, Debug)]
struct CanaryState {
    config: CanaryConfig,
    deploy_marker: DeployMarker,
    phase: CanaryPhase,
    started_at: DateTime<Utc>,
    sli_deltas: Vec<SLIDelta>,
    sample_count: u64,
    error_count: u64,
}

/// Prometheus metrics for canary analysis
pub struct CanaryMetrics {
    pub canary_active: Family<CanaryLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub canary_duration_seconds: Family<CanaryLabels, Histogram>,
    pub canary_outcome: Family<CanaryOutcomeLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub sli_regression_detected: Family<CanaryLabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CanaryLabels {
    pub canary_name: String,
    pub service: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CanaryOutcomeLabels {
    pub canary_name: String,
    pub service: String,
    pub outcome: String, // promoted, rolled_back, failed
}

impl CanaryMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            canary_active: Family::default(),
            canary_duration_seconds: Family::default(),
            canary_outcome: Family::default(),
            sli_regression_detected: Family::default(),
        };

        registry.register(
            "canary_active",
            "Currently active canaries",
            metrics.canary_active.clone(),
        ).unwrap();

        registry.register(
            "canary_duration_seconds",
            "Canary analysis duration",
            metrics.canary_duration_seconds.clone(),
        ).unwrap();

        registry.register(
            "canary_outcome_total",
            "Canary outcomes total",
            metrics.canary_outcome.clone(),
        ).unwrap();

        registry.register(
            "canary_sli_regression_detected",
            "SLI regression detected during canary",
            metrics.sli_regression_detected.clone(),
        ).unwrap();

        metrics
    }
}

impl CanaryAnalyzer {
    /// Create a new canary analyzer
    pub fn new(config: CanaryConfig, sli_tracker: Arc<LatencySLITracker>, registry: &mut Registry) -> Self {
        Self {
            config,
            sli_tracker,
            active_canaries: Arc::new(RwLock::new(BTreeMap::new())),
            metrics: Arc::new(CanaryMetrics::new(registry)),
        }
    }

    /// Start a canary analysis for a deployment
    pub async fn start_canary(&self, deploy_marker: DeployMarker) -> Result<CanaryResult> {
        let canary_id = format!("{}-{}", self.config.name, deploy_marker.id);
        
        let state = CanaryState {
            config: self.config.clone(),
            deploy_marker: deploy_marker.clone(),
            phase: CanaryPhase::Running,
            started_at: Utc::now(),
            sli_deltas: vec![],
            sample_count: 0,
            error_count: 0,
        };

        {
            let mut active = self.active_canaries.write().await;
            active.insert(canary_id.clone(), state);
        }

        // Update metrics
        self.metrics.canary_active
            .get_or_create(&CanaryLabels {
                canary_name: self.config.name.clone(),
                service: self.config.service.clone(),
            })
            .inc();

        info!("Started canary analysis: {} for deploy {}", canary_id, deploy_marker.id);

        // Record baseline SLI values
        for sli_name in &self.config.sli_names {
            if let Some(current_value) = self.sli_tracker.get_sli_value(sli_name).await {
                self.sli_tracker.set_baseline(sli_name, &deploy_marker.id, current_value).await;
            }
        }

        Ok(CanaryResult {
            canary_name: self.config.name.clone(),
            deploy_marker_id: deploy_marker.id,
            phase: CanaryPhase::Running,
            started_at: Utc::now(),
            completed_at: None,
            sli_deltas: vec![],
            overall_assessment: CanaryAssessment::Healthy,
            regression_detected: false,
            error_rate: 0.0,
            sample_count: 0,
            recommendation: CanaryRecommendation::ManualReview,
        })
    }

    /// Analyze canary progress (called periodically)
    pub async fn analyze_canary(&self, deploy_marker_id: &str) -> Result<CanaryResult> {
        let canary_id = format!("{}-{}", self.config.name, deploy_marker_id);
        
        let mut active = self.active_canaries.write().await;
        let state = active.get_mut(&canary_id).ok_or_else(|| {
            Error::NotFound(format!("Canary not found: {}", canary_id))
        })?;

        // Check if canary duration elapsed
        let elapsed = Utc::now().signed_duration_since(state.started_at);
        if elapsed >= state.config.duration {
            state.phase = CanaryPhase::Analyzing;
        }

        // Calculate SLI deltas
        let mut all_deltas = Vec::new();
        let mut regression_detected = false;
        let mut worst_delta = 0.0;

        for sli_name in &state.config.sli_names {
            if let Ok(Some(delta)) = self.sli_tracker.calculate_delta(sli_name, &state.deploy_marker).await {
                all_deltas.push(delta.clone());
                
                if delta.is_regression {
                    regression_detected = true;
                    if delta.delta < worst_delta {
                        worst_delta = delta.delta;
                    }
                }
            }
        }

        state.sli_deltas = all_deltas.clone();

        // Calculate error rate (placeholder - would come from actual metrics)
        let error_rate = if state.sample_count > 0 {
            state.error_count as f64 / state.sample_count as f64
        } else {
            0.0
        };

        // Determine assessment and recommendation
        let (assessment, recommendation) = if regression_detected || error_rate > state.config.error_rate_threshold {
            (CanaryAssessment::Critical, CanaryRecommendation::Rollback)
        } else if worst_delta < -0.5 || error_rate > state.config.error_rate_threshold * 0.5 {
            (CanaryAssessment::Degraded, CanaryRecommendation::Extend)
        } else {
            (CanaryAssessment::Healthy, CanaryRecommendation::Promote)
        };

        // Check if analysis complete
        if state.phase == CanaryPhase::Analyzing {
            state.phase = if regression_detected {
                CanaryPhase::RolledBack
            } else {
                CanaryPhase::Promoted
            };
        }

        let result = CanaryResult {
            canary_name: self.config.name.clone(),
            deploy_marker_id: deploy_marker_id.to_string(),
            phase: state.phase.clone(),
            started_at: state.started_at,
            completed_at: if state.phase != CanaryPhase::Running && state.phase != CanaryPhase::Analyzing {
                Some(Utc::now())
            } else {
                None
            },
            sli_deltas: all_deltas,
            overall_assessment: assessment,
            regression_detected,
            error_rate,
            sample_count: state.sample_count,
            recommendation,
        };

        // If canary complete, update metrics and clean up
        if state.phase == CanaryPhase::Promoted || state.phase == CanaryPhase::RolledBack || state.phase == CanaryPhase::Failed {
            let duration = Utc::now().signed_duration_since(state.started_at);
            self.metrics.canary_duration_seconds
                .get_or_create(&CanaryLabels {
                    canary_name: self.config.name.clone(),
                    service: self.config.service.clone(),
                })
                .observe(duration.num_seconds() as f64);

            let outcome = match state.phase {
                CanaryPhase::Promoted => "promoted",
                CanaryPhase::RolledBack => "rolled_back",
                _ => "failed",
            };

            self.metrics.canary_outcome
                .get_or_create(&CanaryOutcomeLabels {
                    canary_name: self.config.name.clone(),
                    service: self.config.service.clone(),
                    outcome: outcome.to_string(),
                })
                .inc();

            self.metrics.canary_active
                .get_or_create(&CanaryLabels {
                    canary_name: self.config.name.clone(),
                    service: self.config.service.clone(),
                })
                .dec();

            if regression_detected {
                self.metrics.sli_regression_detected
                    .get_or_create(&CanaryLabels {
                        canary_name: self.config.name.clone(),
                        service: self.config.service.clone(),
                    })
                    .set(1);
            }

            active.remove(&canary_id);
        }

        Ok(result)
    }

    /// Record a sample for the canary
    pub async fn record_sample(&self, deploy_marker_id: &str, is_error: bool) -> Result<()> {
        let canary_id = format!("{}-{}", self.config.name, deploy_marker_id);
        let mut active = self.active_canaries.write().await;
        
        if let Some(state) = active.get_mut(&canary_id) {
            state.sample_count += 1;
            if is_error {
                state.error_count += 1;
            }
        }
        
        Ok(())
    }

    /// Get canary status
    pub async fn get_canary_status(&self, deploy_marker_id: &str) -> Option<CanaryResult> {
        let canary_id = format!("{}-{}", self.config.name, deploy_marker_id);
        let active = self.active_canaries.read().await;
        
        active.get(&canary_id).map(|state| {
            CanaryResult {
                canary_name: self.config.name.clone(),
                deploy_marker_id: deploy_marker_id.to_string(),
                phase: state.phase.clone(),
                started_at: state.started_at,
                completed_at: None,
                sli_deltas: state.sli_deltas.clone(),
                overall_assessment: CanaryAssessment::Healthy,
                regression_detected: state.sli_deltas.iter().any(|d| d.is_regression),
                error_rate: if state.sample_count > 0 { state.error_count as f64 / state.sample_count as f64 } else { 0.0 },
                sample_count: state.sample_count,
                recommendation: CanaryRecommendation::ManualReview,
            }
        })
    }

    /// Force promote a canary
    pub async fn promote_canary(&self, deploy_marker_id: &str) -> Result<CanaryResult> {
        let canary_id = format!("{}-{}", self.config.name, deploy_marker_id);
        let mut active = self.active_canaries.write().await;
        
        if let Some(state) = active.get_mut(&canary_id) {
            state.phase = CanaryPhase::Promoted;
        }
        
        self.analyze_canary(deploy_marker_id).await
    }

    /// Force rollback a canary
    pub async fn rollback_canary(&self, deploy_marker_id: &str) -> Result<CanaryResult> {
        let canary_id = format!("{}-{}", self.config.name, deploy_marker_id);
        let mut active = self.active_canaries.write().await;
        
        if let Some(state) = active.get_mut(&canary_id) {
            state.phase = CanaryPhase::RolledBack;
        }
        
        self.analyze_canary(deploy_marker_id).await
    }

    /// List active canaries
    pub async fn list_active_canaries(&self) -> Vec<CanaryResult> {
        let active = self.active_canaries.read().await;
        active.values().map(|state| {
            CanaryResult {
                canary_name: self.config.name.clone(),
                deploy_marker_id: state.deploy_marker.id.clone(),
                phase: state.phase.clone(),
                started_at: state.started_at,
                completed_at: None,
                sli_deltas: state.sli_deltas.clone(),
                overall_assessment: CanaryAssessment::Healthy,
                regression_detected: state.sli_deltas.iter().any(|d| d.is_regression),
                error_rate: if state.sample_count > 0 { state.error_count as f64 / state.sample_count as f64 } else { 0.0 },
                sample_count: state.sample_count,
                recommendation: CanaryRecommendation::ManualReview,
            }
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_canary_config_defaults() {
        let config = CanaryConfig::default();
        assert_eq!(config.traffic_percentage, 10);
        assert_eq!(config.regression_threshold, 1.0);
        assert_eq!(config.error_rate_threshold, 0.01);
        assert_eq!(config.min_samples, 100);
    }

    #[test]
    fn test_canary_phase_serialization() {
        let phase = CanaryPhase::Running;
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, "\"Running\"");
    }

    #[test]
    fn test_canary_assessment_serialization() {
        let assessment = CanaryAssessment::Healthy;
        let json = serde_json::to_string(&assessment).unwrap();
        assert_eq!(json, "\"Healthy\"");
    }
}
