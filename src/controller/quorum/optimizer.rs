//! Quorum set optimization background worker
//!
//! Analyzes peer performance and automatically suggests or applies quorum set updates
//! to improve network latency and maintain consensus health.

use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    runtime::events::{EventType, Recorder, Reporter},
    Client, Resource, ResourceExt,
};
use tracing::{debug, error, info, instrument, warn};

use super::analyzer::{QuorumAnalyzer, QuorumSetRecommendation};
use crate::crd::{NodeType, QuorumOptimizationMode, StellarNode};
use crate::error::{Error, Result};

/// QuorumOptimizer manages the lifecycle of quorum set optimization for Stellar validators.
pub struct QuorumOptimizer {
    client: Client,
    reporter: Reporter,
    analyzer: Arc<tokio::sync::Mutex<QuorumAnalyzer>>,
}

impl QuorumOptimizer {
    pub fn new(client: Client, reporter: Reporter) -> Self {
        // Initialize with a default timeout and window size for measurements
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 50, 3);
        Self {
            client,
            reporter,
            analyzer: Arc::new(tokio::sync::Mutex::new(analyzer)),
        }
    }

    /// Start the quorum optimization background worker
    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("Starting Quorum Optimization background worker");

        loop {
            if let Err(e) = self.optimize_all_nodes().await {
                error!("Error during quorum optimization cycle: {}", e);
            }

            // Sleep for a base interval, individual nodes have their own intervals
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    }

    /// Perform optimization for all applicable StellarNodes
    async fn optimize_all_nodes(&self) -> Result<()> {
        let nodes: Api<StellarNode> = Api::all(self.client.clone());
        let lp = ListParams::default();
        let node_list = nodes.list(&lp).await.map_err(Error::KubeError)?;

        for node in node_list.items {
            if node.spec.node_type == NodeType::Validator {
                if let Some(config) = node
                    .spec
                    .validator_config
                    .as_ref()
                    .and_then(|c| c.quorum_optimization.as_ref())
                {
                    if config.enabled {
                        self.optimize_node(&node, config).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Analyze and optimize a single node's quorum set
    #[instrument(skip(self, node, config), fields(node = %node.name_any()))]
    async fn optimize_node(
        &self,
        node: &StellarNode,
        config: &crate::crd::types::QuorumOptimizationConfig,
    ) -> Result<()> {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        // Get pod IPs for this validator
        let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), &namespace);
        let label_selector = format!(
            "app.kubernetes.io/instance={},stellar.org/node-type=validator",
            name
        );
        let pods = pod_api
            .list(&ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        let pod_ips: Vec<String> = pods
            .items
            .iter()
            .filter_map(|p| p.status.as_ref().and_then(|s| s.pod_ip.as_ref()))
            .cloned()
            .collect();

        if pod_ips.is_empty() {
            debug!(
                "No running pods found for validator {}, skipping optimization",
                name
            );
            return Ok(());
        }

        // Run analysis
        let mut analyzer = self.analyzer.lock().await;
        match analyzer.analyze_quorum(pod_ips).await {
            Ok(result) => {
                if let Some(recommendation) = result.recommendation {
                    info!(
                        "Quorum optimization recommended for {}: {}",
                        name, recommendation.message
                    );

                    match config.mode {
                        QuorumOptimizationMode::Auto => {
                            self.apply_recommendation(node, recommendation).await?;
                        }
                        QuorumOptimizationMode::Manual => {
                            self.suggest_recommendation(node, recommendation).await?;
                        }
                    }
                } else {
                    debug!("No quorum optimization recommended for {}", name);
                }
            }
            Err(e) => {
                warn!("Failed to analyze quorum for {}: {}", name, e);
            }
        }

        Ok(())
    }

    /// Apply the recommendation by patching the CRD
    async fn apply_recommendation(
        &self,
        node: &StellarNode,
        recommendation: QuorumSetRecommendation,
    ) -> Result<()> {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let nodes: Api<StellarNode> = Api::namespaced(self.client.clone(), &namespace);

        info!("Automatically applying quorum optimization for {}", name);

        // Convert recommendation to TOML (simplified for now, ideally we'd use a TOML builder)
        let mut qset_toml = format!("THRESHOLD={}\n", recommendation.recommended_threshold);
        qset_toml.push_str("VALIDATORS=[\n");
        for v in recommendation.recommended_validators {
            qset_toml.push_str(&format!("  \"{}\",\n", v));
        }
        qset_toml.push_str("]\n");

        let patch = serde_json::json!({
            "spec": {
                "validatorConfig": {
                    "quorumSet": qset_toml
                }
            }
        });

        nodes
            .patch(
                &name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Merge(&patch),
            )
            .await
            .map_err(Error::KubeError)?;

        self.emit_optimization_event(node, "QuorumOptimizationApplied", &recommendation.message)
            .await?;

        Ok(())
    }

    /// Suggest the recommendation by emitting an event and updating status
    async fn suggest_recommendation(
        &self,
        node: &StellarNode,
        recommendation: QuorumSetRecommendation,
    ) -> Result<()> {
        info!(
            "Suggesting quorum optimization for {}: {}",
            node.name_any(),
            recommendation.message
        );

        self.emit_optimization_event(node, "QuorumOptimizationSuggested", &recommendation.message)
            .await?;

        // We could also update the status with the recommendation, but for now events are enough.
        Ok(())
    }

    /// Emit a Kubernetes event for the optimization action
    async fn emit_optimization_event(
        &self,
        node: &StellarNode,
        reason: &str,
        message: &str,
    ) -> Result<()> {
        let recorder = Recorder::new(
            self.client.clone(),
            self.reporter.clone(),
            node.object_ref(&()),
        );

        recorder
            .publish(kube::runtime::events::Event {
                type_: EventType::Normal,
                reason: reason.into(),
                note: Some(message.to_string()),
                action: "QuorumOptimization".into(),
                secondary: None,
            })
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }
}
