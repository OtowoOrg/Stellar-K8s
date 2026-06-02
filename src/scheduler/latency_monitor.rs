//! Dynamic latency monitoring and pod eviction for proximity-aware scheduling.
//!
//! When inter-peer latency exceeds the configured threshold, validator pods are
//! evicted so the custom scheduler can reschedule them onto lower-latency nodes.

use anyhow::Result;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::EvictParams, Api, Client, ResourceExt};
use std::time::Duration;
use tracing::{info, warn};

use super::prometheus::PrometheusClient;
use super::scoring::extract_peer_names_from_toml;

/// Default maximum acceptable quorum peer latency in milliseconds.
pub const DEFAULT_LATENCY_THRESHOLD_MS: f64 = 150.0;

/// How often the monitor evaluates latency and triggers evictions.
const MONITOR_INTERVAL: Duration = Duration::from_secs(60);

/// Cooldown between evictions for the same pod to avoid thrashing.
const EVICTION_COOLDOWN: Duration = Duration::from_secs(300);

pub struct LatencyMonitor {
    client: Client,
    prometheus: PrometheusClient,
    threshold_ms: f64,
    last_evictions: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl LatencyMonitor {
    pub fn new(client: Client, prometheus_url: String) -> Self {
        let threshold = std::env::var("LATENCY_EVICTION_THRESHOLD_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_LATENCY_THRESHOLD_MS);

        Self {
            client,
            prometheus: PrometheusClient::new(prometheus_url),
            threshold_ms: threshold,
            last_evictions: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Run the latency monitor loop indefinitely.
    pub async fn run(&self) -> Result<()> {
        info!(
            "Starting latency monitor (threshold={}ms, interval={}s)",
            self.threshold_ms,
            MONITOR_INTERVAL.as_secs()
        );

        loop {
            match self.evaluate_and_benchmark().await {
                Ok(bm) => {
                    info!(
                        "Latency monitor cycle: evaluated={} above_threshold={} evictions={} avg_ms={:.1} max_ms={:.1}",
                        bm.pods_evaluated,
                        bm.pods_above_threshold,
                        bm.evictions_triggered,
                        bm.avg_latency_ms.unwrap_or(0.0),
                        bm.max_latency_ms.unwrap_or(0.0),
                    );
                }
                Err(e) => warn!("Latency monitor cycle error: {}", e),
            }
            tokio::time::sleep(MONITOR_INTERVAL).await;
        }
    }

    async fn evaluate_once(&self) -> Result<()> {
        self.evaluate_and_benchmark().await.map(|_| ())
    }

    async fn measure_quorum_latency(&self, pod: &Pod) -> Result<Option<f64>> {
        let instance_name = match pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("app.kubernetes.io/instance"))
        {
            Some(n) => n.clone(),
            None => return Ok(None),
        };

        let namespace = pod.metadata.namespace.as_deref().unwrap_or("default");
        let stellar_nodes: Api<crate::crd::StellarNode> =
            Api::namespaced(self.client.clone(), namespace);

        let node_cr = match stellar_nodes.get(&instance_name).await {
            Ok(n) if n.spec.proximity_aware => n,
            _ => return Ok(None),
        };

        let quorum_set = match node_cr
            .spec
            .validator_config
            .as_ref()
            .and_then(|c| c.quorum_set.as_ref())
        {
            Some(q) => q,
            None => return Ok(None),
        };

        let peer_names = extract_peer_names_from_toml(quorum_set);
        if peer_names.is_empty() {
            return Ok(None);
        }

        let mut latencies = Vec::new();
        for peer in &peer_names {
            if let Ok(Some(lat)) = self
                .prometheus
                .get_validator_latency(namespace, peer, "5m")
                .await
            {
                latencies.push(lat);
            }
        }

        if latencies.is_empty() {
            return Ok(None);
        }

        let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
        Ok(Some(avg))
    }

    async fn evict_pod(&self, namespace: &str, pod_name: &str) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        match pods.evict(pod_name, &EvictParams::default()).await {
            Ok(_) => {
                info!(
                    "Evicted pod {}/{} for latency optimization",
                    namespace, pod_name
                );
                Ok(())
            }
            Err(e) => {
                warn!("Failed to evict pod {}/{}: {}", namespace, pod_name, e);
                Ok(())
            }
        }
    }

    fn is_in_cooldown(&self, namespace: &str, pod_name: &str) -> bool {
        let key = format!("{namespace}/{pod_name}");
        let guard = self.last_evictions.lock().unwrap();
        guard
            .get(&key)
            .is_some_and(|t| t.elapsed() < EVICTION_COOLDOWN)
    }

    fn record_eviction(&self, namespace: &str, pod_name: &str) {
        let key = format!("{namespace}/{pod_name}");
        let mut guard = self.last_evictions.lock().unwrap();
        guard.insert(key, std::time::Instant::now());
    }
}

fn is_proximity_validator(pod: &Pod) -> bool {
    pod.metadata.labels.as_ref().is_some_and(|labels| {
        labels.get("stellar.org/node-type").map(|s| s.as_str()) == Some("Validator")
            && labels.contains_key("app.kubernetes.io/instance")
    })
}

/// Benchmark summary produced by the latency monitor after each evaluation cycle.
///
/// Exposed for observability and integration tests. The scheduler uses these
/// numbers to decide whether eviction-based rescheduling is improving latency.
#[derive(Clone, Debug, Default)]
pub struct LatencyBenchmark {
    /// Number of validator pods evaluated in this cycle.
    pub pods_evaluated: usize,
    /// Number of pods whose average quorum latency exceeded the threshold.
    pub pods_above_threshold: usize,
    /// Number of evictions triggered in this cycle.
    pub evictions_triggered: usize,
    /// Average quorum latency across all evaluated pods (ms). `None` if no data.
    pub avg_latency_ms: Option<f64>,
    /// Maximum quorum latency observed across all evaluated pods (ms). `None` if no data.
    pub max_latency_ms: Option<f64>,
}

impl LatencyMonitor {
    /// Run one evaluation cycle and return a [`LatencyBenchmark`] summary.
    ///
    /// This is the testable, metrics-producing variant of `evaluate_once`.
    pub async fn evaluate_and_benchmark(&self) -> Result<LatencyBenchmark> {
        let pods: Api<Pod> = Api::all(self.client.clone());
        let all_pods = pods.list(&kube::api::ListParams::default()).await?;

        let mut benchmark = LatencyBenchmark::default();
        let mut latency_samples: Vec<f64> = Vec::new();

        for pod in all_pods.items {
            if !is_proximity_validator(&pod) {
                continue;
            }

            benchmark.pods_evaluated += 1;
            let pod_name = pod.name_any();
            let namespace = pod.namespace().unwrap_or_else(|| "default".into());

            if self.is_in_cooldown(&namespace, &pod_name) {
                continue;
            }

            let avg_latency = self.measure_quorum_latency(&pod).await?;
            if let Some(latency) = avg_latency {
                latency_samples.push(latency);
                if latency > self.threshold_ms {
                    benchmark.pods_above_threshold += 1;
                    info!(
                        "Pod {}/{} avg quorum latency {:.1}ms exceeds threshold {:.1}ms — evicting for reschedule",
                        namespace, pod_name, latency, self.threshold_ms
                    );
                    self.evict_pod(&namespace, &pod_name).await?;
                    self.record_eviction(&namespace, &pod_name);
                    benchmark.evictions_triggered += 1;
                }
            }
        }

        if !latency_samples.is_empty() {
            let sum: f64 = latency_samples.iter().sum();
            benchmark.avg_latency_ms = Some(sum / latency_samples.len() as f64);
            benchmark.max_latency_ms = latency_samples.iter().cloned().reduce(f64::max);
        }

        Ok(benchmark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn make_validator_pod(name: &str) -> Pod {
        let mut labels = BTreeMap::new();
        labels.insert("stellar.org/node-type".to_string(), "Validator".to_string());
        labels.insert("app.kubernetes.io/instance".to_string(), name.to_string());
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("stellar".to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_is_proximity_validator() {
        assert!(is_proximity_validator(&make_validator_pod("v1")));
    }

    #[test]
    fn test_horizon_pod_not_proximity_validator() {
        let mut labels = BTreeMap::new();
        labels.insert("stellar.org/node-type".to_string(), "Horizon".to_string());
        let pod = Pod {
            metadata: ObjectMeta {
                labels: Some(labels),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!is_proximity_validator(&pod));
    }

    #[test]
    fn test_latency_threshold_default() {
        assert_eq!(DEFAULT_LATENCY_THRESHOLD_MS, 150.0);
    }

    #[test]
    fn test_eviction_cooldown_logic() {
        let mut last_evictions: std::collections::HashMap<String, std::time::Instant> =
            std::collections::HashMap::new();
        last_evictions.insert("stellar/v1".to_string(), std::time::Instant::now());

        let in_cooldown = last_evictions
            .get("stellar/v1")
            .is_some_and(|t| t.elapsed() < EVICTION_COOLDOWN);
        assert!(in_cooldown);

        let not_in_cooldown = last_evictions
            .get("stellar/v2")
            .is_some_and(|t| t.elapsed() < EVICTION_COOLDOWN);
        assert!(!not_in_cooldown);
    }

    #[test]
    fn test_benchmark_default_is_zero() {
        let bm = LatencyBenchmark::default();
        assert_eq!(bm.pods_evaluated, 0);
        assert_eq!(bm.pods_above_threshold, 0);
        assert_eq!(bm.evictions_triggered, 0);
        assert!(bm.avg_latency_ms.is_none());
        assert!(bm.max_latency_ms.is_none());
    }

    #[test]
    fn test_benchmark_avg_and_max_computed() {
        // Simulate what evaluate_and_benchmark does with latency samples
        let samples = vec![10.0_f64, 50.0, 200.0];
        let sum: f64 = samples.iter().sum();
        let avg = sum / samples.len() as f64;
        let max = samples.iter().cloned().reduce(f64::max).unwrap();
        assert!((avg - 86.666).abs() < 0.01);
        assert_eq!(max, 200.0);
    }

    #[test]
    fn test_benchmark_above_threshold_count() {
        let threshold = 150.0_f64;
        let samples = vec![10.0_f64, 50.0, 200.0, 300.0];
        let above = samples.iter().filter(|&&l| l > threshold).count();
        assert_eq!(above, 2);
    }
}
