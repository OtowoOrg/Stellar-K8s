//! Scheduling policy framework with constraint definitions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SchedulingConstraint {
    ResourceLimit {
        cpu_milli: u64,
        memory_mb: u64,
    },
    NodeAffinity {
        key: String,
        values: Vec<String>,
    },
    PodAntiAffinity {
        label_selector: String,
    },
    Topology {
        spread_key: String,
        max_skew: i32,
    },
    CostBudget {
        max_hourly_cost_usd: f64,
    },
    NodeTaint {
        key: String,
        value: String,
        effect: String,
    },
}

/// Weights for multi-objective scoring (must sum to ~1.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityWeights {
    pub resource_fit: f64,
    pub cost: f64,
    pub locality: f64,
    pub balance: f64,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            resource_fit: 0.40,
            cost: 0.25,
            locality: 0.20,
            balance: 0.15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulingPolicy {
    pub name: String,
    pub constraints: Vec<SchedulingConstraint>,
    pub priority_weights: PriorityWeights,
    /// Preemption enabled for this policy
    pub allow_preemption: bool,
}

impl SchedulingPolicy {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            constraints: Vec::new(),
            priority_weights: PriorityWeights::default(),
            allow_preemption: false,
        }
    }

    pub fn with_constraint(mut self, c: SchedulingConstraint) -> Self {
        self.constraints.push(c);
        self
    }

    pub fn with_weights(mut self, w: PriorityWeights) -> Self {
        self.priority_weights = w;
        self
    }

    /// Default policy for Stellar validator nodes.
    pub fn stellar_validator() -> Self {
        Self::new("stellar-validator")
            .with_constraint(SchedulingConstraint::ResourceLimit {
                cpu_milli: 2000,
                memory_mb: 4096,
            })
            .with_constraint(SchedulingConstraint::PodAntiAffinity {
                label_selector: "stellar.org/node-type=Validator".to_string(),
            })
            .with_constraint(SchedulingConstraint::Topology {
                spread_key: "topology.kubernetes.io/zone".to_string(),
                max_skew: 1,
            })
    }
}
