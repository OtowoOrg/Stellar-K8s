// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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
        let mut score: f32 = 0.0;
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
