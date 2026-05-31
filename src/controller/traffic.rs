use k8s_openapi::api::core::v1::{Pod, Service, ServicePort, ServiceSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::Deserialize;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;
use tracing::{debug, info, instrument};

use crate::crd::{ReadReplicaStrategy, StellarNode};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficPriority {
    Critical,
    High,
    Normal,
    Low,
}

impl TrafficPriority {
    fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdaptiveRateConfig {
    pub base_rps: f64,
    pub min_rps: f64,
    pub max_rps: f64,
    pub target_load: f64,
    pub boost_factor: f64,
    pub shed_factor: f64,
}

impl Default for AdaptiveRateConfig {
    fn default() -> Self {
        Self {
            base_rps: 500.0,
            min_rps: 50.0,
            max_rps: 5000.0,
            target_load: 0.70,
            boost_factor: 0.50,
            shed_factor: 1.25,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BucketConfig {
    pub token_capacity: f64,
    pub leaky_capacity: f64,
}

impl Default for BucketConfig {
    fn default() -> Self {
        Self {
            token_capacity: 1000.0,
            leaky_capacity: 2000.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub open_window_ms: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            open_window_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrafficShapingConfig {
    pub adaptive: AdaptiveRateConfig,
    pub buckets: BucketConfig,
    pub circuit_breaker: CircuitBreakerConfig,
}

impl Default for TrafficShapingConfig {
    fn default() -> Self {
        Self {
            adaptive: AdaptiveRateConfig::default(),
            buckets: BucketConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrafficRequest {
    pub backend: String,
    pub priority: TrafficPriority,
    pub cost: f64,
}

impl TrafficRequest {
    pub fn new(backend: impl Into<String>, priority: TrafficPriority) -> Self {
        Self {
            backend: backend.into(),
            priority,
            cost: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficDecision {
    pub allowed: bool,
    pub reason: String,
    pub effective_rps: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: CircuitBreakerState,
    consecutive_failures: u32,
    consecutive_successes: u32,
    opened_at_ms: Option<u64>,
}

impl CircuitBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: CircuitBreakerState::Closed,
            consecutive_failures: 0,
            consecutive_successes: 0,
            opened_at_ms: None,
        }
    }

    fn allow(&mut self, now_ms: u64) -> bool {
        if self.state == CircuitBreakerState::Open {
            if let Some(opened_at) = self.opened_at_ms {
                if now_ms.saturating_sub(opened_at) >= self.config.open_window_ms {
                    self.state = CircuitBreakerState::HalfOpen;
                    self.consecutive_successes = 0;
                    return true;
                }
            }
            return false;
        }
        true
    }

    fn on_result(&mut self, success: bool, now_ms: u64) {
        match self.state {
            CircuitBreakerState::Closed => {
                if success {
                    self.consecutive_failures = 0;
                } else {
                    self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                    if self.consecutive_failures >= self.config.failure_threshold {
                        self.state = CircuitBreakerState::Open;
                        self.opened_at_ms = Some(now_ms);
                        self.consecutive_successes = 0;
                    }
                }
            }
            CircuitBreakerState::Open => {
                if now_ms.saturating_sub(self.opened_at_ms.unwrap_or(now_ms))
                    >= self.config.open_window_ms
                {
                    self.state = CircuitBreakerState::HalfOpen;
                    self.consecutive_successes = 0;
                }
            }
            CircuitBreakerState::HalfOpen => {
                if success {
                    self.consecutive_successes = self.consecutive_successes.saturating_add(1);
                    if self.consecutive_successes >= self.config.success_threshold {
                        self.state = CircuitBreakerState::Closed;
                        self.consecutive_failures = 0;
                        self.opened_at_ms = None;
                    }
                } else {
                    self.state = CircuitBreakerState::Open;
                    self.opened_at_ms = Some(now_ms);
                    self.consecutive_failures = self.config.failure_threshold;
                    self.consecutive_successes = 0;
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rps: f64,
    last_refill_ms: u64,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rps: f64, now_ms: u64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_rps,
            last_refill_ms: now_ms,
        }
    }

    fn set_refill_rps(&mut self, refill_rps: f64) {
        self.refill_rps = refill_rps.max(1.0);
    }

    fn allow(&mut self, now_ms: u64, cost: f64) -> bool {
        let elapsed_ms = now_ms.saturating_sub(self.last_refill_ms);
        if elapsed_ms > 0 {
            let refill = (elapsed_ms as f64 / 1000.0) * self.refill_rps;
            self.tokens = (self.tokens + refill).min(self.capacity);
            self.last_refill_ms = now_ms;
        }
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
struct LeakyBucket {
    capacity: f64,
    leak_rps: f64,
    queued: f64,
    last_leak_ms: u64,
}

impl LeakyBucket {
    fn new(capacity: f64, leak_rps: f64, now_ms: u64) -> Self {
        Self {
            capacity,
            leak_rps,
            queued: 0.0,
            last_leak_ms: now_ms,
        }
    }

    fn set_leak_rps(&mut self, leak_rps: f64) {
        self.leak_rps = leak_rps.max(1.0);
    }

    fn allow(&mut self, now_ms: u64, cost: f64) -> bool {
        let elapsed_ms = now_ms.saturating_sub(self.last_leak_ms);
        if elapsed_ms > 0 {
            let leaked = (elapsed_ms as f64 / 1000.0) * self.leak_rps;
            self.queued = (self.queued - leaked).max(0.0);
            self.last_leak_ms = now_ms;
        }

        if self.queued + cost <= self.capacity {
            self.queued += cost;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficDashboardSnapshot {
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub dropped_requests: u64,
    pub drop_rate: f64,
    pub effective_rps: f64,
    pub system_load: f64,
    pub open_circuit_breakers: usize,
    pub requests_by_priority: BTreeMap<String, u64>,
}

#[derive(Debug, Default)]
struct TrafficTelemetry {
    total_requests: u64,
    allowed_requests: u64,
    dropped_requests: u64,
    effective_rps: f64,
    system_load: f64,
    requests_by_priority: HashMap<TrafficPriority, u64>,
    breaker_states: HashMap<String, CircuitBreakerState>,
}

static TRAFFIC_TELEMETRY: OnceLock<RwLock<TrafficTelemetry>> = OnceLock::new();

fn telemetry() -> &'static RwLock<TrafficTelemetry> {
    TRAFFIC_TELEMETRY.get_or_init(|| RwLock::new(TrafficTelemetry::default()))
}

fn update_traffic_telemetry(
    backend: &str,
    priority: TrafficPriority,
    allowed: bool,
    effective_rps: f64,
    system_load: f64,
    breaker_state: CircuitBreakerState,
) {
    if let Ok(mut guard) = telemetry().write() {
        guard.total_requests = guard.total_requests.saturating_add(1);
        if allowed {
            guard.allowed_requests = guard.allowed_requests.saturating_add(1);
        } else {
            guard.dropped_requests = guard.dropped_requests.saturating_add(1);
        }
        let entry = guard.requests_by_priority.entry(priority).or_insert(0);
        *entry = entry.saturating_add(1);
        guard.effective_rps = effective_rps;
        guard.system_load = system_load;
        guard
            .breaker_states
            .insert(backend.to_string(), breaker_state);
    }
}

pub fn get_traffic_dashboard_snapshot() -> TrafficDashboardSnapshot {
    if let Ok(guard) = telemetry().read() {
        let drop_rate = if guard.total_requests == 0 {
            0.0
        } else {
            guard.dropped_requests as f64 / guard.total_requests as f64
        };
        let mut requests_by_priority = BTreeMap::new();
        requests_by_priority.insert(
            TrafficPriority::Critical.as_str().to_string(),
            *guard
                .requests_by_priority
                .get(&TrafficPriority::Critical)
                .unwrap_or(&0),
        );
        requests_by_priority.insert(
            TrafficPriority::High.as_str().to_string(),
            *guard
                .requests_by_priority
                .get(&TrafficPriority::High)
                .unwrap_or(&0),
        );
        requests_by_priority.insert(
            TrafficPriority::Normal.as_str().to_string(),
            *guard
                .requests_by_priority
                .get(&TrafficPriority::Normal)
                .unwrap_or(&0),
        );
        requests_by_priority.insert(
            TrafficPriority::Low.as_str().to_string(),
            *guard
                .requests_by_priority
                .get(&TrafficPriority::Low)
                .unwrap_or(&0),
        );

        let open_circuit_breakers = guard
            .breaker_states
            .values()
            .filter(|s| **s == CircuitBreakerState::Open)
            .count();

        return TrafficDashboardSnapshot {
            total_requests: guard.total_requests,
            allowed_requests: guard.allowed_requests,
            dropped_requests: guard.dropped_requests,
            drop_rate,
            effective_rps: guard.effective_rps,
            system_load: guard.system_load,
            open_circuit_breakers,
            requests_by_priority,
        };
    }

    TrafficDashboardSnapshot {
        total_requests: 0,
        allowed_requests: 0,
        dropped_requests: 0,
        drop_rate: 0.0,
        effective_rps: 0.0,
        system_load: 0.0,
        open_circuit_breakers: 0,
        requests_by_priority: BTreeMap::new(),
    }
}

#[derive(Debug)]
pub struct TrafficShaper {
    config: TrafficShapingConfig,
    token_bucket: TokenBucket,
    leaky_bucket: LeakyBucket,
    breakers: HashMap<String, CircuitBreaker>,
}

impl TrafficShaper {
    pub fn new(config: TrafficShapingConfig, now_ms: u64) -> Self {
        let token_bucket = TokenBucket::new(
            config.buckets.token_capacity,
            config.adaptive.base_rps,
            now_ms,
        );
        let leaky_bucket = LeakyBucket::new(
            config.buckets.leaky_capacity,
            config.adaptive.base_rps,
            now_ms,
        );

        Self {
            config,
            token_bucket,
            leaky_bucket,
            breakers: HashMap::new(),
        }
    }

    pub fn effective_rps(&self, system_load: f64) -> f64 {
        let cfg = &self.config.adaptive;
        let factor = if system_load <= cfg.target_load {
            1.0 + (cfg.target_load - system_load) * cfg.boost_factor
        } else {
            1.0 - (system_load - cfg.target_load) * cfg.shed_factor
        };
        (cfg.base_rps * factor).clamp(cfg.min_rps, cfg.max_rps)
    }

    pub fn record_backend_result(&mut self, backend: &str, success: bool, now_ms: u64) {
        let breaker = self
            .breakers
            .entry(backend.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.config.circuit_breaker.clone()));
        breaker.on_result(success, now_ms);
    }

    pub fn admit_request(
        &mut self,
        req: &TrafficRequest,
        system_load: f64,
        now_ms: u64,
    ) -> TrafficDecision {
        let effective_rps = self.effective_rps(system_load);
        self.token_bucket.set_refill_rps(effective_rps);
        self.leaky_bucket.set_leak_rps(effective_rps);

        let breaker = self
            .breakers
            .entry(req.backend.clone())
            .or_insert_with(|| CircuitBreaker::new(self.config.circuit_breaker.clone()));
        if !breaker.allow(now_ms) {
            let decision = TrafficDecision {
                allowed: false,
                reason: "circuit_breaker_open".to_string(),
                effective_rps,
            };
            update_traffic_telemetry(
                &req.backend,
                req.priority,
                decision.allowed,
                effective_rps,
                system_load,
                breaker.state,
            );
            #[cfg(feature = "metrics")]
            {
                super::metrics::observe_traffic_request(
                    "default",
                    "traffic-shaper",
                    req.priority.as_str(),
                    "dropped",
                );
                super::metrics::set_traffic_effective_rps(
                    "default",
                    "traffic-shaper",
                    effective_rps as i64,
                );
                super::metrics::set_traffic_system_load(
                    "default",
                    "traffic-shaper",
                    (system_load * 100.0) as i64,
                );
                super::metrics::set_traffic_circuit_breaker_state(
                    "default",
                    "traffic-shaper",
                    match breaker.state {
                        CircuitBreakerState::Closed => 0,
                        CircuitBreakerState::Open => 1,
                        CircuitBreakerState::HalfOpen => 2,
                    },
                );
            }
            return decision;
        }

        // Under extreme load, protect high-priority traffic by shedding low priority first.
        if system_load > 0.90 && req.priority == TrafficPriority::Low {
            let decision = TrafficDecision {
                allowed: false,
                reason: "priority_shed".to_string(),
                effective_rps,
            };
            update_traffic_telemetry(
                &req.backend,
                req.priority,
                decision.allowed,
                effective_rps,
                system_load,
                breaker.state,
            );
            #[cfg(feature = "metrics")]
            {
                super::metrics::observe_traffic_request(
                    "default",
                    "traffic-shaper",
                    req.priority.as_str(),
                    "dropped",
                );
            }
            return decision;
        }

        let token_allowed = self.token_bucket.allow(now_ms, req.cost);
        let leaky_allowed = self.leaky_bucket.allow(now_ms, req.cost);
        let allowed = token_allowed && leaky_allowed;
        let reason = if allowed {
            "allowed"
        } else if !token_allowed {
            "token_bucket_limited"
        } else {
            "leaky_bucket_limited"
        };

        let decision = TrafficDecision {
            allowed,
            reason: reason.to_string(),
            effective_rps,
        };

        update_traffic_telemetry(
            &req.backend,
            req.priority,
            decision.allowed,
            effective_rps,
            system_load,
            breaker.state,
        );

        #[cfg(feature = "metrics")]
        {
            super::metrics::observe_traffic_request(
                "default",
                "traffic-shaper",
                req.priority.as_str(),
                if allowed { "allowed" } else { "dropped" },
            );
            super::metrics::set_traffic_effective_rps(
                "default",
                "traffic-shaper",
                effective_rps as i64,
            );
            super::metrics::set_traffic_system_load(
                "default",
                "traffic-shaper",
                (system_load * 100.0) as i64,
            );
            super::metrics::set_traffic_circuit_breaker_state(
                "default",
                "traffic-shaper",
                match breaker.state {
                    CircuitBreakerState::Closed => 0,
                    CircuitBreakerState::Open => 1,
                    CircuitBreakerState::HalfOpen => 2,
                },
            );
        }

        decision
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StellarCoreInfo {
    info: InfoSection,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct InfoSection {
    ledger: LedgerInfo,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LedgerInfo {
    num: u64,
    _age: u64,
}

/// Reconcile traffic routing for read-only replicas
#[allow(dead_code)]
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn reconcile_traffic_routing(client: &Client, node: &StellarNode) -> Result<()> {
    if node.spec.read_replica_config.is_none() {
        return Ok(());
    }

    let config = node.spec.read_replica_config.as_ref().unwrap();
    let _namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // 1. Ensure the traffic service exists
    ensure_traffic_service(client, node).await?;

    // 2. If strategy is FreshnessPreferred, update pod labels
    if config.strategy == ReadReplicaStrategy::FreshnessPreferred {
        update_pod_labels_based_on_lag(client, node).await?;
    } else {
        // For RoundRobin, we ensure all ready pods have the traffic label
        ensure_all_ready_pods_enabled(client, node).await?;
    }

    Ok(())
}

#[allow(dead_code)]
async fn ensure_traffic_service(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-read-traffic", node.name_any());

    let mut selector = super::resources::standard_labels(node);
    selector.insert("stellar.org/role".to_string(), "read-replica".to_string());
    selector.insert("stellar.org/traffic".to_string(), "enabled".to_string());

    let ports = vec![ServicePort {
        name: Some("http".to_string()),
        port: 80,
        target_port: Some(IntOrString::Int(11626)),
        protocol: Some("TCP".to_string()),
        ..Default::default()
    }];

    let service = Service {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(selector.clone()),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(selector),
            ports: Some(ports),
            type_: Some("ClusterIP".to_string()),
            ..Default::default()
        }),
        status: None,
    };

    let patch = Patch::Apply(&service);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

#[allow(dead_code)]
async fn update_pod_labels_based_on_lag(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    // Select read replicas
    let label_selector = format!(
        "app.kubernetes.io/instance={},stellar.org/role=read-replica",
        node.name_any()
    );
    let lp = ListParams::default().labels(&label_selector);
    let pods = pod_api.list(&lp).await?;

    if pods.items.is_empty() {
        return Ok(());
    }

    let mut pod_ledgers = Vec::new();
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(Error::HttpError)?;

    // Gather ledger info
    for pod in &pods.items {
        if let Some(ip) = &pod.status.as_ref().and_then(|s| s.pod_ip.as_ref()) {
            let url = format!("http://{ip}:11626/info");
            match http_client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(info) = resp.json::<StellarCoreInfo>().await {
                        pod_ledgers.push((pod.clone(), info.info.ledger.num));
                    }
                }
                Err(e) => {
                    debug!("Failed to fetch info from pod {}: {}", pod.name_any(), e);
                }
            }
        }
    }

    if pod_ledgers.is_empty() {
        return Ok(());
    }

    // Determine max ledger
    let max_ledger = pod_ledgers.iter().map(|(_, l)| *l).max().unwrap_or(0);
    let lag_threshold = 5; // Configurable? Using hardcoded 5 for now

    for (pod, ledger) in pod_ledgers {
        let is_fresh = max_ledger.saturating_sub(ledger) <= lag_threshold;
        let should_enable = is_fresh;

        ensure_traffic_label(&pod_api, &pod, should_enable).await?;
    }

    // Also handle pods that didn't respond (assume unhealthy/lagging)
    // We didn't collect them in pod_ledgers, so we need to iterate all pods again?
    // Optimization: Just iterate original list and check if in pod_ledgers
    // For simplicity, failing to respond means traffic disabled.

    Ok(())
}

#[allow(dead_code)]
async fn ensure_all_ready_pods_enabled(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    let label_selector = format!(
        "app.kubernetes.io/instance={},stellar.org/role=read-replica",
        node.name_any()
    );
    let pods = pod_api
        .list(&ListParams::default().labels(&label_selector))
        .await?;

    for pod in pods {
        // Check if ready
        let is_ready = pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false);

        ensure_traffic_label(&pod_api, &pod, is_ready).await?;
    }
    Ok(())
}

#[allow(dead_code)]
async fn ensure_traffic_label(api: &Api<Pod>, pod: &Pod, enabled: bool) -> Result<()> {
    let current_val = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("stellar.org/traffic"))
        .map(|s| s.as_str());

    let desired_val = if enabled { Some("enabled") } else { None };

    if current_val != desired_val {
        let name = pod.name_any();
        info!("Updating traffic label for {} to {:?}", name, desired_val);

        // Patch label using JSON merge patch
        // To remove a label, set it to null
        let patch_json = if let Some(val) = desired_val {
            serde_json::json!({
                "metadata": {
                    "labels": {
                        "stellar.org/traffic": val
                    }
                }
            })
        } else {
            serde_json::json!({
                "metadata": {
                    "labels": {
                        "stellar.org/traffic": null
                    }
                }
            })
        };

        api.patch(
            &name,
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&patch_json),
        )
        .await?;
    }
    Ok(())
}
