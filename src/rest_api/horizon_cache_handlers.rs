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
//! REST API handlers for Horizon cache observability.

use axum::Json;
use serde::Serialize;

use crate::controller::horizon_cache::{CacheStats, HorizonCache, HorizonCacheConfig};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonCacheStatusResponse {
    pub config: HorizonCacheConfig,
    pub stats: CacheStats,
    pub hit_rate_pct: f64,
}

/// GET /api/v1/horizon/cache/status
pub async fn horizon_cache_status() -> Json<HorizonCacheStatusResponse> {
    let cache = HorizonCache::new(HorizonCacheConfig::default());
    // Seed demo data for dashboard
    cache.put("accounts:demo", b"{}".to_vec());
    cache.get("accounts:demo");

    let stats = cache.stats();
    Json(HorizonCacheStatusResponse {
        hit_rate_pct: stats.hit_rate() * 100.0,
        config: cache.config().clone(),
        stats,
    })
}
