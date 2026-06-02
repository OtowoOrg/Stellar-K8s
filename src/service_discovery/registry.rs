//! Service registry with health tracking per StellarNode service

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::health::HealthScore;

/// A registered service endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceRegistration {
    pub id: String,
    pub name: String,
    pub namespace: String,
    pub service_type: ServiceType,
    pub endpoint: String,
    pub port: u16,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub version: String,
    pub health_score: HealthScore,
    pub registered_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ServiceType {
    StellarCore,
    Horizon,
    SorobanRpc,
    Operator,
    Sidecar,
    Unknown,
}

impl ServiceRegistration {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        namespace: impl Into<String>,
        service_type: ServiceType,
        endpoint: impl Into<String>,
        port: u16,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            name: name.into(),
            namespace: namespace.into(),
            service_type,
            endpoint: endpoint.into(),
            port,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            version: "unknown".into(),
            health_score: HealthScore::default(),
            registered_at: now,
            last_seen: now,
            dependencies: Vec::new(),
        }
    }

    pub fn is_stale(&self, stale_after_secs: i64) -> bool {
        let age = Utc::now().signed_duration_since(self.last_seen).num_seconds();
        age > stale_after_secs
    }
}

/// Service registry — the source of truth for discovered services
pub struct ServiceRegistry {
    services: Arc<RwLock<HashMap<String, ServiceRegistration>>>,
    stale_threshold_secs: i64,
}

impl ServiceRegistry {
    pub fn new(stale_threshold_secs: i64) -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            stale_threshold_secs,
        }
    }

    pub async fn register(&self, svc: ServiceRegistration) {
        info!(id = %svc.id, name = %svc.name, ns = %svc.namespace, "Service registered");
        self.services.write().await.insert(svc.id.clone(), svc);
    }

    pub async fn deregister(&self, id: &str) {
        if self.services.write().await.remove(id).is_some() {
            info!(id, "Service deregistered");
        }
    }

    pub async fn heartbeat(&self, id: &str, score: HealthScore) {
        let mut svcs = self.services.write().await;
        if let Some(svc) = svcs.get_mut(id) {
            svc.last_seen = Utc::now();
            svc.health_score = score;
            debug!(id, "Heartbeat received");
        } else {
            warn!(id, "Heartbeat for unknown service");
        }
    }

    pub async fn get(&self, id: &str) -> Option<ServiceRegistration> {
        self.services.read().await.get(id).cloned()
    }

    pub async fn list_healthy(&self) -> Vec<ServiceRegistration> {
        self.services
            .read()
            .await
            .values()
            .filter(|s| s.health_score.is_healthy() && !s.is_stale(self.stale_threshold_secs))
            .cloned()
            .collect()
    }

    pub async fn prune_stale(&self) -> usize {
        let mut svcs = self.services.write().await;
        let before = svcs.len();
        svcs.retain(|_, s| !s.is_stale(self.stale_threshold_secs));
        let pruned = before - svcs.len();
        if pruned > 0 {
            warn!(pruned, "Pruned stale service registrations");
        }
        pruned
    }

    pub async fn all(&self) -> Vec<ServiceRegistration> {
        self.services.read().await.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_list() {
        let reg = ServiceRegistry::new(300);
        let svc = ServiceRegistration::new("id1", "horizon-0", "default", ServiceType::Horizon, "10.0.0.1", 8000);
        reg.register(svc).await;
        let healthy = reg.list_healthy().await;
        assert_eq!(healthy.len(), 1);
    }

    #[tokio::test]
    async fn test_deregister() {
        let reg = ServiceRegistry::new(300);
        let svc = ServiceRegistration::new("id1", "core-0", "default", ServiceType::StellarCore, "10.0.0.2", 11626);
        reg.register(svc).await;
        reg.deregister("id1").await;
        assert!(reg.get("id1").await.is_none());
    }
}
