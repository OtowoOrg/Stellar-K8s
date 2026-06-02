//! Network anomaly detection: DDoS, port scanning, unusual traffic patterns.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::flow::NetworkFlow;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyType {
    DDoS,
    PortScan,
    UnusualTraffic,
    LateralMovement,
    DataExfiltration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum AnomalySeverity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAnomaly {
    pub anomaly_type: AnomalyType,
    pub severity: AnomalySeverity,
    pub source_ip: String,
    pub description: String,
    pub detected_at: DateTime<Utc>,
    pub flow_count: usize,
}

pub struct AnomalyDetector {
    /// Packets-per-second threshold for DDoS detection
    pub ddos_pps_threshold: u64,
    /// Number of distinct ports to trigger port scan detection
    pub port_scan_threshold: usize,
    /// Bytes threshold for data exfiltration detection
    pub exfil_bytes_threshold: u64,
}

impl Default for AnomalyDetector {
    fn default() -> Self {
        Self {
            ddos_pps_threshold: 10_000,
            port_scan_threshold: 20,
            exfil_bytes_threshold: 100 * 1024 * 1024, // 100 MB
        }
    }
}

impl AnomalyDetector {
    pub fn detect_all(&self, flows: &[NetworkFlow]) -> Vec<NetworkAnomaly> {
        let mut anomalies = Vec::new();
        anomalies.extend(self.detect_ddos(flows));
        anomalies.extend(self.detect_port_scan(flows));
        anomalies.extend(self.detect_data_exfiltration(flows));
        anomalies
    }

    pub fn detect_ddos(&self, flows: &[NetworkFlow]) -> Vec<NetworkAnomaly> {
        // Group by source IP, sum packets, check rate
        let mut src_packets: HashMap<&str, u64> = HashMap::new();
        let mut src_flows: HashMap<&str, usize> = HashMap::new();
        for f in flows {
            *src_packets.entry(&f.src_ip).or_default() += f.packets;
            *src_flows.entry(&f.src_ip).or_default() += 1;
        }

        src_packets
            .into_iter()
            .filter(|(_, packets)| *packets > self.ddos_pps_threshold)
            .map(|(ip, packets)| NetworkAnomaly {
                anomaly_type: AnomalyType::DDoS,
                severity: if packets > self.ddos_pps_threshold * 10 {
                    AnomalySeverity::Critical
                } else {
                    AnomalySeverity::High
                },
                source_ip: ip.to_string(),
                description: format!(
                    "Potential DDoS: {ip} sent {packets} packets (threshold: {})",
                    self.ddos_pps_threshold
                ),
                detected_at: Utc::now(),
                flow_count: *src_flows.get(ip).unwrap_or(&0),
            })
            .collect()
    }

    pub fn detect_port_scan(&self, flows: &[NetworkFlow]) -> Vec<NetworkAnomaly> {
        // Group by source IP, count distinct destination ports
        let mut src_ports: HashMap<&str, std::collections::HashSet<u16>> = HashMap::new();
        let mut src_flows: HashMap<&str, usize> = HashMap::new();
        for f in flows {
            src_ports.entry(&f.src_ip).or_default().insert(f.dst_port);
            *src_flows.entry(&f.src_ip).or_default() += 1;
        }

        src_ports
            .into_iter()
            .filter(|(_, ports)| ports.len() >= self.port_scan_threshold)
            .map(|(ip, ports)| NetworkAnomaly {
                anomaly_type: AnomalyType::PortScan,
                severity: AnomalySeverity::High,
                source_ip: ip.to_string(),
                description: format!(
                    "Port scan detected: {ip} probed {} distinct ports",
                    ports.len()
                ),
                detected_at: Utc::now(),
                flow_count: *src_flows.get(ip).unwrap_or(&0),
            })
            .collect()
    }

    pub fn detect_data_exfiltration(&self, flows: &[NetworkFlow]) -> Vec<NetworkAnomaly> {
        let mut src_bytes: HashMap<&str, u64> = HashMap::new();
        let mut src_flows: HashMap<&str, usize> = HashMap::new();
        for f in flows {
            *src_bytes.entry(&f.src_ip).or_default() += f.bytes;
            *src_flows.entry(&f.src_ip).or_default() += 1;
        }

        src_bytes
            .into_iter()
            .filter(|(_, bytes)| *bytes > self.exfil_bytes_threshold)
            .map(|(ip, bytes)| NetworkAnomaly {
                anomaly_type: AnomalyType::DataExfiltration,
                severity: AnomalySeverity::Critical,
                source_ip: ip.to_string(),
                description: format!(
                    "Potential data exfiltration: {ip} sent {} MB",
                    bytes / 1024 / 1024
                ),
                detected_at: Utc::now(),
                flow_count: *src_flows.get(ip).unwrap_or(&0),
            })
            .collect()
    }
}
