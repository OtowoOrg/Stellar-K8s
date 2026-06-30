//! API analytics and usage tracking.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// A single recorded API request event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestEvent {
    pub timestamp: String,
    pub route_id: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub latency_ms: u64,
    pub api_key_id: Option<String>,
    pub version: String,
}

/// Aggregated usage stats per route.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteStats {
    pub total_requests: u64,
    pub error_count: u64,
    pub total_latency_ms: u64,
}

impl RouteStats {
    pub fn mean_latency_ms(&self) -> u64 {
        self.total_latency_ms
            .checked_div(self.total_requests)
            .unwrap_or(0)
    }
}

/// Thread-safe analytics store.
#[derive(Clone, Default)]
pub struct AnalyticsStore {
    events: Arc<RwLock<Vec<RequestEvent>>>,
    stats: Arc<RwLock<HashMap<String, RouteStats>>>,
    capacity: usize,
}

impl AnalyticsStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::with_capacity(capacity))),
            stats: Arc::new(RwLock::new(HashMap::new())),
            capacity,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        &self,
        route_id: impl Into<String>,
        method: impl Into<String>,
        path: impl Into<String>,
        status: u16,
        latency: Duration,
        api_key_id: Option<String>,
        version: impl Into<String>,
    ) {
        let route_id = route_id.into();
        let latency_ms = latency.as_millis() as u64;

        let event = RequestEvent {
            timestamp: Utc::now().to_rfc3339(),
            route_id: route_id.clone(),
            method: method.into(),
            path: path.into(),
            status,
            latency_ms,
            api_key_id,
            version: version.into(),
        };

        // Update aggregated stats
        {
            let mut stats = self.stats.write().await;
            let entry = stats.entry(route_id).or_default();
            entry.total_requests += 1;
            entry.total_latency_ms += latency_ms;
            if status >= 400 {
                entry.error_count += 1;
            }
        }

        // Append event with ring-buffer eviction
        let mut events = self.events.write().await;
        if events.len() >= self.capacity {
            events.remove(0);
        }
        events.push(event);
    }

    pub async fn stats_snapshot(&self) -> HashMap<String, RouteStats> {
        self.stats.read().await.clone()
    }

    pub async fn recent_events(&self, limit: usize) -> Vec<RequestEvent> {
        let events = self.events.read().await;
        let start = events.len().saturating_sub(limit);
        events[start..].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_and_aggregates() {
        let store = AnalyticsStore::new(100);
        store
            .record(
                "route-1",
                "GET",
                "/api/v1/tx",
                200,
                Duration::from_millis(50),
                None,
                "v1",
            )
            .await;
        store
            .record(
                "route-1",
                "GET",
                "/api/v1/tx",
                500,
                Duration::from_millis(100),
                None,
                "v1",
            )
            .await;
        let stats = store.stats_snapshot().await;
        let s = &stats["route-1"];
        assert_eq!(s.total_requests, 2);
        assert_eq!(s.error_count, 1);
        assert_eq!(s.mean_latency_ms(), 75);
    }
}
