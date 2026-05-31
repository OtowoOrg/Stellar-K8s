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
            if let Err(e) = self.evaluate_once().await {
                warn!("Latency monitor cycle error: {}", e);
            }
            tokio::time::sleep(MONITOR_INTERVAL).await;
        }
    }

    async fn evaluate_once(&self) -> Result<()> {
        let pods: Api<Pod> = Api::all(self.client.clone());
        let all_pods = pods.list(&kube::api::ListParams::default()).await?;

        for pod in all_pods.items {
            if !is_proximity_validator(&pod) {
                continue;
            }

            let pod_name = pod.name_any();
            let namespace = pod.namespace().unwrap_or_else(|| "default".into());

            if self.is_in_cooldown(&namespace, &pod_name) {
                continue;
            }

            let avg_latency = self.measure_quorum_latency(&pod).await?;
            if let Some(latency) = avg_latency {
                if latency > self.threshold_ms {
                    info!(
                        "Pod {}/{} avg quorum latency {:.1}ms exceeds threshold {:.1}ms — evicting for reschedule",
                        namespace, pod_name, latency, self.threshold_ms
                    );
                    self.evict_pod(&namespace, &pod_name).await?;
                    self.record_eviction(&namespace, &pod_name);
                }
            }
        }

        Ok(())
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
}
