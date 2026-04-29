//! Spot/Preemptible Instance Termination Handler
//!
//! Polls cloud-provider metadata endpoints to detect imminent spot/preemptible
//! instance termination. On detection it:
//!
//! 1. Cordons the Kubernetes node (marks it unschedulable).
//! 2. Sets `maintenance_mode: true` on every `StellarNode` running on that node.
//! 3. Evicts all Stellar pods so they reschedule on stable instances.
//!
//! The handler is cloud-agnostic: it probes AWS, GCP, and Azure endpoints
//! concurrently and acts on the first positive signal.
//!
//! # Cloud endpoints
//!
//! | Cloud | URL | Termination signal |
//! |-------|-----|--------------------|
//! | AWS   | `http://169.254.169.254/latest/meta-data/spot/termination-time` | HTTP 200 |
//! | GCP   | `http://metadata.google.internal/computeMetadata/v1/instance/preempted` | body `TRUE` |
//! | Azure | `http://169.254.169.254/metadata/scheduledevents?api-version=2020-07-01` | `Preempt` event |

use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{
    api::{Api, EvictParams, ListParams, Patch, PatchParams},
    runtime::events::{EventType, Recorder, Reporter},
    Client, Resource, ResourceExt,
};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

/// How often to poll the metadata endpoints (cloud providers recommend ≤5 s).
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// HTTP timeout for each metadata probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

// ── Azure scheduled-events response types ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AzureScheduledEvents {
    #[serde(rename = "Events")]
    events: Vec<AzureEvent>,
}

#[derive(Debug, Deserialize)]
struct AzureEvent {
    #[serde(rename = "EventType")]
    event_type: String,
}

// ── SpotDrainHandler ─────────────────────────────────────────────────────────

/// Monitors cloud metadata endpoints and triggers graceful drain on spot termination.
pub struct SpotDrainHandler {
    client: Client,
    reporter: Reporter,
    /// Name of the Kubernetes node this process is running on.
    node_name: String,
    http: reqwest::Client,
}

impl SpotDrainHandler {
    /// Create a new handler.
    ///
    /// `node_name` should be the value of the `spec.nodeName` field for the
    /// operator pod, typically sourced from the `NODE_NAME` env var.
    pub fn new(client: Client, reporter: Reporter, node_name: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            reporter,
            node_name,
            http,
        }
    }

    /// Run the polling loop. Blocks until termination is detected, then drains.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!(
            node = %self.node_name,
            "SpotDrainHandler started – polling cloud metadata endpoints every {}s",
            POLL_INTERVAL.as_secs()
        );

        loop {
            if self.is_terminating().await {
                info!(
                    node = %self.node_name,
                    "Spot/preemptible termination notice received – initiating graceful drain"
                );
                if let Err(e) = self.drain().await {
                    error!(node = %self.node_name, "Drain failed: {}", e);
                }
                // After draining there is nothing more to do; the instance will be
                // reclaimed by the cloud provider shortly.
                return Ok(());
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    // ── Detection ────────────────────────────────────────────────────────────

    /// Returns `true` if any cloud provider signals imminent termination.
    async fn is_terminating(&self) -> bool {
        // Run all probes concurrently; first `true` wins.
        let (aws, gcp, azure) = tokio::join!(
            self.probe_aws(),
            self.probe_gcp(),
            self.probe_azure(),
        );
        aws || gcp || azure
    }

    /// AWS: HTTP 200 on the termination-time endpoint means the instance is
    /// scheduled for termination within ~2 minutes.
    async fn probe_aws(&self) -> bool {
        match self
            .http
            .get("http://169.254.169.254/latest/meta-data/spot/termination-time")
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// GCP: the preempted endpoint returns the string `TRUE` when the VM is
    /// about to be preempted.
    async fn probe_gcp(&self) -> bool {
        match self
            .http
            .get("http://metadata.google.internal/computeMetadata/v1/instance/preempted")
            .header("Metadata-Flavor", "Google")
            .send()
            .await
        {
            Ok(resp) => {
                resp.text()
                    .await
                    .map(|t| t.trim().eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
            }
            Err(_) => false,
        }
    }

    /// Azure: a `Preempt` event in the scheduled-events response means the VM
    /// will be evicted.
    async fn probe_azure(&self) -> bool {
        match self
            .http
            .get("http://169.254.169.254/metadata/scheduledevents?api-version=2020-07-01")
            .header("Metadata", "true")
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(events) = resp.json::<AzureScheduledEvents>().await {
                    events
                        .events
                        .iter()
                        .any(|e| e.event_type.eq_ignore_ascii_case("Preempt"))
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    // ── Drain ────────────────────────────────────────────────────────────────

    /// Cordon the node, set maintenance_mode on all StellarNodes, then evict pods.
    async fn drain(&self) -> Result<()> {
        self.cordon_node().await?;
        self.set_stellar_nodes_maintenance_mode().await?;
        self.evict_stellar_pods().await?;
        Ok(())
    }

    /// Mark the Kubernetes node as unschedulable (cordon).
    async fn cordon_node(&self) -> Result<()> {
        let nodes: Api<Node> = Api::all(self.client.clone());
        let patch = json!({
            "spec": { "unschedulable": true },
            "metadata": {
                "annotations": {
                    "stellar.org/spot-drain": "true",
                    "stellar.org/spot-drain-time": chrono::Utc::now().to_rfc3339(),
                }
            }
        });
        nodes
            .patch(
                &self.node_name,
                &PatchParams::apply("stellar-spot-drain"),
                &Patch::Merge(&patch),
            )
            .await
            .map_err(Error::KubeError)?;
        info!(node = %self.node_name, "Node cordoned for spot drain");
        Ok(())
    }

    /// Set `spec.maintenanceMode = true` on every StellarNode whose pod runs on
    /// this node, so the reconciler knows not to schedule new work here.
    async fn set_stellar_nodes_maintenance_mode(&self) -> Result<()> {
        let stellar_nodes: Api<StellarNode> = Api::all(self.client.clone());
        let all = stellar_nodes
            .list(&ListParams::default())
            .await
            .map_err(Error::KubeError)?;

        for sn in all.items {
            let ns = sn.namespace().unwrap_or_else(|| "default".to_string());
            let name = sn.name_any();

            // Only act on nodes that are actually scheduled on this instance.
            if !self.stellar_node_is_on_this_instance(&sn).await {
                continue;
            }

            let patch = json!({ "spec": { "maintenanceMode": true } });
            let api: Api<StellarNode> = Api::namespaced(self.client.clone(), &ns);
            match api
                .patch(
                    &name,
                    &PatchParams::apply("stellar-spot-drain"),
                    &Patch::Merge(&patch),
                )
                .await
            {
                Ok(_) => info!(
                    node = %self.node_name,
                    stellar_node = %name,
                    "Set maintenanceMode=true on StellarNode"
                ),
                Err(e) => warn!(
                    node = %self.node_name,
                    stellar_node = %name,
                    "Failed to set maintenanceMode: {}", e
                ),
            }
        }
        Ok(())
    }

    /// Returns true if any pod belonging to this StellarNode runs on this node.
    async fn stellar_node_is_on_this_instance(&self, sn: &StellarNode) -> bool {
        let ns = sn.namespace().unwrap_or_else(|| "default".to_string());
        let name = sn.name_any();
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &ns);
        let lp = ListParams::default().labels(&format!(
            "app.kubernetes.io/instance={},app.kubernetes.io/managed-by=stellar-operator",
            name
        ));
        match pods.list(&lp).await {
            Ok(list) => list.items.iter().any(|p| {
                p.spec
                    .as_ref()
                    .and_then(|s| s.node_name.as_deref())
                    == Some(self.node_name.as_str())
            }),
            Err(_) => false,
        }
    }

    /// Evict all Stellar-operator-managed pods on this node.
    async fn evict_stellar_pods(&self) -> Result<()> {
        let pods: Api<Pod> = Api::all(self.client.clone());
        let lp = ListParams::default()
            .fields(&format!("spec.nodeName={}", self.node_name))
            .labels("app.kubernetes.io/managed-by=stellar-operator");

        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let ns = pod.namespace().unwrap_or_else(|| "default".to_string());
            let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), &ns);

            let recorder = Recorder::new(self.client.clone(), self.reporter.clone(), pod.object_ref(&()));

            match pod_api.evict(&pod_name, &EvictParams::default()).await {
                Ok(_) => {
                    info!(pod = %pod_name, namespace = %ns, "Evicted pod during spot drain");
                    let _ = recorder
                        .publish(kube::runtime::events::Event {
                            type_: EventType::Normal,
                            reason: "SpotDrain".into(),
                            note: Some(format!(
                                "Pod evicted: spot/preemptible termination notice received on node {}",
                                self.node_name
                            )),
                            action: "Evicting".into(),
                            secondary: None,
                        })
                        .await;
                }
                Err(e) => warn!(pod = %pod_name, "Failed to evict pod: {}", e),
            }
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Azure JSON parsing ────────────────────────────────────────────────────

    #[test]
    fn test_azure_preempt_event_detected() {
        let json = r#"{"Events":[{"EventType":"Preempt","EventId":"abc","ResourceType":"VirtualMachine","Resources":["vm1"],"EventStatus":"Scheduled","NotBefore":""}]}"#;
        let events: AzureScheduledEvents = serde_json::from_str(json).unwrap();
        assert!(events
            .events
            .iter()
            .any(|e| e.event_type.eq_ignore_ascii_case("Preempt")));
    }

    #[test]
    fn test_azure_no_preempt_event() {
        let json = r#"{"Events":[{"EventType":"Reboot","EventId":"xyz","ResourceType":"VirtualMachine","Resources":["vm1"],"EventStatus":"Scheduled","NotBefore":""}]}"#;
        let events: AzureScheduledEvents = serde_json::from_str(json).unwrap();
        assert!(!events
            .events
            .iter()
            .any(|e| e.event_type.eq_ignore_ascii_case("Preempt")));
    }

    #[test]
    fn test_azure_empty_events() {
        let json = r#"{"Events":[]}"#;
        let events: AzureScheduledEvents = serde_json::from_str(json).unwrap();
        assert!(!events
            .events
            .iter()
            .any(|e| e.event_type.eq_ignore_ascii_case("Preempt")));
    }

    // ── Mock-server based probe tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_probe_aws_terminating() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/latest/meta-data/spot/termination-time",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                "2024-01-01T00:00:00Z",
            ))
            .mount(&server)
            .await;

        let http = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .unwrap();
        let url = format!(
            "{}/latest/meta-data/spot/termination-time",
            server.uri()
        );
        let resp = http.get(&url).send().await.unwrap();
        assert!(resp.status().is_success());
    }

    #[tokio::test]
    async fn test_probe_aws_not_terminating() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/latest/meta-data/spot/termination-time",
            ))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let http = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .unwrap();
        let url = format!(
            "{}/latest/meta-data/spot/termination-time",
            server.uri()
        );
        let resp = http.get(&url).send().await.unwrap();
        assert!(!resp.status().is_success());
    }

    #[tokio::test]
    async fn test_probe_gcp_preempted_true() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/computeMetadata/v1/instance/preempted",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("TRUE"))
            .mount(&server)
            .await;

        let http = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .unwrap();
        let url = format!("{}/computeMetadata/v1/instance/preempted", server.uri());
        let body = http.get(&url).send().await.unwrap().text().await.unwrap();
        assert!(body.trim().eq_ignore_ascii_case("true"));
    }

    #[tokio::test]
    async fn test_probe_gcp_not_preempted() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/computeMetadata/v1/instance/preempted",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("FALSE"))
            .mount(&server)
            .await;

        let http = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .unwrap();
        let url = format!("{}/computeMetadata/v1/instance/preempted", server.uri());
        let body = http.get(&url).send().await.unwrap().text().await.unwrap();
        assert!(!body.trim().eq_ignore_ascii_case("true"));
    }
}
