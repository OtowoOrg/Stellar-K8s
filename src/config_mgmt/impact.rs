//! Configuration Change Impact Analysis
//!
//! Analyzes the potential impact of a configuration change before it is applied.

use crate::crd::StellarNodeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    pub score: f32, // 0.0 (no impact) to 1.0 (high impact)
    pub requires_restart: bool,
    pub potential_downtime: bool,
    pub resource_delta_cpu: f64,
    pub resource_delta_mem: i64,
}

pub struct ImpactAnalyzer;

impl ImpactAnalyzer {
    pub fn analyze(old: &StellarNodeSpec, new: &StellarNodeSpec) -> ImpactAnalysis {
        let mut score = 0.0;
        let mut requires_restart = false;
        let mut potential_downtime = false;

        // Version changes are high impact and require restart
        if old.version != new.version {
            score += 0.8;
            requires_restart = true;
            potential_downtime = true;
        }

        // Resource changes are medium impact
        if old.resources != new.resources {
            score += 0.3;
            requires_restart = true; // K8s pod restart usually needed for resource changes
        }

        // Network changes are critical
        if old.network != new.network {
            score = 1.0;
            requires_restart = true;
            potential_downtime = true;
        }

        ImpactAnalysis {
            score: score.min(1.0),
            requires_restart,
            potential_downtime,
            resource_delta_cpu: 0.0, // Simplified
            resource_delta_mem: 0,   // Simplified
        }
    }
}
