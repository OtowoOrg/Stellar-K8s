//! Horizon ingestion lag-based rollout gate
//!
//! Manages StatefulSet rolling updates for Horizon nodes by monitoring ingestion lag
//! and pausing updates until pods meet health thresholds.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{Event, EventSource, ObjectFieldSelector, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::{Client, ResourceExt};
use tracing::{debug, info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

use super::health::{RolloutHealthChecker, RolloutHealthConfig};
use super::{CHECK_START_TIME_ANNOTATION, LAST_CHECKED_POD_ANNOTATION, ROLLOUT_GATE_ANNOTATION};

/// Manages rollout gating for Horizon nodes
pub struct HorizonRolloutGate;

/// Result of a rollout gate check
#[derive(Debug, Clone)]
pub enum RolloutGateResult {
    /// Rollout can proceed
    Approved {
        pod_name: String,
        ingestion_lag: u64,
    },

    /// Rollout is paused, waiting for pod to catch up
    Paused {
        pod_name: String,
        ingestion_lag: u64,
        elapsed: Duration,
        timeout: Duration,
    },

    /// Rollout timed out waiting for pod health
    Failed {
        pod_name: String,
        reason: String,
    },

    /// No update in progress
    NoUpdate,
}

impl HorizonRolloutGate {
    /// Check if a StatefulSet update should proceed
    ///
    /// This function:
    /// 1. Detects if a rolling update is in progress
    /// 2. Finds the most recently updated pod
    /// 3. Checks its Horizon ingestion lag
    /// 4. Allows update to proceed only if lag <= threshold
    /// 5. Emits Kubernetes events for visibility
    pub async fn check_update_progress(
        client: &Client,
        node: &StellarNode,
        statefulset: &StatefulSet,
        config: &RolloutHealthConfig,
    ) -> Result<RolloutGateResult> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let sts_name = statefulset.name_any();

        debug!(
            "Checking rollout gate for StatefulSet {}/{}",
            namespace, sts_name
        );

        // Check if update is in progress
        let update_status = Self::get_update_status(statefulset);
        if update_status.no_update_in_progress {
            debug!("No update in progress for {}", sts_name);
            return Ok(RolloutGateResult::NoUpdate);
        }

        // Get the most recently updated pod
        let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
        let pods = pod_api
            .list(&Default::default())
            .await
            .map_err(Error::KubeError)?;

        let current_pod = pods.items.iter().find(|pod| {
            pod.labels()
                .get("app.kubernetes.io/instance")
                .map(|v| v == &node.name_any())
                .unwrap_or(false)
                && pod
                    .labels()
                    .get("app.kubernetes.io/component")
                    .map(|v| v == "horizon")
                    .unwrap_or(false)
        });

        let pod = match current_pod {
            Some(p) => p,
            None => {
                return Ok(RolloutGateResult::NoUpdate);
            }
        };

        let pod_name = pod.name_any();
        let pod_ip = match &pod.status {
            Some(status) => match status.pod_ip.as_ref() {
                Some(ip) => ip.clone(),
                None => {
                    debug!("Pod {} does not have an IP yet", pod_name);
                    return Ok(RolloutGateResult::Paused {
                        pod_name,
                        ingestion_lag: u64::MAX,
                        elapsed: Duration::from_secs(0),
                        timeout: config.rollout_pause_timeout,
                    });
                }
            },
            None => {
                debug!("Pod {} has no status yet", pod_name);
                return Ok(RolloutGateResult::NoUpdate);
            }
        };

        // Check if pod is ready
        let is_ready = Self::is_pod_ready(pod);
        if !is_ready {
            debug!("Pod {} is not ready yet", pod_name);
            return Ok(RolloutGateResult::Paused {
                pod_name,
                ingestion_lag: u64::MAX,
                elapsed: Duration::from_secs(0),
                timeout: config.rollout_pause_timeout,
            });
        }

        // Get check start time from pod annotations
        let annotations = pod.annotations();
        let check_start_time = annotations
            .get(CHECK_START_TIME_ANNOTATION)
            .and_then(|ts| ts.parse::<u64>().ok())
            .map(SystemTime::UNIX_EPOCH + Duration::from_secs_since_epoch)
            .unwrap_or_else(SystemTime::now);

        let elapsed = SystemTime::now()
            .duration_since(check_start_time)
            .unwrap_or_default();

        // Check ingestion lag
        let health = RolloutHealthChecker::check_horizon_ingestion_lag(&pod_ip, config).await?;

        info!(
            "Pod {}: ingestion_lag={}, meets_threshold={}",
            pod_name, health.ingestion_lag, health.meets_threshold
        );

        if health.meets_threshold {
            // Update pod annotation to mark gate as passed
            Self::mark_gate_passed(client, pod).await.ok();

            info!("Rollout gate APPROVED for pod {}", pod_name);
            Ok(RolloutGateResult::Approved {
                pod_name,
                ingestion_lag: health.ingestion_lag,
            })
        } else if elapsed > config.rollout_pause_timeout {
            warn!(
                "Rollout gate FAILED for pod {}: timeout after {:?}",
                pod_name, elapsed
            );

            // Emit warning event
            Self::emit_event(client, pod, "RolloutGateTimeout", &health.message)
                .await
                .ok();

            Ok(RolloutGateResult::Failed {
                pod_name,
                reason: format!(
                    "Ingestion lag {} did not reach threshold within {:?}",
                    health.ingestion_lag, config.rollout_pause_timeout
                ),
            })
        } else {
            debug!(
                "Rollout gate PAUSED for pod {}: lag {}, elapsed {:?}",
                pod_name, health.ingestion_lag, elapsed
            );

            // Emit info event
            Self::emit_event(client, pod, "RolloutGatePaused", &health.message)
                .await
                .ok();

            Ok(RolloutGateResult::Paused {
                pod_name,
                ingestion_lag: health.ingestion_lag,
                elapsed,
                timeout: config.rollout_pause_timeout,
            })
        }
    }

    /// Check if a pod is ready
    fn is_pod_ready(pod: &Pod) -> bool {
        if let Some(status) = &pod.status {
            if let Some(conditions) = &status.conditions {
                return conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True");
            }
        }
        false
    }

    /// Get StatefulSet update status
    fn get_update_status(statefulset: &StatefulSet) -> UpdateStatus {
        let spec = &statefulset.spec;
        let status = &statefulset.status;

        let desired_replicas = spec.as_ref().map(|s| s.replicas.unwrap_or(1)).unwrap_or(1);
        let updated_replicas = status
            .as_ref()
            .and_then(|s| s.updated_replicas)
            .unwrap_or(0);
        let ready_replicas = status.as_ref().and_then(|s| s.ready_replicas).unwrap_or(0);

        UpdateStatus {
            no_update_in_progress: updated_replicas == desired_replicas && ready_replicas == desired_replicas,
            updated_replicas,
            desired_replicas,
        }
    }

    /// Mark a pod as having passed the rollout gate
    async fn mark_gate_passed(client: &Client, pod: &Pod) -> Result<()> {
        let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
        let name = pod.name_any();

        let mut annotations = pod.annotations().clone();
        annotations.insert(ROLLOUT_GATE_ANNOTATION.to_string(), "passed".to_string());

        let patch = serde_json::json!({
            "metadata": {
                "annotations": annotations
            }
        });

        let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
        pod_api
            .patch(
                &name,
                &PatchParams::apply("rollout-gate"),
                &Patch::Merge(patch),
            )
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }

    /// Emit a Kubernetes event for visibility
    async fn emit_event(
        client: &Client,
        pod: &Pod,
        event_type: &str,
        message: &str,
    ) -> Result<()> {
        let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
        let pod_name = pod.name_any();

        let now = chrono::Utc::now();

        let event = Event {
            metadata: ObjectMeta {
                name: Some(format!("{}.{}", pod_name, now.timestamp())),
                namespace: Some(namespace.clone()),
                ..Default::default()
            },
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                api_version: Some("v1".to_string()),
                kind: Some("Pod".to_string()),
                name: Some(pod_name.clone()),
                namespace: Some(namespace),
                ..Default::default()
            },
            reason: Some(event_type.to_string()),
            message: Some(message.to_string()),
            type_: Some(if event_type.contains("Timeout") || event_type.contains("Failed") {
                "Warning".to_string()
            } else {
                "Normal".to_string()
            }),
            event_time: None,
            first_timestamp: Some(now.into()),
            last_timestamp: Some(now.into()),
            count: Some(1),
            source: Some(EventSource {
                component: Some("rollout-gate".to_string()),
                host: None,
            }),
            series: None,
            action: None,
            related: None,
            reporting_component: Some("stellar-operator".to_string()),
            reporting_instance: Some("rollout-gate".to_string()),
        };

        let event_api: Api<Event> = Api::namespaced(client.clone(), &namespace);
        event_api
            .create(&PostParams::default(), &event)
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }
}

/// Update status information
struct UpdateStatus {
    /// Whether no update is in progress
    no_update_in_progress: bool,

    /// Number of pods already updated
    updated_replicas: i32,

    /// Desired number of replicas
    desired_replicas: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rollout_gate_result_approved() {
        let result = RolloutGateResult::Approved {
            pod_name: "test-pod-0".to_string(),
            ingestion_lag: 1,
        };

        match result {
            RolloutGateResult::Approved {
                pod_name,
                ingestion_lag,
            } => {
                assert_eq!(pod_name, "test-pod-0");
                assert_eq!(ingestion_lag, 1);
            }
            _ => panic!("Expected Approved"),
        }
    }

    #[test]
    fn test_rollout_gate_result_paused() {
        let result = RolloutGateResult::Paused {
            pod_name: "test-pod-0".to_string(),
            ingestion_lag: 100,
            elapsed: Duration::from_secs(30),
            timeout: Duration::from_secs(180),
        };

        match result {
            RolloutGateResult::Paused {
                pod_name,
                ingestion_lag,
                elapsed,
                timeout,
            } => {
                assert_eq!(pod_name, "test-pod-0");
                assert_eq!(ingestion_lag, 100);
                assert_eq!(elapsed, Duration::from_secs(30));
                assert_eq!(timeout, Duration::from_secs(180));
            }
            _ => panic!("Expected Paused"),
        }
    }
}
