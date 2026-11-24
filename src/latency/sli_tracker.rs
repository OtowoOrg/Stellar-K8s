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
//! Latency SLI tracker with windowed calculations

use std::collections::{BTreeMap, VecDeque};
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

use crate::error::{Error, Result};
use crate::latency::deploy_marker::DeployMarker;

/// SLI configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SLIConfig {
    /// SLI name
    pub name: String,
    /// Service this SLI applies to
    pub service: String,
    /// Latency threshold in milliseconds (p99, p95, etc.)
    pub threshold_ms: f64,
    /// Target percentage (e.g., 99.9 for 99.9% under threshold)
    pub target_percentage: f64,
    /// Evaluation window
    pub window: SLIWindow,
    /// Labels for metric grouping
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

/// SLI evaluation window
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SLIWindow {
    /// Last 5 minutes
    FiveMinutes,
    /// Last 15 minutes
    FifteenMinutes,
    /// Last hour
    OneHour,
    /// Last 6 hours
    SixHours,
    /// Last 24 hours
    TwentyFourHours,
    /// Last 7 days
    SevenDays,
    /// Last 30 days
    ThirtyDays,
    /// Custom window in seconds
    Custom(u64),
}

impl SLIWindow {
    pub fn duration(&self) -> Duration {
        match self {
            SLIWindow::FiveMinutes => Duration::from_secs(5 * 60),
            SLIWindow::FifteenMinutes => Duration::from_secs(15 * 60),
            SLIWindow::OneHour => Duration::from_secs(60 * 60),
            SLIWindow::SixHours => Duration::from_secs(6 * 60 * 60),
            SLIWindow::TwentyFourHours => Duration::from_secs(24 * 60 * 60),
            SLIWindow::SevenDays => Duration::from_secs(7 * 24 * 60 * 60),
            SLIWindow::ThirtyDays => Duration::from_secs(30 * 24 * 60 * 60),
            SLIWindow::Custom(secs) => Duration::from_secs(*secs),
        }
    }
}

/// SLI delta calculation result
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SLIDelta {
    pub sli_name: String,
    pub service: String,
    pub deploy_marker_id: String,
    pub pre_deploy_value: f64,
    pub post_deploy_value: f64,
    pub delta: f64,
    pub delta_percentage: f64,
    pub is_regression: bool,
    pub regression_threshold: f64,
    pub calculated_at: DateTime<Utc>,
    pub window: SLIWindow,
}

/// Latency sample for SLI calculation
#[derive(Clone, Debug)]
struct LatencySample {
    timestamp: DateTime<Utc>,
    latency_ms: f64,
    labels: BTreeMap<String, String>,
}

/// SLI tracker for latency budgets
pub struct LatencySLITracker {
    configs: Arc<RwLock<BTreeMap<String, SLIConfig>>>,
    /// Latency samples per SLI: sli_name -> VecDeque<LatencySample>
    samples: Arc<RwLock<BTreeMap<String, VecDeque<LatencySample>>>>,
    /// Baseline values per SLI per deploy marker
    baselines: Arc<RwLock<BTreeMap<String, BTreeMap<String, f64>>>>, // sli_name -> deploy_id -> baseline
    /// Prometheus metrics
    metrics: Arc<SLIMetrics>,
    /// Max samples per SLI
    max_samples_per_sli: usize,
}

/// Prometheus metrics for SLI tracking
pub struct SLIMetrics {
    pub sli_value: Family<SLILabels, Gauge<f64, std::sync::atomic::AtomicU64>>,
    pub sli_target: Family<SLILabels, Gauge<f64, std::sync::atomic::AtomicU64>>,
    pub sli_delta: Family<SLIDeltaLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,
    pub sli_regression: Family<SLILabels, Gauge<u64, std::sync::atomic::AtomicU64>>,
    pub latency_histogram: Family<SLILabels, Histogram>,
    pub samples_count: Family<SLILabels, Counter<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SLILabels {
    pub sli_name: String,
    pub service: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SLIDeltaLabels {
    pub sli_name: String,
    pub service: String,
    pub deploy_id: String,
}

impl SLIMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            sli_value: Family::default(),
            sli_target: Family::default(),
            sli_delta: Family::default(),
            sli_regression: Family::default(),
            latency_histogram: Family::default(),
            samples_count: Family::default(),
        };

        registry.register(
            "latency_sli_value",
            "Current SLI value (percentage under threshold)",
            metrics.sli_value.clone(),
        ).unwrap();

        registry.register(
            "latency_sli_target",
            "SLI target percentage",
            metrics.sli_target.clone(),
        ).unwrap();

        registry.register(
            "latency_sli_delta",
            "SLI delta from pre-deploy baseline",
            metrics.sli_delta.clone(),
        ).unwrap();

        registry.register(
            "latency_sli_regression",
            "Whether SLI shows regression (1=yes, 0=no)",
            metrics.sli_regression.clone(),
        ).unwrap();

        registry.register(
            "latency_sli_histogram",
            "Latency distribution for SLI calculation",
            metrics.latency_histogram.clone(),
        ).unwrap();

        registry.register(
            "latency_sli_samples_total",
            "Total latency samples processed",
            metrics.samples_count.clone(),
        ).unwrap();

        metrics
    }
}

impl LatencySLITracker {
    /// Create a new SLI tracker
    pub fn new(registry: &mut Registry) -> Self {
        Self {
            configs: Arc::new(RwLock::new(BTreeMap::new())),
            samples: Arc::new(RwLock::new(BTreeMap::new())),
            baselines: Arc::new(RwLock::new(BTreeMap::new())),
            metrics: Arc::new(SLIMetrics::new(registry)),
            max_samples_per_sli: 100000,
        }
    }

    /// Register an SLI configuration
    pub async fn register_sli(&self, config: SLIConfig) {
        let mut configs = self.configs.write().await;
        configs.insert(config.name.clone(), config.clone());
        
        // Initialize metrics
        self.metrics.sli_target
            .get_or_create(&SLILabels {
                sli_name: config.name.clone(),
                service: config.service.clone(),
            })
            .set(config.target_percentage);
        
        info!("Registered SLI: {} for service {}", config.name, config.service);
    }

    /// Record a latency sample
    pub async fn record_latency(
        &self,
        sli_name: &str,
        latency_ms: f64,
        labels: BTreeMap<String, String>,
    ) -> Result<()> {
        let configs = self.configs.read().await;
        let config = configs.get(sli_name).ok_or_else(|| {
            Error::NotFound(format!("SLI not found: {}", sli_name))
        })?;

        let sample = LatencySample {
            timestamp: Utc::now(),
            latency_ms,
            labels,
        };

        // Store sample
        {
            let mut samples = self.samples.write().await;
            let sli_samples = samples.entry(sli_name.to_string()).or_default();
            sli_samples.push_back(sample);
            
            // Trim old samples beyond window
            let cutoff = Utc::now() - config.window.duration();
            while let Some(front) = sli_samples.front() {
                if front.timestamp < cutoff {
                    sli_samples.pop_front();
                } else {
                    break;
                }
            }
            
            // Enforce max samples
            if sli_samples.len() > self.max_samples_per_sli {
                let to_remove = sli_samples.len() - self.max_samples_per_sli;
                for _ in 0..to_remove {
                    sli_samples.pop_front();
                }
            }
        }

        // Update metrics
        self.metrics.samples_count
            .get_or_create(&SLILabels {
                sli_name: sli_name.to_string(),
                service: config.service.clone(),
            })
            .inc();

        self.metrics.latency_histogram
            .get_or_create(&SLILabels {
                sli_name: sli_name.to_string(),
                service: config.service.clone(),
            })
            .observe(latency_ms);

        // Recalculate SLI value
        self.recalculate_sli(sli_name).await?;

        Ok(())
    }

    /// Recalculate SLI value from samples
    async fn recalculate_sli(&self, sli_name: &str) -> Result<()> {
        let configs = self.configs.read().await;
        let config = configs.get(sli_name).ok_or_else(|| {
            Error::NotFound(format!("SLI not found: {}", sli_name))
        })?;

        let samples = self.samples.read().await;
        let sli_samples = samples.get(sli_name);
        
        let value = if let Some(samples) = sli_samples {
            if samples.is_empty() {
                config.target_percentage // Default to target if no samples
            } else {
                let under_threshold = samples.iter()
                    .filter(|s| s.latency_ms <= config.threshold_ms)
                    .count();
                (under_threshold as f64 / samples.len() as f64) * 100.0
            }
        } else {
            config.target_percentage
        };

        // Update metric
        self.metrics.sli_value
            .get_or_create(&SLILabels {
                sli_name: sli_name.to_string(),
                service: config.service.clone(),
            })
            .set(value);

        Ok(())
    }

    /// Set pre-deploy baseline for an SLI
    pub async fn set_baseline(&self, sli_name: &str, deploy_id: &str, value: f64) {
        let mut baselines = self.baselines.write().await;
        let sli_baselines = baselines.entry(sli_name.to_string()).or_default();
        sli_baselines.insert(deploy_id.to_string(), value);
        
        debug!("Set baseline for SLI {} deploy {}: {:.2}%", sli_name, deploy_id, value);
    }

    /// Calculate SLI delta from pre-deploy baseline
    pub async fn calculate_delta(&self, sli_name: &str, deploy_marker: &DeployMarker) -> Result<Option<SLIDelta>> {
        let configs = self.configs.read().await;
        let config = configs.get(sli_name).ok_or_else(|| {
            Error::NotFound(format!("SLI not found: {}", sli_name))
        })?;

        // Get current SLI value
        let samples = self.samples.read().await;
        let sli_samples = samples.get(sli_name);
        
        let current_value = if let Some(samples) = sli_samples {
            if samples.is_empty() {
                return Ok(None);
            }
            let under_threshold = samples.iter()
                .filter(|s| s.latency_ms <= config.threshold_ms)
                .count();
            (under_threshold as f64 / samples.len() as f64) * 100.0
        } else {
            return Ok(None);
        };

        // Get pre-deploy baseline
        let baselines = self.baselines.read().await;
        let pre_deploy_value = baselines.get(sli_name)
            .and_then(|b| b.get(&deploy_marker.id))
            .copied()
            .or_else(|| deploy_marker.pre_deploy_baselines.get(sli_name).copied());

        if let Some(baseline) = pre_deploy_value {
            let delta = current_value - baseline;
            let delta_percentage = if baseline != 0.0 {
                (delta / baseline) * 100.0
            } else {
                0.0
            };
            
            // Consider regression if delta is negative and exceeds threshold (e.g., 1% drop)
            let regression_threshold = 1.0; // 1 percentage point
            let is_regression = delta < -regression_threshold;

            let delta_result = SLIDelta {
                sli_name: sli_name.to_string(),
                service: config.service.clone(),
                deploy_marker_id: deploy_marker.id.clone(),
                pre_deploy_value: baseline,
                post_deploy_value: current_value,
                delta,
                delta_percentage,
                is_regression,
                regression_threshold,
                calculated_at: Utc::now(),
                window: config.window.clone(),
            };

            // Update metrics
            self.metrics.sli_delta
                .get_or_create(&SLIDeltaLabels {
                    sli_name: sli_name.to_string(),
                    service: config.service.clone(),
                    deploy_id: deploy_marker.id.clone(),
                })
                .set(delta);

            self.metrics.sli_regression
                .get_or_create(&SLILabels {
                    sli_name: sli_name.to_string(),
                    service: config.service.clone(),
                })
                .set(if is_regression { 1 } else { 0 });

            if is_regression {
                warn!(
                    "SLI REGRESSION detected: {} deploy {} baseline={:.2}% current={:.2}% delta={:.2}%",
                    sli_name, deploy_marker.id, baseline, current_value, delta
                );
            } else {
                info!(
                    "SLI delta calculated: {} deploy {} baseline={:.2}% current={:.2}% delta={:.2}%",
                    sli_name, deploy_marker.id, baseline, current_value, delta
                );
            }

            Ok(Some(delta_result))
        } else {
            debug!("No baseline for SLI {} deploy {}", sli_name, deploy_marker.id);
            Ok(None)
        }
    }

    /// Get current SLI value
    pub async fn get_sli_value(&self, sli_name: &str) -> Option<f64> {
        let samples = self.samples.read().await;
        let configs = self.configs.read().await;
        let config = configs.get(sli_name)?;
        
        samples.get(sli_name).map(|sli_samples| {
            if sli_samples.is_empty() {
                config.target_percentage
            } else {
                let under_threshold = sli_samples.iter()
                    .filter(|s| s.latency_ms <= config.threshold_ms)
                    .count();
                (under_threshold as f64 / sli_samples.len() as f64) * 100.0
            }
        })
    }

    /// Get all SLI deltas for a deploy marker
    pub async fn get_deltas_for_deploy(&self, deploy_id: &str) -> Vec<SLIDelta> {
        let configs = self.configs.read().await;
        let mut deltas = Vec::new();
        
        for (sli_name, config) in configs.iter() {
            if config.service == deploy_id.split('-').next().unwrap_or("") || true {
                // In real impl, match service from deploy marker
                if let Ok(Some(delta)) = self.calculate_delta(sli_name, &DeployMarker {
                    id: deploy_id.to_string(),
                    ..Default::default()
                }).await {
                    deltas.push(delta);
                }
            }
        }
        
        deltas
    }

    /// Get all registered SLIs
    pub async fn get_all_slis(&self) -> Vec<SLIConfig> {
        self.configs.read().await.values().cloned().collect()
    }
}

impl Default for DeployMarker {
    fn default() -> Self {
        Self {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            service: String::new(),
            environment: String::new(),
            commit_sha: String::new(),
            git_ref: None,
            version: None,
            strategy: Default::default(),
            deployed_by: None,
            change_ids: vec![],
            metadata: BTreeMap::new(),
            pre_deploy_baselines: BTreeMap::new(),
        }
    }
}

/// Duration serialization helper
pub mod duration_serde {
    use chrono::Duration;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}s", duration.num_seconds()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.ends_with('s') {
            let secs = s.trim_end_matches('s').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::seconds(secs))
        } else if s.ends_with('m') {
            let mins = s.trim_end_matches('m').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::minutes(mins))
        } else if s.ends_with('h') {
            let hours = s.trim_end_matches('h').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::hours(hours))
        } else if s.ends_with('d') {
            let days = s.trim_end_matches('d').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::days(days))
        } else {
            Err(serde::de::Error::custom("duration must end with s, m, h, or d"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn test_sli_tracker_basic() {
        let mut registry = Registry::default();
        let tracker = LatencySLITracker::new(&mut registry);
        
        let config = SLIConfig {
            name: "api_latency_p99".to_string(),
            service: "api-gateway".to_string(),
            threshold_ms: 500.0,
            target_percentage: 99.9,
            window: SLIWindow::FiveMinutes,
            labels: BTreeMap::new(),
        };
        
        tracker.register_sli(config).await;
        
        // Record some latency samples
        for i in 0..100 {
            let mut labels = BTreeMap::new();
            labels.insert("endpoint".to_string(), "/api/test".to_string());
            tracker.record_latency("api_latency_p99", i as f64 * 5.0, labels).await.unwrap();
        }
        
        let value = tracker.get_sli_value("api_latency_p99").await.unwrap();
        assert!(value > 0.0);
    }

    #[test]
    fn test_sli_window_duration() {
        assert_eq!(SLIWindow::FiveMinutes.duration(), Duration::from_secs(300));
        assert_eq!(SLIWindow::OneHour.duration(), Duration::from_secs(3600));
        assert_eq!(SLIWindow::Custom(123).duration(), Duration::from_secs(123));
    }
}