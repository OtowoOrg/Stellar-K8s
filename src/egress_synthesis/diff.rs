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
//! Readable diffs between the applied and the synthesized egress policy.

use std::collections::BTreeSet;

use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

/// The difference between the applied egress policy and a candidate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyDiff {
    /// Rules the candidate adds.
    pub added: Vec<String>,
    /// Rules the candidate drops.
    pub removed: Vec<String>,
    /// Rules both policies share.
    pub unchanged: Vec<String>,
}

impl PolicyDiff {
    /// Whether the candidate is identical to the applied policy.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// Render the diff as YAML so it can be reviewed in a pull request.
    pub fn to_yaml(&self) -> String {
        let mut out = String::from("# egress policy diff\n");
        out.push_str(&render_section("added", &self.added));
        out.push_str(&render_section("removed", &self.removed));
        out.push_str(&render_section("unchanged", &self.unchanged));
        out
    }
}

/// Render the difference between the applied and the candidate policy.
pub fn render_policy_diff(applied: &NetworkPolicy, candidate: &NetworkPolicy) -> PolicyDiff {
    let applied_rules = egress_rules(applied);
    let candidate_rules = egress_rules(candidate);

    let mut diff = PolicyDiff::default();

    for rule in &candidate_rules {
        if applied_rules.contains(rule) {
            diff.unchanged.push(rule.clone());
        } else {
            diff.added.push(rule.clone());
        }
    }

    for rule in &applied_rules {
        if !candidate_rules.contains(rule) {
            diff.removed.push(rule.clone());
        }
    }

    diff
}

/// Extract the canonical rule strings of a policy.
fn egress_rules(policy: &NetworkPolicy) -> BTreeSet<String> {
    let mut rules = BTreeSet::new();

    let Some(spec) = policy.spec.as_ref() else {
        return rules;
    };

    for rule in spec.egress.iter().flatten() {
        let cidrs: Vec<String> = rule
            .to
            .iter()
            .flatten()
            .filter_map(|peer| peer.ip_block.as_ref())
            .map(|block| block.cidr.clone())
            .collect();

        let ports: Vec<(u16, String)> = rule
            .ports
            .iter()
            .flatten()
            .map(|port| {
                let number = match port.port.as_ref() {
                    Some(IntOrString::Int(value)) => *value as u16,
                    _ => 0,
                };
                (
                    number,
                    port.protocol.clone().unwrap_or_else(|| "TCP".to_string()),
                )
            })
            .collect();

        for cidr in &cidrs {
            if ports.is_empty() {
                rules.insert(format!("{cidr}:any/ANY"));
                continue;
            }
            for (port, protocol) in &ports {
                rules.insert(format!("{cidr}:{port}/{protocol}"));
            }
        }
    }

    rules
}

/// Render one section of the diff.
fn render_section(name: &str, entries: &[String]) -> String {
    if entries.is_empty() {
        return format!("{name}: []\n");
    }
    let mut out = format!("{name}:\n");
    for entry in entries {
        out.push_str(&format!("  - \"{entry}\"\n"));
    }
    out
}
