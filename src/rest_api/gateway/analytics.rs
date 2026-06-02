//! API Analytics and Usage Tracking
//!
//! Provides comprehensive API analytics including:
//! - Request/response logging
//! - Latency metrics
//! - Error tracking
//! - Usage patterns
//! - Client statistics
//! - API health monitoring

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

/// Individual API call record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCall {
    pub timestamp: DateTime<Utc>,
    pub path: String,
    pub method: String,
    pub status: u16,
    pub latency_ms: u64,
    pub client_id: String,
    pub client_ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Aggregated metrics for a time window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindowMetrics {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_latency_ms: u64,
    pub avg_latency_ms: f64,
    pub p50_latency_ms: u64,
    pub p90_latency_ms: u64,
    pub p99_latency_ms: u64,
    pub requests_per_second: f64,
    pub error_rate: f64,
    pub status_codes: HashMap<u16, u64>,
    pub top_paths: Vec<PathMetrics>,
    pub top_clients: Vec<ClientMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMetrics {
    pub path: String,
    pub requests: u64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMetrics {
    pub client_id: String,
    pub requests: u64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
}

/// Client usage summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientUsage {
    pub client_id: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub total_requests: u64,
    pub total_errors: u64,
    pub avg_latency_ms: f64,
    pub quota_usage: Option<QuotaUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaUsage {
    pub requests_used: u64,
    pub requests_limit: u64,
    pub bandwidth_used_mb: u64,
    pub bandwidth_limit_mb: u64,
    pub percent_used: f64,
}

/// API health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiHealth {
    pub status: HealthStatus,
    pub uptime_seconds: u64,
    pub total_requests: u64,
    pub requests_per_second: f64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
    pub last_error: Option<String>,
    pub last_error_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Analytics collector
pub struct Analytics {
    calls: VecDeque<ApiCall>,
    client_stats: HashMap<String, ClientStats>,
    path_stats: HashMap<String, PathStats>,
    start_time: DateTime<Utc>,
    max_calls: usize,
    total_calls: u64,
    total_errors: u64,
}

#[derive(Debug)]
struct ClientStats {
    client_id: String,
    requests: u64,
    errors: u64,
    total_latency_ms: u64,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

#[derive(Debug)]
struct PathStats {
    path: String,
    requests: u64,
    errors: u64,
    total_latency_ms: u64,
    status_codes: HashMap<u16, u64>,
}

impl Analytics {
    pub fn new() -> Self {
        Self {
            calls: VecDeque::new(),
            client_stats: HashMap::new(),
            path_stats: HashMap::new(),
            start_time: Utc::now(),
            max_calls: 10000,
            total_calls: 0,
            total_errors: 0,
        }
    }

    pub fn with_max_calls(max_calls: usize) -> Self {
        let mut analytics = Self::new();
        analytics.max_calls = max_calls;
        analytics
    }

    /// Record an API call
    pub fn record_call(&mut self, call: ApiCall) {
        self.total_calls += 1;
        
        if call.status >= 400 || call.status == 0 {
            self.total_errors += 1;
        }

        // Add to call history
        if self.calls.len() >= self.max_calls {
            self.calls.pop_front();
        }
        self.calls.push_back(call.clone());

        // Update client stats
        let client = self.client_stats.entry(call.client_id.clone()).or_insert_with(|| ClientStats {
            client_id: call.client_id.clone(),
            requests: 0,
            errors: 0,
            total_latency_ms: 0,
            first_seen: call.timestamp,
            last_seen: call.timestamp,
        });
        client.requests += 1;
        client.total_latency_ms += call.latency_ms;
        if call.status >= 400 {
            client.errors += 1;
        }
        client.last_seen = call.timestamp;

        // Update path stats
        let path = self.path_stats.entry(call.path.clone()).or_insert_with(|| PathStats {
            path: call.path.clone(),
            requests: 0,
            errors: 0,
            total_latency_ms: 0,
            status_codes: HashMap::new(),
        });
        path.requests += 1;
        path.total_latency_ms += call.latency_ms;
        if call.status >= 400 {
            path.errors += 1;
        }
        *path.status_codes.entry(call.status).or_insert(0) += 1;
    }

    /// Get metrics for a time window
    pub fn get_window_metrics(&self, window: Duration) -> TimeWindowMetrics {
        let now = Utc::now();
        let window_start = now - ChronoDuration::from_std(window).unwrap_or(ChronoDuration::days(1));
        
        let mut calls_in_window: Vec<&ApiCall> = self.calls.iter()
            .filter(|c| c.timestamp >= window_start)
            .collect();

        let total_requests = calls_in_window.len() as u64;
        let successful_requests = calls_in_window.iter()
            .filter(|c| c.status < 400)
            .count() as u64;
        let failed_requests = total_requests - successful_requests;

        let total_latency: u64 = calls_in_window.iter()
            .map(|c| c.latency_ms)
            .sum();
        
        let avg_latency = if total_requests > 0 {
            total_latency as f64 / total_requests as f64
        } else {
            0.0
        };

        // Calculate percentiles
        let mut latencies: Vec<u64> = calls_in_window.iter()
            .map(|c| c.latency_ms)
            .collect();
        latencies.sort();
        
        let p50 = percentile(&latencies, 50);
        let p90 = percentile(&latencies, 90);
        let p99 = percentile(&latencies, 99);

        // Status code distribution
        let mut status_codes = HashMap::new();
        for call in &calls_in_window {
            *status_codes.entry(call.status).or_insert(0) += 1;
        }

        // Top paths
        let mut path_map: HashMap<String, (u64, u64, u64)> = HashMap::new();
        for call in &calls_in_window {
            let entry = path_map.entry(call.path.clone()).or_insert((0, 0, 0));
            entry.0 += 1;
            entry.1 += call.latency_ms;
            if call.status >= 400 {
                entry.2 += 1;
            }
        }
        let mut top_paths: Vec<PathMetrics> = path_map.into_iter()
            .map(|(path, (requests, latency, errors))| PathMetrics {
                path,
                requests,
                avg_latency_ms: if requests > 0 { latency as f64 / requests as f64 } else { 0.0 },
                error_rate: if requests > 0 { errors as f64 / requests as f64 } else { 0.0 },
            })
            .collect();
        top_paths.sort_by(|a, b| b.requests.cmp(&a.requests));
        top_paths.truncate(10);

        // Top clients
        let mut client_map: HashMap<String, (u64, u64, u64)> = HashMap::new();
        for call in &calls_in_window {
            let entry = client_map.entry(call.client_id.clone()).or_insert((0, 0, 0));
            entry.0 += 1;
            entry.1 += call.latency_ms;
            if call.status >= 400 {
                entry.2 += 1;
            }
        }
        let mut top_clients: Vec<ClientMetrics> = client_map.into_iter()
            .map(|(client_id, (requests, latency, errors))| ClientMetrics {
                client_id,
                requests,
                avg_latency_ms: if requests > 0 { latency as f64 / requests as f64 } else { 0.0 },
                error_rate: if requests > 0 { errors as f64 / requests as f64 } else { 0.0 },
            })
            .collect();
        top_clients.sort_by(|a, b| b.requests.cmp(&a.requests));
        top_clients.truncate(10);

        let window_secs = window.as_secs() as f64;
        let requests_per_second = if window_secs > 0.0 {
            total_requests as f64 / window_secs
        } else {
            0.0
        };

        TimeWindowMetrics {
            window_start,
            window_end: now,
            total_requests,
            successful_requests,
            failed_requests,
            total_latency_ms: total_latency,
            avg_latency_ms: avg_latency,
            p50_latency_ms: p50,
            p90_latency_ms: p90,
            p99_latency_ms: p99,
            requests_per_second,
            error_rate: if total_requests > 0 {
                failed_requests as f64 / total_requests as f64
            } else {
                0.0
            },
            status_codes,
            top_paths,
            top_clients,
        }
    }

    /// Get client usage statistics
    pub fn get_client_usage(&self) -> Vec<ClientUsage> {
        self.client_stats.values()
            .map(|stats| ClientUsage {
                client_id: stats.client_id.clone(),
                first_seen: stats.first_seen,
                last_seen: stats.last_seen,
                total_requests: stats.requests,
                total_errors: stats.errors,
                avg_latency_ms: if stats.requests > 0 {
                    stats.total_latency_ms as f64 / stats.requests as f64
                } else {
                    0.0
                },
                quota_usage: None,
            })
            .collect()
    }

    /// Get API health status
    pub fn get_health(&self) -> ApiHealth {
        let window = Duration::from_secs(60);
        let metrics = self.get_window_metrics(window);
        
        let status = if metrics.error_rate < 0.01 && metrics.avg_latency_ms < 1000.0 {
            HealthStatus::Healthy
        } else if metrics.error_rate < 0.05 && metrics.avg_latency_ms < 3000.0 {
            HealthStatus::Degraded
        } else {
            HealthStatus::Unhealthy
        };

        let uptime = Utc::now() - self.start_time;
        
        ApiHealth {
            status,
            uptime_seconds: uptime.num_seconds() as u64,
            total_requests: self.total_calls,
            requests_per_second: metrics.requests_per_second,
            avg_latency_ms: metrics.avg_latency_ms,
            error_rate: metrics.error_rate,
            last_error: None,
            last_error_time: None,
        }
    }

    /// Get recent calls
    pub fn get_recent_calls(&self, limit: usize) -> Vec<ApiCall> {
        self.calls.iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get total calls count
    pub fn total_calls(&self) -> u64 {
        self.total_calls
    }

    /// Get total errors count
    pub fn total_errors(&self) -> u64 {
        self.total_errors
    }
}

impl Default for Analytics {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate percentile from sorted array
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = (sorted.len() * p / 100).min(sorted.len() - 1);
    sorted[index]
}

/// Gateway metrics for prometheus export
pub struct GatewayMetrics {
    pub total_requests: prometheus_client::metrics::counter::Counter,
    pub total_errors: prometheus_client::metrics::counter::Counter,
    pub requests_by_method: prometheus_client::metrics::family::Family<
        prometheus_client::metrics::counter::Counter,
        prometheus_client::metrics::family::MetricLabel,
    >,
    pub requests_by_status: prometheus_client::metrics::family::Family<
        prometheus_client::metrics::counter::Counter,
        prometheus_client::metrics::family::MetricLabel,
    >,
    pub requests_by_path: prometheus_client::metrics::family::Family<
        prometheus_client::metrics::counter::Counter,
        prometheus_client::metrics::family::MetricLabel,
    >,
    pub latency_histogram: prometheus_client::metrics::histogram::Histogram,
}

impl GatewayMetrics {
    pub fn new() -> Self {
        Self {
            total_requests: prometheus_client::metrics::counter::Counter::new(
                "gateway_total_requests",
                "Total number of requests processed by the gateway",
            ),
            total_errors: prometheus_client::metrics::counter::Counter::new(
                "gateway_total_errors",
                "Total number of errors processed by the gateway",
            ),
            requests_by_method: prometheus_client::metrics::family::Family::new(
                |labels| prometheus_client::metrics::counter::Counter::new(
                    "gateway_requests_total",
                    "Total requests by HTTP method",
                ),
            ),
            requests_by_status: prometheus_client::metrics::family::Family::new(
                |labels| prometheus_client::metrics::counter::Counter::new(
                    "gateway_responses_total",
                    "Total responses by status code",
                ),
            ),
            requests_by_path: prometheus_client::metrics::family::Family::new(
                |labels| prometheus_client::metrics::counter::Counter::new(
                    "gateway_path_requests_total",
                    "Total requests by path",
                ),
            ),
            latency_histogram: prometheus_client::metrics::histogram::Histogram::new(
                "gateway_request_duration_seconds",
                "Request duration in seconds",
                vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
            ),
        }
    }

    /// Record a request
    pub fn record_request(&self, call: &ApiCall) {
        self.total_requests.inc();
        
        let method_label = prometheus_client::metrics::family::MetricLabel::new(
            "method", call.method.clone()
        );
        self.requests_by_method.get_or_create(&[method_label]).inc();

        let status_label = prometheus_client::metrics::family::MetricLabel::new(
            "status", call.status.to_string()
        );
        self.requests_by_status.get_or_create(&[status_label]).inc();

        if call.status >= 400 {
            self.total_errors.inc();
        }

        // Record latency (convert ms to seconds)
        let latency_sec = call.latency_ms as f64 / 1000.0;
        self.latency_histogram.observe(latency_sec);
    }
}

impl Default for GatewayMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile() {
        let sorted = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(percentile(&sorted, 50), 5);
        assert_eq!(percentile(&sorted, 90), 9);
        assert_eq!(percentile(&sorted, 99), 10);
    }

    #[test]
    fn test_analytics_record() {
        let mut analytics = Analytics::new();
        
        analytics.record_call(ApiCall {
            timestamp: Utc::now(),
            path: "/api/users".to_string(),
            method: "GET".to_string(),
            status: 200,
            latency_ms: 100,
            client_id: "test-client".to_string(),
            client_ip: "127.0.0.1".to_string(),
            user_agent: None,
            request_id: None,
            error_message: None,
        });

        assert_eq!(analytics.total_calls(), 1);
        assert_eq!(analytics.total_errors(), 0);
    }

    #[test]
    fn test_analytics_errors() {
        let mut analytics = Analytics::new();
        
        analytics.record_call(ApiCall {
            timestamp: Utc::now(),
            path: "/api/users".to_string(),
            method: "GET".to_string(),
            status: 500,
            latency_ms: 100,
            client_id: "test-client".to_string(),
            client_ip: "127.0.0.1".to_string(),
            user_agent: None,
            request_id: None,
            error_message: Some("Internal error".to_string()),
        });

        assert_eq!(analytics.total_calls(), 1);
        assert_eq!(analytics.total_errors(), 1);
    }
}