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
//! Clustering of observed destinations into CIDR + port tuples.
//!
//! A workload usually talks to a handful of address ranges rather than to
//! thousands of single hosts. Clustering the observed destinations keeps the
//! synthesized policy small and reviewable while still describing what the
//! workload really did.

use std::collections::BTreeMap;

use super::flow::ObservedFlow;

/// A group of observed destinations that share a CIDR, a port and a protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationCluster {
    /// The aggregated address range, e.g. `10.0.0.0/24`.
    pub cidr: String,
    /// The destination port.
    pub port: u16,
    /// The transport protocol, upper case.
    pub protocol: String,
    /// The distinct destinations observed inside the range.
    pub observed_ips: Vec<String>,
    /// How many flows were seen for this tuple.
    pub flow_count: usize,
    /// Total bytes seen for this tuple.
    pub total_bytes: u64,
}

impl DestinationCluster {
    /// Number of distinct destinations covered by the tuple.
    pub fn destination_count(&self) -> usize {
        self.observed_ips.len()
    }
}

/// Cluster the observed destinations of `flows` into CIDR + port tuples.
///
/// `prefix_len` is the IPv4 prefix length used for the aggregation; `24`
/// keeps a cluster on a single subnet. Destinations that are not IPv4 are
/// kept as individual entries so they are never widened by accident.
pub fn cluster_destinations(flows: &[ObservedFlow], prefix_len: u8) -> Vec<DestinationCluster> {
    let prefix_len = prefix_len.min(32);

    let mut grouped: BTreeMap<(String, u16, String), DestinationCluster> = BTreeMap::new();

    for flow in flows {
        let cidr = match ipv4_network(&flow.dst_ip, prefix_len) {
            Some(network) => network,
            None => flow.dst_ip.clone(),
        };

        let key = (cidr.clone(), flow.dst_port, flow.protocol.clone());
        let cluster = grouped.entry(key).or_insert_with(|| DestinationCluster {
            cidr,
            port: flow.dst_port,
            protocol: flow.protocol.clone(),
            observed_ips: Vec::new(),
            flow_count: 0,
            total_bytes: 0,
        });

        cluster.flow_count += 1;
        cluster.total_bytes += flow.bytes;
        if !cluster.observed_ips.contains(&flow.dst_ip) {
            cluster.observed_ips.push(flow.dst_ip.clone());
        }
    }

    let mut clusters: Vec<DestinationCluster> = grouped.into_values().collect();
    for cluster in &mut clusters {
        cluster.observed_ips.sort();
    }
    clusters.sort_by(|left, right| {
        right
            .flow_count
            .cmp(&left.flow_count)
            .then_with(|| left.cidr.cmp(&right.cidr))
            .then_with(|| left.port.cmp(&right.port))
            .then_with(|| left.protocol.cmp(&right.protocol))
    });

    clusters
}

/// Aggregate an IPv4 address into its network address for `prefix_len`.
pub fn ipv4_network(ip: &str, prefix_len: u8) -> Option<String> {
    let address = ipv4_to_u32(ip)?;
    let mask = prefix_mask(prefix_len);
    let network = address & mask;
    Some(format!("{}/{}", u32_to_ipv4(network), prefix_len))
}

/// Convert an IPv4 address into its numeric representation.
pub fn ipv4_to_u32(ip: &str) -> Option<u32> {
    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in ip.trim().split('.') {
        if count == 4 {
            return None;
        }
        let octet: u8 = part.parse().ok()?;
        octets[count] = octet;
        count += 1;
    }
    if count != 4 {
        return None;
    }
    Some(u32::from_be_bytes(octets))
}

/// Convert a numeric address back into its dotted representation.
pub fn u32_to_ipv4(address: u32) -> String {
    let octets = address.to_be_bytes();
    format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])
}

fn prefix_mask(prefix_len: u8) -> u32 {
    if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    }
}
