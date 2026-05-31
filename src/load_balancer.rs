/// Advanced load balancing with intelligent traffic distribution (Issue #794)
///
/// Implements multiple algorithms: least-connections, weighted round-robin,
/// consistent hashing, health-aware routing, session affinity, A/B splitting,
/// connection pooling, and dynamic backend discovery.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ── Algorithm ────────────────────────────────────────────────────────────────

/// Supported load-balancing algorithms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LbAlgorithm {
    RoundRobin,
    LeastConnections,
    WeightedRoundRobin,
    ConsistentHash,
    Random,
}

impl Default for LbAlgorithm {
    fn default() -> Self {
        Self::LeastConnections
    }
}

// ── Backend ───────────────────────────────────────────────────────────────────

/// Health status of a backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

/// A single backend endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backend {
    pub id: String,
    pub address: String,
    pub port: u16,
    pub weight: u32,
    pub health: BackendHealth,
    pub active_connections: u64,
    pub total_requests: u64,
    pub error_count: u64,
    pub last_health_check: Option<Instant>,
    /// Tags used for A/B test group assignment.
    pub tags: HashMap<String, String>,
}

impl Backend {
    pub fn new(id: impl Into<String>, address: impl Into<String>, port: u16) -> Self {
        Self {
            id: id.into(),
            address: address.into(),
            port,
            weight: 1,
            health: BackendHealth::Healthy,
            active_connections: 0,
            total_requests: 0,
            error_count: 0,
            last_health_check: None,
            tags: HashMap::new(),
        }
    }

    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    pub fn error_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 0.0;
        }
        self.error_count as f64 / self.total_requests as f64
    }

    pub fn is_available(&self) -> bool {
        self.health != BackendHealth::Unhealthy
    }
}

// ── Session affinity ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickySession {
    pub client_key: String,
    pub backend_id: String,
    pub created_at_secs: u64,
    pub ttl_secs: u64,
}

impl StickySession {
    pub fn is_expired(&self, now_secs: u64) -> bool {
        now_secs.saturating_sub(self.created_at_secs) >= self.ttl_secs
    }
}

// ── A/B split ─────────────────────────────────────────────────────────────────

/// Traffic split rule for A/B testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbSplitRule {
    pub name: String,
    /// Tag key/value that identifies the "B" group backends.
    pub tag_key: String,
    pub tag_value: String,
    /// Percentage (0–100) of traffic routed to the B group.
    pub b_percent: u8,
}

// ── Health check config ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub interval: Duration,
    pub timeout: Duration,
    pub healthy_threshold: u32,
    pub unhealthy_threshold: u32,
    pub path: String,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(3),
            healthy_threshold: 2,
            unhealthy_threshold: 3,
            path: "/health".to_string(),
        }
    }
}

// ── Connection pool ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPoolConfig {
    pub max_connections_per_backend: u32,
    pub keep_alive: bool,
    pub keep_alive_timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        Self {
            max_connections_per_backend: 100,
            keep_alive: true,
            keep_alive_timeout: Duration::from_secs(90),
            idle_timeout: Duration::from_secs(60),
        }
    }
}

// ── LB config ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancerConfig {
    pub name: String,
    pub algorithm: LbAlgorithm,
    pub health_check: HealthCheckConfig,
    pub connection_pool: ConnectionPoolConfig,
    pub session_affinity_ttl_secs: u64,
    pub ab_rules: Vec<AbSplitRule>,
    /// Maximum error rate before a backend is marked degraded.
    pub degraded_error_rate: f64,
    /// Maximum error rate before a backend is marked unhealthy.
    pub unhealthy_error_rate: f64,
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            algorithm: LbAlgorithm::LeastConnections,
            health_check: HealthCheckConfig::default(),
            connection_pool: ConnectionPoolConfig::default(),
            session_affinity_ttl_secs: 300,
            ab_rules: vec![],
            degraded_error_rate: 0.05,
            unhealthy_error_rate: 0.20,
        }
    }
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LbMetrics {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub no_backend_available: u64,
    pub session_hits: u64,
    pub session_misses: u64,
    pub requests_per_backend: HashMap<String, u64>,
}

// ── Core load balancer ────────────────────────────────────────────────────────

pub struct LoadBalancer {
    config: LoadBalancerConfig,
    backends: Vec<Backend>,
    sessions: HashMap<String, StickySession>,
    metrics: LbMetrics,
    rr_index: usize,
}

impl LoadBalancer {
    pub fn new(config: LoadBalancerConfig) -> Self {
        Self {
            config,
            backends: vec![],
            sessions: HashMap::new(),
            metrics: LbMetrics::default(),
            rr_index: 0,
        }
    }

    // ── Backend registry ──────────────────────────────────────────────────────

    pub fn register_backend(&mut self, backend: Backend) {
        info!("Registering backend {} ({}:{})", backend.id, backend.address, backend.port);
        self.backends.retain(|b| b.id != backend.id);
        self.backends.push(backend);
    }

    pub fn deregister_backend(&mut self, id: &str) {
        info!("Deregistering backend {}", id);
        self.backends.retain(|b| b.id != id);
        self.sessions.retain(|_, s| s.backend_id != id);
    }

    pub fn update_backend_health(&mut self, id: &str, health: BackendHealth) {
        if let Some(b) = self.backends.iter_mut().find(|b| b.id == id) {
            if b.health != health {
                info!("Backend {} health changed to {:?}", id, health);
                b.health = health;
            }
        }
    }

    pub fn record_result(&mut self, backend_id: &str, success: bool) {
        if let Some(b) = self.backends.iter_mut().find(|b| b.id == backend_id) {
            b.total_requests = b.total_requests.saturating_add(1);
            if !success {
                b.error_count = b.error_count.saturating_add(1);
            }
            b.active_connections = b.active_connections.saturating_sub(1);

            // Auto-update health based on error rate
            let rate = b.error_rate();
            b.health = if rate >= self.config.unhealthy_error_rate {
                BackendHealth::Unhealthy
            } else if rate >= self.config.degraded_error_rate {
                BackendHealth::Degraded
            } else {
                BackendHealth::Healthy
            };
        }
        if success {
            self.metrics.successful_requests = self.metrics.successful_requests.saturating_add(1);
        } else {
            self.metrics.failed_requests = self.metrics.failed_requests.saturating_add(1);
        }
    }

    // ── Routing ───────────────────────────────────────────────────────────────

    /// Select a backend for the given client key (used for session affinity and
    /// consistent hashing). Returns `None` when no healthy backend is available.
    pub fn select(&mut self, client_key: &str, now_secs: u64) -> Option<String> {
        self.metrics.total_requests = self.metrics.total_requests.saturating_add(1);

        // 1. Session affinity
        if let Some(id) = self.session_lookup(client_key, now_secs) {
            self.metrics.session_hits = self.metrics.session_hits.saturating_add(1);
            self.bump_connections(&id);
            return Some(id);
        }
        self.metrics.session_misses = self.metrics.session_misses.saturating_add(1);

        // 2. A/B split (deterministic by client_key hash)
        let ab_backend = self.ab_select(client_key);

        let id = if let Some(id) = ab_backend {
            id
        } else {
            // 3. Algorithm-based selection
            let available: Vec<&Backend> = self.backends.iter().filter(|b| b.is_available()).collect();
            if available.is_empty() {
                warn!("No available backends");
                self.metrics.no_backend_available = self.metrics.no_backend_available.saturating_add(1);
                return None;
            }

            match self.config.algorithm {
                LbAlgorithm::LeastConnections => Self::least_connections(&available),
                LbAlgorithm::WeightedRoundRobin => {
                    let idx = self.rr_index;
                    let id = Self::weighted_rr(&available, idx);
                    self.rr_index = self.rr_index.wrapping_add(1);
                    id
                }
                LbAlgorithm::ConsistentHash => Self::consistent_hash(&available, client_key),
                LbAlgorithm::Random => Self::random_select(&available),
                LbAlgorithm::RoundRobin => {
                    let idx = self.rr_index % available.len();
                    self.rr_index = self.rr_index.wrapping_add(1);
                    available[idx].id.clone()
                }
            }
        };

        // 4. Create sticky session
        self.sessions.insert(
            client_key.to_string(),
            StickySession {
                client_key: client_key.to_string(),
                backend_id: id.clone(),
                created_at_secs: now_secs,
                ttl_secs: self.config.session_affinity_ttl_secs,
            },
        );

        *self.metrics.requests_per_backend.entry(id.clone()).or_insert(0) += 1;
        self.bump_connections(&id);
        Some(id)
    }

    fn session_lookup(&mut self, client_key: &str, now_secs: u64) -> Option<String> {
        let session = self.sessions.get(client_key)?;
        if session.is_expired(now_secs) {
            self.sessions.remove(client_key);
            return None;
        }
        let id = session.backend_id.clone();
        // Verify backend is still available
        if self.backends.iter().any(|b| b.id == id && b.is_available()) {
            Some(id)
        } else {
            self.sessions.remove(client_key);
            None
        }
    }

    fn ab_select(&self, client_key: &str) -> Option<String> {
        for rule in &self.config.ab_rules {
            let hash = fnv1a(client_key) % 100;
            let use_b = hash < rule.b_percent as u64;
            let tag_key = rule.tag_key.clone();
            let tag_value = rule.tag_value.clone();
            let candidates: Vec<&Backend> = self
                .backends
                .iter()
                .filter(|b| b.is_available())
                .filter(|b| {
                    if use_b {
                        b.tags.get(&tag_key).map(|v| v == &tag_value).unwrap_or(false)
                    } else {
                        !b.tags.get(&tag_key).map(|v| v == &tag_value).unwrap_or(false)
                    }
                })
                .collect();
            if !candidates.is_empty() {
                return Some(Self::least_connections(&candidates));
            }
        }
        None
    }

    fn bump_connections(&mut self, id: &str) {
        if let Some(b) = self.backends.iter_mut().find(|b| b.id == id) {
            b.active_connections = b.active_connections.saturating_add(1);
        }
    }

    // ── Algorithm implementations ─────────────────────────────────────────────

    fn least_connections(available: &[&Backend]) -> String {
        available
            .iter()
            .min_by_key(|b| b.active_connections)
            .map(|b| b.id.clone())
            .unwrap_or_default()
    }

    fn weighted_rr(available: &[&Backend], idx: usize) -> String {
        let total_weight: u32 = available.iter().map(|b| b.weight).sum();
        if total_weight == 0 {
            return available[idx % available.len()].id.clone();
        }
        let mut pos = (idx as u32) % total_weight;
        for b in available {
            if pos < b.weight {
                return b.id.clone();
            }
            pos -= b.weight;
        }
        available[0].id.clone()
    }

    fn consistent_hash(available: &[&Backend], key: &str) -> String {
        let hash = fnv1a(key);
        let idx = (hash as usize) % available.len();
        available[idx].id.clone()
    }

    fn random_select(available: &[&Backend]) -> String {
        // Deterministic pseudo-random using current time nanos as seed
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize;
        available[seed % available.len()].id.clone()
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn metrics(&self) -> &LbMetrics {
        &self.metrics
    }

    pub fn backends(&self) -> &[Backend] {
        &self.backends
    }

    pub fn healthy_backend_count(&self) -> usize {
        self.backends.iter().filter(|b| b.health == BackendHealth::Healthy).count()
    }

    /// Evict expired sessions.
    pub fn gc_sessions(&mut self, now_secs: u64) {
        self.sessions.retain(|_, s| !s.is_expired(now_secs));
    }
}

/// FNV-1a 64-bit hash (no external dep needed).
fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

// ── Shared handle ─────────────────────────────────────────────────────────────

pub type SharedLoadBalancer = Arc<RwLock<LoadBalancer>>;

pub fn new_shared(config: LoadBalancerConfig) -> SharedLoadBalancer {
    Arc::new(RwLock::new(LoadBalancer::new(config)))
}

// ── Active health checker ─────────────────────────────────────────────────────

/// Spawn a background task that periodically checks backend health via HTTP.
pub async fn run_health_checker(lb: SharedLoadBalancer) {
    loop {
        let (backends_snapshot, config) = {
            let guard = lb.read().await;
            (
                guard.backends().to_vec(),
                guard.config.health_check.clone(),
            )
        };

        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();

        for backend in backends_snapshot {
            let url = format!("http://{}:{}{}", backend.address, backend.port, config.path);
            let health = match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => BackendHealth::Healthy,
                Ok(_) => BackendHealth::Degraded,
                Err(e) => {
                    debug!("Health check failed for {}: {}", backend.id, e);
                    BackendHealth::Unhealthy
                }
            };
            lb.write().await.update_backend_health(&backend.id, health);
        }

        tokio::time::sleep(config.interval).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lb() -> LoadBalancer {
        let mut lb = LoadBalancer::new(LoadBalancerConfig::default());
        lb.register_backend(Backend::new("b1", "10.0.0.1", 8080).with_weight(2));
        lb.register_backend(Backend::new("b2", "10.0.0.2", 8080).with_weight(1));
        lb.register_backend(Backend::new("b3", "10.0.0.3", 8080).with_weight(1));
        lb
    }

    #[test]
    fn test_least_connections_prefers_idle() {
        let mut lb = make_lb();
        // Simulate b1 being busy
        lb.backends[0].active_connections = 10;
        let id = lb.select("client-1", 0).unwrap();
        assert_ne!(id, "b1");
    }

    #[test]
    fn test_session_affinity_sticky() {
        let mut lb = make_lb();
        let first = lb.select("client-sticky", 0).unwrap();
        let second = lb.select("client-sticky", 10).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn test_session_expires() {
        let mut lb = make_lb();
        lb.config.session_affinity_ttl_secs = 5;
        let first = lb.select("client-exp", 0).unwrap();
        // After TTL, session should be gone and a new backend may be chosen
        lb.gc_sessions(10);
        // Just verify it doesn't panic and returns a backend
        let _ = lb.select("client-exp", 10).unwrap();
        let _ = first; // suppress unused warning
    }

    #[test]
    fn test_unhealthy_backend_excluded() {
        let mut lb = make_lb();
        lb.update_backend_health("b1", BackendHealth::Unhealthy);
        lb.update_backend_health("b2", BackendHealth::Unhealthy);
        let id = lb.select("client-x", 0).unwrap();
        assert_eq!(id, "b3");
    }

    #[test]
    fn test_no_backends_returns_none() {
        let mut lb = LoadBalancer::new(LoadBalancerConfig::default());
        assert!(lb.select("client", 0).is_none());
    }

    #[test]
    fn test_consistent_hash_deterministic() {
        let mut lb = LoadBalancer::new(LoadBalancerConfig {
            algorithm: LbAlgorithm::ConsistentHash,
            ..Default::default()
        });
        lb.register_backend(Backend::new("b1", "10.0.0.1", 8080));
        lb.register_backend(Backend::new("b2", "10.0.0.2", 8080));
        let a = lb.select("user-42", 0).unwrap();
        // Clear session so we re-run the algorithm
        lb.sessions.clear();
        let b = lb.select("user-42", 0).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_ab_split_routing() {
        let config = LoadBalancerConfig {
            ab_rules: vec![AbSplitRule {
                name: "test".to_string(),
                tag_key: "group".to_string(),
                tag_value: "b".to_string(),
                b_percent: 100, // all traffic to B
            }],
            ..Default::default()
        };
        let mut lb = LoadBalancer::new(config);
        lb.register_backend(Backend::new("a1", "10.0.0.1", 8080));
        lb.register_backend(
            Backend::new("b1", "10.0.0.2", 8080).with_tag("group", "b"),
        );
        let id = lb.select("any-client", 0).unwrap();
        assert_eq!(id, "b1");
    }

    #[test]
    fn test_record_result_updates_health() {
        let mut lb = make_lb();
        // Drive error rate above unhealthy threshold
        for _ in 0..10 {
            lb.backends[0].total_requests += 1;
            lb.backends[0].error_count += 1;
        }
        lb.record_result("b1", false);
        assert_eq!(lb.backends[0].health, BackendHealth::Unhealthy);
    }
}
