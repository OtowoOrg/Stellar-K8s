//! Network performance analysis: latency, throughput, bottleneck detection.

use serde::{Deserialize, Serialize};

use super::flow::NetworkFlow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyPercentiles {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub min_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputStats {
    pub total_bytes: u64,
    pub total_packets: u64,
    /// Megabits per second (estimated over flow window)
    pub throughput_mbps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    pub source: String,
    pub destination: String,
    pub avg_latency_ms: f64,
    pub reason: String,
}

pub struct PerformanceAnalyzer;

impl PerformanceAnalyzer {
    pub fn compute_latency_percentiles(flows: &[NetworkFlow]) -> Option<LatencyPercentiles> {
        if flows.is_empty() {
            return None;
        }
        let mut latencies: Vec<f64> = flows.iter().map(|f| f.duration_ms as f64).collect();
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let len = latencies.len();

        Some(LatencyPercentiles {
            p50_ms: percentile(&latencies, 50.0),
            p95_ms: percentile(&latencies, 95.0),
            p99_ms: percentile(&latencies, 99.0),
            max_ms: latencies[len - 1],
            min_ms: latencies[0],
        })
    }

    pub fn compute_throughput(flows: &[NetworkFlow]) -> ThroughputStats {
        let total_bytes: u64 = flows.iter().map(|f| f.bytes).sum();
        let total_packets: u64 = flows.iter().map(|f| f.packets).sum();

        // Estimate window from min/max timestamps
        let throughput_mbps = if flows.len() < 2 {
            0.0
        } else {
            let min_ts = flows.iter().map(|f| f.timestamp).min().unwrap();
            let max_ts = flows.iter().map(|f| f.timestamp).max().unwrap();
            let window_secs = (max_ts - min_ts).num_seconds().max(1) as f64;
            (total_bytes as f64 * 8.0) / (window_secs * 1_000_000.0)
        };

        ThroughputStats {
            total_bytes,
            total_packets,
            throughput_mbps,
        }
    }

    pub fn identify_bottlenecks(
        flows: &[NetworkFlow],
        latency_threshold_ms: f64,
    ) -> Vec<Bottleneck> {
        use std::collections::HashMap;

        let mut pair_latencies: HashMap<(String, String), Vec<f64>> = HashMap::new();
        for f in flows {
            let dst = f.service_name.clone().unwrap_or_else(|| f.dst_ip.clone());
            pair_latencies
                .entry((f.pod_name.clone(), dst))
                .or_default()
                .push(f.duration_ms as f64);
        }

        pair_latencies
            .into_iter()
            .filter_map(|((src, dst), latencies)| {
                let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
                if avg > latency_threshold_ms {
                    Some(Bottleneck {
                        source: src,
                        destination: dst,
                        avg_latency_ms: avg,
                        reason: format!(
                            "Average latency {avg:.1}ms exceeds threshold {latency_threshold_ms}ms"
                        ),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
