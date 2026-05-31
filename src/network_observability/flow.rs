//! Network flow capture and storage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_FLOWS: usize = 10_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Protocol {
    Tcp,
    Udp,
    Icmp,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkFlow {
    pub src_ip: String,
    pub dst_ip: String,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: Protocol,
    pub bytes: u64,
    pub packets: u64,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
    pub namespace: String,
    pub pod_name: String,
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowStats {
    pub total_flows: usize,
    pub total_bytes: u64,
    pub total_packets: u64,
    pub avg_duration_ms: f64,
    pub top_talkers: Vec<(String, u64)>, // (src_ip, bytes)
}

#[derive(Clone)]
pub struct FlowStore {
    flows: Arc<RwLock<VecDeque<NetworkFlow>>>,
}

impl FlowStore {
    pub fn new() -> Self {
        Self {
            flows: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_FLOWS))),
        }
    }

    pub async fn add_flow(&self, flow: NetworkFlow) {
        let mut store = self.flows.write().await;
        if store.len() >= MAX_FLOWS {
            store.pop_front();
        }
        store.push_back(flow);
    }

    pub async fn add_flows(&self, flows: Vec<NetworkFlow>) {
        for flow in flows {
            self.add_flow(flow).await;
        }
    }

    pub async fn query_by_namespace(&self, namespace: &str) -> Vec<NetworkFlow> {
        self.flows
            .read()
            .await
            .iter()
            .filter(|f| f.namespace == namespace)
            .cloned()
            .collect()
    }

    pub async fn query_by_pod(&self, pod_name: &str) -> Vec<NetworkFlow> {
        self.flows
            .read()
            .await
            .iter()
            .filter(|f| f.pod_name == pod_name)
            .cloned()
            .collect()
    }

    pub async fn query_recent(&self, limit: usize) -> Vec<NetworkFlow> {
        let store = self.flows.read().await;
        store.iter().rev().take(limit).cloned().collect()
    }

    pub async fn all(&self) -> Vec<NetworkFlow> {
        self.flows.read().await.iter().cloned().collect()
    }

    pub async fn stats(&self) -> FlowStats {
        let flows = self.flows.read().await;
        if flows.is_empty() {
            return FlowStats::default();
        }
        let total_bytes: u64 = flows.iter().map(|f| f.bytes).sum();
        let total_packets: u64 = flows.iter().map(|f| f.packets).sum();
        let avg_duration =
            flows.iter().map(|f| f.duration_ms as f64).sum::<f64>() / flows.len() as f64;

        let mut talker_map: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        for f in flows.iter() {
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

impl Default for FlowStore {
    fn default() -> Self {
        Self::new()
    }
}
