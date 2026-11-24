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
//! Candidate egress policies built from observed destinations.

use std::collections::BTreeMap;

use k8s_openapi::api::networking::v1::{
    IPBlock, NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyPeer, NetworkPolicyPort,
    NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use super::cluster::DestinationCluster;
use super::staging::StagingMode;

/// Label that marks a policy as synthesized by the egress synthesizer.
pub const SYNTHESIZED_LABEL: &str = "stellar-k8s.io/egress-synthesized";
/// Label that carries the enforcement mode of a synthesized policy.
pub const MODE_LABEL: &str = "stellar-k8s.io/egress-mode";
/// Label that carries the source of the synthesized policy.
pub const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
/// Value written to the managed-by label.
pub const MANAGED_BY_VALUE: &str = "stellar-k8s";

/// A single egress rule: one address range on one port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressRule {
    /// The address range the rule allows.
    pub cidr: String,
    /// The port the rule allows.
    pub port: u16,
    /// The transport protocol the rule allows.
    pub protocol: String,
}

impl EgressRule {
    /// Render the rule in the canonical `cidr:port/protocol` form.
    pub fn to_canonical(&self) -> String {
        format!("{}:{}/{}", self.cidr, self.port, self.protocol)
    }
}

/// A candidate egress policy derived from observed traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePolicy {
    /// Name of the policy.
    pub name: String,
    /// Namespace the policy belongs to.
    pub namespace: String,
    /// The egress rules of the policy.
    pub rules: Vec<EgressRule>,
    /// The mode the policy is rendered for.
    pub mode: StagingMode,
}

impl CandidatePolicy {
    /// Build a candidate policy from clustered destinations.
    pub fn build(
        workload: &str,
        namespace: &str,
        clusters: &[DestinationCluster],
        mode: StagingMode,
    ) -> Self {
        let rules = clusters
            .iter()
            .map(|cluster| EgressRule {
                cidr: cluster.cidr.clone(),
                port: cluster.port,
                protocol: cluster.protocol.clone(),
            })
            .collect();

        Self {
            name: format!("egress-{}", sanitize_name(workload)),
            namespace: namespace.to_string(),
            rules,
            mode,
        }
    }

    /// Labels applied to the rendered policy.
    pub fn labels(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
            (MODE_LABEL.to_string(), self.mode.as_str().to_string()),
            (SYNTHESIZED_LABEL.to_string(), "true".to_string()),
        ])
    }

    /// Render the candidate as a Kubernetes `NetworkPolicy`.
    pub fn to_network_policy(&self) -> NetworkPolicy {
        let rules: Vec<NetworkPolicyEgressRule> = self
            .rules
            .iter()
            .map(|rule| NetworkPolicyEgressRule {
                to: Some(vec![NetworkPolicyPeer {
                    ip_block: Some(IPBlock {
                        cidr: rule.cidr.clone(),
                        except: None,
                    }),
                    ..Default::default()
                }]),
                ports: Some(vec![NetworkPolicyPort {
                    port: Some(IntOrString::Int(i32::from(rule.port))),
                    protocol: Some(rule.protocol.clone()),
                    ..Default::default()
                }]),
            })
            .collect();

        NetworkPolicy {
            metadata: ObjectMeta {
                name: Some(self.name.clone()),
                namespace: Some(self.namespace.clone()),
                labels: Some(self.labels()),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec {
                pod_selector: LabelSelector::default(),
                ingress: Some(Vec::new()),
                egress: Some(rules),
                policy_types: Some(vec!["Egress".to_string()]),
            }),
        }
    }

    /// Render the candidate as YAML for review.
    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(&self.to_network_policy())
            .unwrap_or_else(|_| format!("# failed to render {}\n", self.name))
    }
}

/// Turn a workload name into a name that is valid for a Kubernetes object.
fn sanitize_name(workload: &str) -> String {
    let sanitized: String = workload
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('-').to_ascii_lowercase();
    if sanitized.is_empty() {
        "workload".to_string()
    } else {
        sanitized
    }
}
