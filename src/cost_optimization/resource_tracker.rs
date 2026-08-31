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
//! Resource Usage Tracking and Right-Sizing Recommendations (issue #1413)
//!
//! Tracks actual CPU, memory, and storage utilisation for each workload over
//! a rolling window and computes right-sizing suggestions when a resource is
//! consistently over- or under-provisioned.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Observed resource utilisation for a single workload at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceObservation {
    /// Workload identifier (namespace/name).
    pub workload_id: String,
    pub observed_at: DateTime<Utc>,
    /// CPU request in millicores (provisioned).
    pub cpu_request_m: u32,
    /// CPU limit in millicores (provisioned).
    pub cpu_limit_m: u32,
    /// Observed peak CPU in millicores over the sample interval.
    pub cpu_peak_m: u32,
    /// Observed P95 CPU in millicores.
    pub cpu_p95_m: u32,
    /// Memory request in bytes (provisioned).
    pub memory_request_bytes: u64,
    /// Memory limit in bytes (provisioned).
    pub memory_limit_bytes: u64,
    /// Observed peak RSS in bytes.
    pub memory_peak_bytes: u64,
    /// Observed P95 RSS in bytes.
    pub memory_p95_bytes: u64,
    /// Storage provisioned in bytes.
    pub storage_provisioned_bytes: u64,
    /// Storage actually used in bytes.
    pub storage_used_bytes: u64,
}

impl ResourceObservation {
    /// CPU utilisation ratio (peak / request). Values >1.0 indicate throttling.
    pub fn cpu_utilisation_ratio(&self) -> f64 {
        if self.cpu_request_m == 0 {
            return 1.0;
        }
        self.cpu_p95_m as f64 / self.cpu_request_m as f64
    }

    /// Memory utilisation ratio (P95 RSS / request).
    pub fn memory_utilisation_ratio(&self) -> f64 {
        if self.memory_request_bytes == 0 {
            return 1.0;
        }
        self.memory_p95_bytes as f64 / self.memory_request_bytes as f64
    }

    /// Storage utilisation ratio.
    pub fn storage_utilisation_ratio(&self) -> f64 {
        if self.storage_provisioned_bytes == 0 {
            return 0.0;
        }
        self.storage_used_bytes as f64 / self.storage_provisioned_bytes as f64
    }
}

/// A right-sizing recommendation generated from usage observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RightSizingRecommendation {
    pub workload_id: String,
    pub generated_at: DateTime<Utc>,
    /// Recommended CPU request (millicores).
    pub recommended_cpu_request_m: u32,
    /// Recommended CPU limit (millicores).
    pub recommended_cpu_limit_m: u32,
    /// Recommended memory request (bytes).
    pub recommended_memory_request_bytes: u64,
    /// Recommended memory limit (bytes).
    pub recommended_memory_limit_bytes: u64,
    /// Estimated monthly savings (USD) from applying this recommendation.
    pub estimated_monthly_savings_usd: f64,
    /// Human-readable rationale.
    pub rationale: String,
    /// How confident the engine is in this recommendation (0–1).
    pub confidence: f64,
    /// Current waste percentage (resources provisioned but never used).
    pub waste_pct: f64,
}

/// Configuration for the right-sizing engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RightSizingConfig {
    /// Number of observations to retain per workload.
    pub window_size: usize,
    /// Safety headroom factor applied to P95 when computing recommendations.
    /// A value of 1.2 adds 20% headroom above observed P95.
    pub headroom_factor: f64,
    /// Minimum utilisation ratio below which over-provisioning is flagged.
    pub over_provision_threshold: f64,
    /// USD cost per millicore-hour (approximate).
    pub cpu_cost_per_m_hour: f64,
    /// USD cost per GB-hour of RAM (approximate).
    pub mem_cost_per_gb_hour: f64,
}

impl Default for RightSizingConfig {
    fn default() -> Self {
        Self {
            window_size: 336,                // 2 weeks of hourly samples
            headroom_factor: 1.20,           // 20% headroom
            over_provision_threshold: 0.5,   // flag when using <50% of request
            cpu_cost_per_m_hour: 0.000_048,  // ~$35/vCPU/month
            mem_cost_per_gb_hour: 0.000_006, // ~$4.32/GB/month
        }
    }
}

/// Tracks observations and emits right-sizing recommendations.
pub struct ResourceTracker {
    config: RightSizingConfig,
    // workload_id → ring buffer of observations
    observations: HashMap<String, VecDeque<ResourceObservation>>,
}

impl ResourceTracker {
    pub fn new(config: RightSizingConfig) -> Self {
        Self {
            config,
            observations: HashMap::new(),
        }
    }

    /// Record a new observation for a workload.
    pub fn record(&mut self, obs: ResourceObservation) {
        let window = self
            .observations
            .entry(obs.workload_id.clone())
            .or_default();
        if window.len() >= self.config.window_size {
            window.pop_front();
        }
        window.push_back(obs);
    }

    /// Compute a right-sizing recommendation for the given workload.
    /// Returns `None` if there are fewer than 24 observations.
    pub fn recommend(&self, workload_id: &str) -> Option<RightSizingRecommendation> {
        let window = self.observations.get(workload_id)?;
        if window.len() < 24 {
            return None; // not enough history
        }

        // P95 across the window
        let mut cpu_values: Vec<u32> = window.iter().map(|o| o.cpu_p95_m).collect();
        let mut mem_values: Vec<u64> = window.iter().map(|o| o.memory_p95_bytes).collect();
        cpu_values.sort_unstable();
        mem_values.sort_unstable();
        let p95_idx = (cpu_values.len() as f64 * 0.95) as usize;

        let p95_cpu = cpu_values[p95_idx.min(cpu_values.len() - 1)];
        let p95_mem = mem_values[p95_idx.min(mem_values.len() - 1)];

        let last = window.back()?;
        let rec_cpu = (p95_cpu as f64 * self.config.headroom_factor) as u32;
        let rec_mem = (p95_mem as f64 * self.config.headroom_factor) as u64;
        let rec_cpu_limit = rec_cpu * 2; // limit = 2× request is common practice
        let rec_mem_limit = rec_mem + 128 * 1024 * 1024; // +128 MB for limit

        // Estimate waste
        let cpu_waste_pct = if last.cpu_request_m > 0 {
            (1.0 - last.cpu_p95_m as f64 / last.cpu_request_m as f64) * 100.0
        } else {
            0.0
        };
        let mem_waste_pct = if last.memory_request_bytes > 0 {
            (1.0 - last.memory_p95_bytes as f64 / last.memory_request_bytes as f64) * 100.0
        } else {
            0.0
        };
        let waste_pct = (cpu_waste_pct + mem_waste_pct) / 2.0;

        // Estimate savings: (provisioned - recommended) × cost rate × hours/month
        let hours_per_month = 730.0_f64;
        let cpu_freed_m = last.cpu_request_m.saturating_sub(rec_cpu) as f64;
        let mem_freed_gb =
            (last.memory_request_bytes.saturating_sub(rec_mem)) as f64 / (1024.0 * 1024.0 * 1024.0);
        let savings = (cpu_freed_m * self.config.cpu_cost_per_m_hour
            + mem_freed_gb * self.config.mem_cost_per_gb_hour)
            * hours_per_month;

        let confidence = (window.len() as f64 / self.config.window_size as f64).min(1.0);

        let rationale = format!(
            "Observed P95 CPU {p95_cpu}m (request: {}m), P95 memory {:.0} MB (request: {:.0} MB) \
             over {} samples. Recommending {}m / {:.0} MB with {:.0}% headroom. \
             Estimated waste: {:.1}%.",
            last.cpu_request_m,
            p95_mem as f64 / (1024.0 * 1024.0),
            last.memory_request_bytes as f64 / (1024.0 * 1024.0),
            window.len(),
            rec_cpu,
            rec_mem as f64 / (1024.0 * 1024.0),
            (self.config.headroom_factor - 1.0) * 100.0,
            waste_pct,
        );

        Some(RightSizingRecommendation {
            workload_id: workload_id.to_string(),
            generated_at: Utc::now(),
            recommended_cpu_request_m: rec_cpu,
            recommended_cpu_limit_m: rec_cpu_limit,
            recommended_memory_request_bytes: rec_mem,
            recommended_memory_limit_bytes: rec_mem_limit,
            estimated_monthly_savings_usd: savings.max(0.0),
            rationale,
            confidence,
            waste_pct: waste_pct.max(0.0),
        })
    }

    /// Return recommendations for every tracked workload with enough history.
    pub fn all_recommendations(&self) -> Vec<RightSizingRecommendation> {
        self.observations
            .keys()
            .filter_map(|id| self.recommend(id))
            .collect()
    }

    /// Number of observations for a given workload.
    pub fn observation_count(&self, workload_id: &str) -> usize {
        self.observations
            .get(workload_id)
            .map(|w| w.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_obs(wid: &str, cpu_p95: u32, mem_p95_mb: u64) -> ResourceObservation {
        ResourceObservation {
            workload_id: wid.to_string(),
            observed_at: Utc::now(),
            cpu_request_m: 500,
            cpu_limit_m: 1000,
            cpu_peak_m: cpu_p95 + 50,
            cpu_p95_m: cpu_p95,
            memory_request_bytes: 512 * 1024 * 1024,
            memory_limit_bytes: 1024 * 1024 * 1024,
            memory_peak_bytes: mem_p95_mb * 1024 * 1024 + 10 * 1024 * 1024,
            memory_p95_bytes: mem_p95_mb * 1024 * 1024,
            storage_provisioned_bytes: 100 * 1024 * 1024 * 1024,
            storage_used_bytes: 30 * 1024 * 1024 * 1024,
        }
    }

    #[test]
    fn no_recommendation_below_24_samples() {
        let mut tracker = ResourceTracker::new(RightSizingConfig::default());
        for _ in 0..23 {
            tracker.record(make_obs("app/web", 100, 200));
        }
        assert!(tracker.recommend("app/web").is_none());
    }

    #[test]
    fn recommendation_generated_with_sufficient_samples() {
        let mut tracker = ResourceTracker::new(RightSizingConfig::default());
        for _ in 0..48 {
            tracker.record(make_obs("app/web", 100, 200));
        }
        let rec = tracker.recommend("app/web").unwrap();
        // P95 = 100m, headroom 20% → 120m
        assert_eq!(rec.recommended_cpu_request_m, 120);
        // Savings should be positive (500m provisioned > 120m recommended)
        assert!(rec.estimated_monthly_savings_usd > 0.0);
    }

    #[test]
    fn confidence_grows_with_sample_count() {
        let mut tracker = ResourceTracker::new(RightSizingConfig {
            window_size: 100,
            ..Default::default()
        });
        for _ in 0..50 {
            tracker.record(make_obs("app/api", 80, 150));
        }
        let rec = tracker.recommend("app/api").unwrap();
        assert!(rec.confidence > 0.0 && rec.confidence <= 1.0);
        assert!((rec.confidence - 0.5).abs() < 0.01);
    }

    #[test]
    fn observation_count_tracks_correctly() {
        let mut tracker = ResourceTracker::new(RightSizingConfig::default());
        tracker.record(make_obs("app/x", 100, 100));
        tracker.record(make_obs("app/x", 100, 100));
        assert_eq!(tracker.observation_count("app/x"), 2);
    }

    #[test]
    fn window_is_bounded() {
        let config = RightSizingConfig {
            window_size: 10,
            ..Default::default()
        };
        let mut tracker = ResourceTracker::new(config);
        for _ in 0..20 {
            tracker.record(make_obs("app/y", 100, 100));
        }
        assert_eq!(tracker.observation_count("app/y"), 10);
    }
}
