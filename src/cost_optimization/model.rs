//! Cost tracking data model with multi-cloud provider support

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CloudProvider {
    Aws,
    Gcp,
    Azure,
    OnPremise,
}

impl std::fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aws => write!(f, "AWS"),
            Self::Gcp => write!(f, "GCP"),
            Self::Azure => write!(f, "Azure"),
            Self::OnPremise => write!(f, "OnPremise"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ResourceType {
    ComputeInstance,
    ManagedDisk,
    NetworkEgress,
    LoadBalancer,
    ObjectStorage,
    ManagedDatabase,
    KubernetesNode,
}

/// A single cost record for a cloud resource in a billing period
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostRecord {
    pub id: String,
    pub provider: CloudProvider,
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub namespace: String,
    pub team: String,
    pub region: String,
    pub instance_type: String,
    pub cost_usd: f64,
    pub usage_hours: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub tags: HashMap<String, String>,
    pub is_spot: bool,
    pub is_reserved: bool,
}

impl CostRecord {
    pub fn hourly_rate(&self) -> f64 {
        if self.usage_hours > 0.0 { self.cost_usd / self.usage_hours } else { 0.0 }
    }
}
