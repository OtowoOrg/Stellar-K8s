//! Flow analysis with protocol detection.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::flow::{FlowStats, NetworkFlow, Protocol};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolBreakdown {
    pub tcp_flows: usize,
    pub udp_flows: usize,
    pub icmp_flows: usize,
    pub unknown_flows: usize,
    pub tcp_bytes: u64,
    pub udp_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortUsage {
    pub port: u16,
    pub flow_count: usize,
    pub total_bytes: u64,
    pub service_hint: Option<String>,
}

pub struct FlowAnalyzer;

impl FlowAnalyzer {
    /// Detect protocol from port numbers when protocol is Unknown.
    pub fn detect_protocol(src_port: u16, dst_port: u16) -> Protocol {
        let well_known = [dst_port, src_port];
        for port in well_known {
            match port {
                80 | 8080 | 8000 => return Protocol::Tcp,
                443 | 8443 => return Protocol::Tcp,
                53 => return Protocol::Udp,
                22 | 6789 | 11625 => return Protocol::Tcp,
                _ => {}
            }
        }
        Protocol::Unknown
    }

    /// Hint at service name from well-known ports.
    pub fn service_hint(port: u16) -> Option<&'static str> {
        match port {
            80 | 8080 => Some("http"),
            443 | 8443 => Some("https"),
            53 => Some("dns"),
            22 => Some("ssh"),
            5432 => Some("postgresql"),
            6379 => Some("redis"),
            9090 => Some("prometheus"),
            11625 => Some("stellar-peer"),
            11626 => Some("stellar-http"),
            _ => None,
        }
    }

    pub fn protocol_breakdown(flows: &[NetworkFlow]) -> ProtocolBreakdown {
        let mut bd = ProtocolBreakdown {
            tcp_flows: 0,
            udp_flows: 0,
            icmp_flows: 0,
            unknown_flows: 0,
            tcp_bytes: 0,
            udp_bytes: 0,
        };
        for f in flows {
            match f.protocol {
                Protocol::Tcp => {
                    bd.tcp_flows += 1;
                    bd.tcp_bytes += f.bytes;
                }
                Protocol::Udp => {
                    bd.udp_flows += 1;
                    bd.udp_bytes += f.bytes;
                }
                Protocol::Icmp => bd.icmp_flows += 1,
                Protocol::Unknown => bd.unknown_flows += 1,
            }
        }
        bd
    }

    pub fn top_ports(flows: &[NetworkFlow], limit: usize) -> Vec<PortUsage> {
        let mut map: HashMap<u16, (usize, u64)> = HashMap::new();
        for f in flows {
            let e = map.entry(f.dst_port).or_default();
            e.0 += 1;
            e.1 += f.bytes;
        }
        let mut ports: Vec<PortUsage> = map
            .into_iter()
            .map(|(port, (count, bytes))| PortUsage {
                port,
                flow_count: count,
                total_bytes: bytes,
                service_hint: Self::service_hint(port).map(|s| s.to_string()),
            })
            .collect();
        ports.sort_by(|a, b| b.flow_count.cmp(&a.flow_count));
        ports.truncate(limit);
        ports
    }

    pub fn compute_stats(flows: &[NetworkFlow]) -> FlowStats {
        if flows.is_empty() {
            return FlowStats::default();
        }
        let total_bytes: u64 = flows.iter().map(|f| f.bytes).sum();
        let total_packets: u64 = flows.iter().map(|f| f.packets).sum();
        let avg_duration =
            flows.iter().map(|f| f.duration_ms as f64).sum::<f64>() / flows.len() as f64;

        let mut talker_map: HashMap<String, u64> = HashMap::new();
        for f in flows {
            *talker_map.entry(f.src_ip.clone()).or_default() += f.bytes;
        }
        let mut top_talkers: Vec<_> = talker_map.into_iter().collect();
        top_talkers.sort_by(|a, b| b.1.cmp(&a.1));
        top_talkers.truncate(10);

        FlowStats {
            total_flows: flows.len(),
            total_bytes,
            total_packets,
            avg_duration_ms: avg_duration,
            top_talkers,
        }
    }
}
