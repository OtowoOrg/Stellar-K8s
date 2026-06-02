//! Intelligent load balancing based on service health scores

use serde::{Deserialize, Serialize};

use super::registry::ServiceRegistration;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub selected_id: String,
    pub selected_endpoint: String,
    pub selected_port: u16,
    pub reason: String,
    pub health_score: f64,
}

/// Load balancing strategies
#[derive(Clone, Debug)]
pub enum LoadBalancer {
    /// Route to the highest-health-score endpoint
    HealthWeighted,
    /// Round-robin across healthy endpoints
    RoundRobin { counter: std::sync::Arc<std::sync::atomic::AtomicUsize> },
    /// Always prefer the lowest latency endpoint
    LeastLatency,
}

impl LoadBalancer {
    pub fn health_weighted() -> Self {
        Self::HealthWeighted
    }

    pub fn round_robin() -> Self {
        Self::RoundRobin {
            counter: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Select an endpoint from the healthy candidates
    pub fn select<'a>(&self, candidates: &'a [ServiceRegistration]) -> Option<RoutingDecision> {
        let healthy: Vec<&ServiceRegistration> =
            candidates.iter().filter(|s| s.health_score.is_healthy()).collect();

        if healthy.is_empty() {
            return None;
        }

        let chosen = match self {
            Self::HealthWeighted => {
                healthy.iter().max_by(|a, b| {
                    a.health_score.score.partial_cmp(&b.health_score.score).unwrap()
                })?
            }
            Self::RoundRobin { counter } => {
                let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % healthy.len();
                &healthy[idx]
            }
            Self::LeastLatency => {
                healthy.iter().min_by(|a, b| {
                    a.health_score
                        .latency_p95_ms
                        .partial_cmp(&b.health_score.latency_p95_ms)
                        .unwrap()
                })?
            }
        };

        Some(RoutingDecision {
            selected_id: chosen.id.clone(),
            selected_endpoint: chosen.endpoint.clone(),
            selected_port: chosen.port,
            reason: format!("health_score={:.2}", chosen.health_score.score),
            health_score: chosen.health_score.score,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_discovery::{health::HealthScore, registry::{ServiceRegistration, ServiceType}};

    fn make_svc(id: &str, score: f64) -> ServiceRegistration {
        let mut svc = ServiceRegistration::new(id, id, "ns", ServiceType::Horizon, "10.0.0.1", 8000);
        svc.health_score = HealthScore { score, ..Default::default() };
        svc
    }

    #[test]
    fn test_health_weighted_selects_best() {
        let svcs = vec![make_svc("s1", 0.6), make_svc("s2", 0.9), make_svc("s3", 0.7)];
        let lb = LoadBalancer::health_weighted();
        let decision = lb.select(&svcs).unwrap();
        assert_eq!(decision.selected_id, "s2");
    }

    #[test]
    fn test_no_healthy_returns_none() {
        let mut svc = make_svc("s1", 0.1);
        svc.health_score.consecutive_failures = 5;
        let decision = LoadBalancer::health_weighted().select(&[svc]);
        assert!(decision.is_none());
    }
}
