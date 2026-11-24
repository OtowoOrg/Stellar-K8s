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
//! Cost allocation and chargeback per namespace/team label

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::model::CostRecord;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NamespaceCost {
    pub namespace: String,
    pub team: String,
    pub total_cost_usd: f64,
    pub resource_count: usize,
    pub breakdown: HashMap<String, f64>,
}

#[derive(Clone, Debug, Default)]
pub struct CostAllocation {
    namespace_costs: HashMap<String, NamespaceCost>,
}

impl CostAllocation {
    pub fn allocate(&mut self, records: &[CostRecord]) {
        self.namespace_costs.clear();
        for r in records {
            let entry = self
                .namespace_costs
                .entry(r.namespace.clone())
                .or_insert_with(|| NamespaceCost {
                    namespace: r.namespace.clone(),
                    team: r.team.clone(),
                    ..Default::default()
                });
            entry.total_cost_usd += r.cost_usd;
            entry.resource_count += 1;
            *entry
                .breakdown
                .entry(format!("{:?}", r.resource_type))
                .or_insert(0.0) += r.cost_usd;
        }
    }

    pub fn by_namespace(&self) -> Vec<&NamespaceCost> {
        let mut v: Vec<&NamespaceCost> = self.namespace_costs.values().collect();
        v.sort_by(|a, b| b.total_cost_usd.partial_cmp(&a.total_cost_usd).unwrap());
        v
    }

    pub fn total(&self) -> f64 {
        self.namespace_costs
            .values()
            .map(|n| n.total_cost_usd)
            .sum()
    }

    pub fn to_csv(&self) -> String {
        let mut csv = "namespace,team,total_cost_usd,resource_count\n".to_string();
        for ns in self.by_namespace() {
            csv.push_str(&format!(
                "{},{},{:.4},{}\n",
                ns.namespace, ns.team, ns.total_cost_usd, ns.resource_count
            ));
        }
        csv
    }
}
