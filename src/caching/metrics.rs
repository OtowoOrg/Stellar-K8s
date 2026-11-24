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
//! Cache Metrics and Statistics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::debug;

use crate::error::Result;

/// Cache Metrics
pub struct CacheMetrics {
    hits: tokio::sync::RwLock<HashMap<String, u64>>,
    misses: tokio::sync::RwLock<HashMap<String, u64>>,
    invalidations: tokio::sync::RwLock<u64>,
}

impl CacheMetrics {
    pub fn new() -> Self {
        Self {
            hits: tokio::sync::RwLock::new(HashMap::new()),
            misses: tokio::sync::RwLock::new(HashMap::new()),
            invalidations: tokio::sync::RwLock::new(0),
        }
    }

    pub async fn record_hit(&self, cache_level: &str) {
        let mut hits = self.hits.write().await;
        *hits.entry(cache_level.to_string()).or_insert(0) += 1;
    }

    pub async fn record_miss(&self, cache_level: &str) {
        let mut misses = self.misses.write().await;
        *misses.entry(cache_level.to_string()).or_insert(0) += 1;
    }

    pub async fn record_invalidation(&self) {
        let mut invalidations = self.invalidations.write().await;
        *invalidations += 1;
    }

    pub async fn get_statistics(&self) -> Result<MetricsStatistics> {
        let hits = self.hits.read().await;
        let misses = self.misses.read().await;
        let invalidations = self.invalidations.read().await;

        let total_hits: u64 = hits.values().sum();
        let total_misses: u64 = misses.values().sum();
        let hit_rate = if total_hits + total_misses > 0 {
            (total_hits as f64) / ((total_hits + total_misses) as f64)
        } else {
            0.0
        };

        Ok(MetricsStatistics {
            total_hits,
            total_misses,
            hit_rate,
            total_invalidations: *invalidations,
            timestamp: Utc::now(),
        })
    }
}

impl Default for CacheMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricsStatistics {
    pub total_hits: u64,
    pub total_misses: u64,
    pub hit_rate: f64,
    pub total_invalidations: u64,
    pub timestamp: DateTime<Utc>,
}
