//! Service version management and canary routing support

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Canary routing configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanaryConfig {
    pub service_name: String,
    pub stable_version: String,
    pub canary_version: String,
    /// Percentage of traffic sent to canary (0–100)
    pub canary_weight_pct: u8,
    pub enabled: bool,
}

impl CanaryConfig {
    pub fn new(service: &str, stable: &str, canary: &str, weight: u8) -> Self {
        Self {
            service_name: service.into(),
            stable_version: stable.into(),
            canary_version: canary.into(),
            canary_weight_pct: weight.min(100),
            enabled: true,
        }
    }

    /// Decide whether to route to canary based on a request identifier hash
    pub fn should_route_to_canary(&self, request_id: &str) -> bool {
        if !self.enabled || self.canary_weight_pct == 0 {
            return false;
        }
        let hash = request_id.bytes().fold(0u64, |acc, b| acc.wrapping_add(b as u64));
        (hash % 100) < self.canary_weight_pct as u64
    }
}

/// Manages versions and canary deployments across services
pub struct VersionManager {
    canary_configs: HashMap<String, CanaryConfig>,
}

impl VersionManager {
    pub fn new() -> Self {
        Self { canary_configs: HashMap::new() }
    }

    pub fn add_canary(&mut self, config: CanaryConfig) {
        self.canary_configs.insert(config.service_name.clone(), config);
    }

    pub fn remove_canary(&mut self, service: &str) {
        self.canary_configs.remove(service);
    }

    pub fn route_version<'a>(&self, service: &str, request_id: &str) -> &'a str {
        if let Some(cfg) = self.canary_configs.get(service) {
            if cfg.should_route_to_canary(request_id) {
                return "canary";
            }
        }
        "stable"
    }

    pub fn canary_configs(&self) -> &HashMap<String, CanaryConfig> {
        &self.canary_configs
    }
}

impl Default for VersionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canary_routing_respects_weight() {
        let cfg = CanaryConfig::new("horizon", "v1.0", "v1.1", 50);
        let mut canary_hits = 0;
        for i in 0..1000 {
            if cfg.should_route_to_canary(&format!("req_{i}")) {
                canary_hits += 1;
            }
        }
        // Expect roughly 50% ± 10%
        assert!(canary_hits > 400 && canary_hits < 600, "canary_hits={canary_hits}");
    }

    #[test]
    fn test_zero_weight_never_routes_to_canary() {
        let cfg = CanaryConfig::new("horizon", "v1.0", "v1.1", 0);
        for i in 0..100 {
            assert!(!cfg.should_route_to_canary(&format!("req_{i}")));
        }
    }
}
