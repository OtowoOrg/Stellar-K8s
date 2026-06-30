//! Network security monitoring and alerting.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::flow::NetworkFlow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAlert {
    pub alert_type: String,
    pub severity: String,
    pub source_ip: String,
    pub destination_ip: Option<String>,
    pub description: String,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub name: String,
    pub allowed_ports: Vec<u16>,
    pub allowed_namespaces: Vec<String>,
    pub deny_external_egress: bool,
}

pub struct SecurityMonitor {
    policies: Vec<NetworkPolicy>,
    /// Known internal IP prefixes
    internal_prefixes: Vec<String>,
}

impl SecurityMonitor {
    pub fn new(policies: Vec<NetworkPolicy>) -> Self {
        Self {
            policies,
            internal_prefixes: vec![
                "10.".to_string(),
                "172.16.".to_string(),
                "192.168.".to_string(),
            ],
        }
    }

    fn is_internal(&self, ip: &str) -> bool {
        self.internal_prefixes
            .iter()
            .any(|p| ip.starts_with(p.as_str()))
    }

    pub fn check_policy_violations(&self, flows: &[NetworkFlow]) -> Vec<SecurityAlert> {
        let mut alerts = Vec::new();
        for flow in flows {
            for policy in &self.policies {
                // Check denied external egress
                if policy.deny_external_egress && !self.is_internal(&flow.dst_ip) {
                    alerts.push(SecurityAlert {
                        alert_type: "PolicyViolation".to_string(),
                        severity: "high".to_string(),
                        source_ip: flow.src_ip.clone(),
                        destination_ip: Some(flow.dst_ip.clone()),
                        description: format!(
                            "Policy '{}': external egress denied for {} -> {}",
                            policy.name, flow.src_ip, flow.dst_ip
                        ),
                        detected_at: Utc::now(),
                    });
                }

                // Check disallowed ports
                if !policy.allowed_ports.is_empty()
                    && !policy.allowed_ports.contains(&flow.dst_port)
                {
                    alerts.push(SecurityAlert {
                        alert_type: "UnauthorizedPort".to_string(),
                        severity: "medium".to_string(),
                        source_ip: flow.src_ip.clone(),
                        destination_ip: Some(flow.dst_ip.clone()),
                        description: format!(
                            "Policy '{}': port {} not in allowed list",
                            policy.name, flow.dst_port
                        ),
                        detected_at: Utc::now(),
                    });
                }
            }
        }
        alerts
    }

    /// Detect lateral movement: internal-to-internal flows hitting many distinct pods.
    pub fn detect_lateral_movement(&self, flows: &[NetworkFlow]) -> Vec<SecurityAlert> {
        let mut src_destinations: HashMap<&str, HashSet<&str>> = HashMap::new();
        for f in flows {
            if self.is_internal(&f.src_ip) && self.is_internal(&f.dst_ip) {
                src_destinations
                    .entry(&f.src_ip)
                    .or_default()
                    .insert(&f.dst_ip);
            }
        }

        src_destinations
            .into_iter()
            .filter(|(_, dsts)| dsts.len() > 10)
            .map(|(src, dsts)| SecurityAlert {
                alert_type: "LateralMovement".to_string(),
                severity: "critical".to_string(),
                source_ip: src.to_string(),
                destination_ip: None,
                description: format!(
                    "Potential lateral movement: {src} connected to {} internal hosts",
                    dsts.len()
                ),
                detected_at: Utc::now(),
            })
            .collect()
    }

    pub fn generate_security_alerts(&self, flows: &[NetworkFlow]) -> Vec<SecurityAlert> {
        let mut alerts = self.check_policy_violations(flows);
        alerts.extend(self.detect_lateral_movement(flows));
        alerts
    }
}

impl Default for SecurityMonitor {
    fn default() -> Self {
        Self::new(vec![])
    }
}
