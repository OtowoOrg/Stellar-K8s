//! Validator key rotation worker and daemon.
//!
//! Rotation is deliberately default-off. When enabled, the daemon only targets
//! validator nodes whose seed source is backed by an external manager that this
//! module knows how to update.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams},
    Client, ResourceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use super::vault::{
    AwsSecretsManagerBackend, AwsSecretsManagerConfig, ManagedSeedSecret, SecretManagerBackend,
    ValidatorSeedMaterial, VaultKv2Backend,
};
use crate::controller::background_jobs::{JobKind, JobRegistry};
use crate::crd::{seed_secret::DEFAULT_SEED_KEY, NodeType, StellarNode};
use crate::error::{Error, Result};

const KEY_ROTATION_JOB_KIND: &str = "validator_key_rotation";
const STELLAR_CORE_HTTP_PORT: u16 = 11626;

/// Operator-level configuration for automated validator key rotation.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorKeyRotationConfig {
    /// Enable the background daemon.
    #[serde(default)]
    pub enabled: bool,
    /// Time between daemon sweeps.
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    /// Consensus validation window after applying a candidate.
    #[serde(default = "default_validation_window_secs")]
    pub validation_window_secs: u64,
    /// Delay between validation samples. A zero-second validation window takes one immediate sample.
    #[serde(default = "default_validation_sample_interval_secs")]
    pub validation_sample_interval_secs: u64,
    /// Minimum authenticated peer count required before and after rotation.
    #[serde(default = "default_min_authenticated_peers")]
    pub min_authenticated_peers: usize,
    /// Number of unhealthy samples tolerated during validation.
    #[serde(default)]
    pub max_unhealthy_samples: u32,
    /// Restore the previous seed automatically if candidate validation fails.
    #[serde(default = "default_rollback_on_failure")]
    pub rollback_on_failure: bool,
    /// AWS region override for Secrets Manager; otherwise the default AWS chain is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_region: Option<String>,
}

impl Default for ValidatorKeyRotationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_interval_secs(),
            validation_window_secs: default_validation_window_secs(),
            validation_sample_interval_secs: default_validation_sample_interval_secs(),
            min_authenticated_peers: default_min_authenticated_peers(),
            max_unhealthy_samples: 0,
            rollback_on_failure: true,
            aws_region: None,
        }
    }
}

fn default_interval_secs() -> u64 {
    86_400
}

fn default_validation_window_secs() -> u64 {
    30
}

fn default_validation_sample_interval_secs() -> u64 {
    5
}

fn default_min_authenticated_peers() -> usize {
    1
}

fn default_rollback_on_failure() -> bool {
    true
}

/// One consensus observation from Stellar Core.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusSnapshot {
    pub ledger_sequence: u64,
    pub ledger_hash: String,
    pub state: String,
    pub authenticated_peer_count: usize,
    pub node_id: Option<String>,
    pub observed_at: DateTime<Utc>,
}

impl ConsensusSnapshot {
    pub fn is_healthy(&self, min_authenticated_peers: usize) -> bool {
        let state = self.state.to_ascii_lowercase();
        let externalized = state.contains("synced") || state.contains("externalize");
        externalized
            && self.ledger_sequence > 0
            && !self.ledger_hash.is_empty()
            && self.authenticated_peer_count >= min_authenticated_peers
    }
}

/// Generates candidate validator seed material.
pub trait SeedGenerator: Send + Sync {
    fn generate_seed(&self) -> Result<ValidatorSeedMaterial>;
}

#[derive(Clone, Debug, Default)]
pub struct StellarSeedGenerator;

impl SeedGenerator for StellarSeedGenerator {
    fn generate_seed(&self) -> Result<ValidatorSeedMaterial> {
        ValidatorSeedMaterial::generate()
    }
}

/// Minimal Stellar Core admin interface needed by the rotation worker.
#[async_trait]
pub trait StellarCoreAdmin: Send + Sync {
    async fn consensus_snapshot(&self) -> Result<ConsensusSnapshot>;
    async fn apply_candidate(&self, candidate: &ManagedSeedSecret) -> Result<()>;
    async fn rollback_to_previous(&self, previous: &ManagedSeedSecret) -> Result<()>;
}

/// HTTP implementation for Stellar Core's local admin API.
#[derive(Clone)]
pub struct HttpStellarCoreAdmin {
    endpoint: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for HttpStellarCoreAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpStellarCoreAdmin")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl HttpStellarCoreAdmin {
    pub fn new(endpoint: impl Into<String>, timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| {
                Error::ConfigError(format!("failed to build Stellar Core admin client: {e}"))
            })?;
        Ok(Self {
            endpoint: endpoint.into(),
            client,
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.endpoint.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    async fn trigger_config_reload(&self) -> Result<()> {
        let url = self.url("/http-command?admin=true&command=config-reload");
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(Error::HttpError)?;

        if !response.status().is_success() {
            return Err(Error::ConfigError(format!(
                "Stellar Core config-reload failed at {}: HTTP {}",
                self.endpoint,
                response.status()
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl StellarCoreAdmin for HttpStellarCoreAdmin {
    async fn consensus_snapshot(&self) -> Result<ConsensusSnapshot> {
        let info_response = self
            .client
            .get(self.url("/info"))
            .send()
            .await
            .map_err(Error::HttpError)?;

        if !info_response.status().is_success() {
            return Err(Error::ConfigError(format!(
                "Stellar Core /info failed at {}: HTTP {}",
                self.endpoint,
                info_response.status()
            )));
        }

        let info: Value = info_response.json().await.map_err(Error::HttpError)?;
        let peer_count = match self.client.get(self.url("/peers")).send().await {
            Ok(response) if response.status().is_success() => {
                let body: Value = response.json().await.map_err(Error::HttpError)?;
                parse_peer_count(&body)
            }
            Ok(response) => {
                warn!(
                    endpoint = %self.endpoint,
                    status = %response.status(),
                    "Stellar Core /peers failed during key-rotation health sample"
                );
                0
            }
            Err(error) => {
                warn!(
                    endpoint = %self.endpoint,
                    error = %error,
                    "Stellar Core /peers unreachable during key-rotation health sample"
                );
                0
            }
        };

        Ok(parse_consensus_snapshot(&info, peer_count))
    }

    async fn apply_candidate(&self, _candidate: &ManagedSeedSecret) -> Result<()> {
        self.trigger_config_reload().await
    }

    async fn rollback_to_previous(&self, _previous: &ManagedSeedSecret) -> Result<()> {
        self.trigger_config_reload().await
    }
}

/// Rotation phase for structured logs and tests.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RotationStage {
    Started,
    Preflight,
    CandidateStored,
    Applied,
    Validation,
    Promoted,
    Rollback,
    Completed,
    Aborted,
    Failed,
}

/// Final rotation outcome.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RotationOutcome {
    Completed,
    RolledBack,
    Aborted,
    Failed,
}

/// Sanitized rotation log entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RotationLogEntry {
    pub timestamp: DateTime<Utc>,
    pub stage: RotationStage,
    pub message: String,
    pub consensus_healthy: Option<bool>,
}

/// Sanitized result returned by a rotation attempt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyRotationReport {
    pub namespace: String,
    pub node_name: String,
    pub backend: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: RotationOutcome,
    pub previous_fingerprint: Option<String>,
    pub candidate_fingerprint: Option<String>,
    pub rollback_performed: bool,
    pub events: Vec<RotationLogEntry>,
}

impl KeyRotationReport {
    fn started(namespace: impl Into<String>, node_name: impl Into<String>, backend: &str) -> Self {
        Self {
            namespace: namespace.into(),
            node_name: node_name.into(),
            backend: backend.to_string(),
            started_at: Utc::now(),
            finished_at: None,
            outcome: RotationOutcome::Failed,
            previous_fingerprint: None,
            candidate_fingerprint: None,
            rollback_performed: false,
            events: Vec::new(),
        }
    }

    fn push(
        &mut self,
        stage: RotationStage,
        message: impl Into<String>,
        consensus_healthy: Option<bool>,
    ) {
        self.events.push(RotationLogEntry {
            timestamp: Utc::now(),
            stage,
            message: message.into(),
            consensus_healthy,
        });
    }

    fn finish(&mut self, outcome: RotationOutcome) {
        self.outcome = outcome;
        self.finished_at = Some(Utc::now());
    }
}

/// Single-node rotation worker.
pub struct KeyRotationWorker {
    namespace: String,
    node_name: String,
    config: ValidatorKeyRotationConfig,
    backend: Arc<dyn SecretManagerBackend>,
    admin: Arc<dyn StellarCoreAdmin>,
    seed_generator: Arc<dyn SeedGenerator>,
}

impl KeyRotationWorker {
    pub fn new(
        namespace: impl Into<String>,
        node_name: impl Into<String>,
        config: ValidatorKeyRotationConfig,
        backend: Arc<dyn SecretManagerBackend>,
        admin: Arc<dyn StellarCoreAdmin>,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            node_name: node_name.into(),
            config,
            backend,
            admin,
            seed_generator: Arc::new(StellarSeedGenerator),
        }
    }

    pub fn with_seed_generator(mut self, seed_generator: Arc<dyn SeedGenerator>) -> Self {
        self.seed_generator = seed_generator;
        self
    }

    pub async fn rotate_once(&self) -> Result<KeyRotationReport> {
        let mut report = KeyRotationReport::started(
            &self.namespace,
            &self.node_name,
            self.backend.backend_name(),
        );
        report.push(
            RotationStage::Started,
            "validator key rotation started",
            None,
        );

        let preflight = self.admin.consensus_snapshot().await?;
        let preflight_healthy = preflight.is_healthy(self.config.min_authenticated_peers);
        report.push(
            RotationStage::Preflight,
            format!(
                "preflight ledger={} peers={} state={}",
                preflight.ledger_sequence, preflight.authenticated_peer_count, preflight.state
            ),
            Some(preflight_healthy),
        );

        if !preflight_healthy {
            report.push(
                RotationStage::Aborted,
                "preflight consensus health check failed; no secret changes applied",
                Some(false),
            );
            report.finish(RotationOutcome::Aborted);
            return Ok(report);
        }

        let previous = self.backend.read_current().await?;
        report.previous_fingerprint = Some(previous.version.fingerprint.clone());

        let candidate_material = self.seed_generator.generate_seed()?;
        report.candidate_fingerprint = Some(candidate_material.fingerprint.clone());
        let candidate = self
            .backend
            .put_candidate(&candidate_material, &previous)
            .await?;
        report.push(
            RotationStage::CandidateStored,
            format!(
                "candidate seed staged with fingerprint {}",
                candidate.version.fingerprint
            ),
            None,
        );

        self.admin.apply_candidate(&candidate).await?;
        report.push(
            RotationStage::Applied,
            "Stellar Core config reload requested for candidate seed",
            None,
        );

        match self
            .validate_consensus_window(&preflight, &mut report)
            .await
        {
            Ok(()) => {
                self.backend
                    .promote_candidate(&candidate, &previous)
                    .await?;
                report.push(
                    RotationStage::Promoted,
                    format!(
                        "candidate seed promoted with fingerprint {}",
                        candidate.version.fingerprint
                    ),
                    None,
                );
                report.push(
                    RotationStage::Completed,
                    "validator key rotation completed without consensus outage",
                    Some(true),
                );
                report.finish(RotationOutcome::Completed);
                Ok(report)
            }
            Err(validation_error) => {
                warn!(
                    node = %self.node_name,
                    namespace = %self.namespace,
                    error = %validation_error,
                    "Candidate key validation failed"
                );
                if self.config.rollback_on_failure {
                    self.backend.rollback(&previous, Some(&candidate)).await?;
                    self.admin.rollback_to_previous(&previous).await?;
                    report.rollback_performed = true;
                    report.push(
                        RotationStage::Rollback,
                        format!(
                            "rolled back to previous fingerprint {} after validation failure",
                            previous.version.fingerprint
                        ),
                        Some(true),
                    );
                    report.finish(RotationOutcome::RolledBack);
                    Ok(report)
                } else {
                    report.push(
                        RotationStage::Failed,
                        format!("candidate validation failed: {validation_error}"),
                        Some(false),
                    );
                    report.finish(RotationOutcome::Failed);
                    Ok(report)
                }
            }
        }
    }

    async fn validate_consensus_window(
        &self,
        preflight: &ConsensusSnapshot,
        report: &mut KeyRotationReport,
    ) -> Result<()> {
        let sample_count = validation_sample_count(
            self.config.validation_window_secs,
            self.config.validation_sample_interval_secs,
        );
        let interval = Duration::from_secs(self.config.validation_sample_interval_secs);
        let mut unhealthy_samples = 0u32;
        let mut last_ledger = preflight.ledger_sequence;

        for sample_index in 0..sample_count {
            if sample_index > 0 && !interval.is_zero() {
                sleep(interval).await;
            }

            let snapshot = self.admin.consensus_snapshot().await?;
            let consensus_healthy = snapshot.is_healthy(self.config.min_authenticated_peers)
                && snapshot.ledger_sequence >= last_ledger;
            if snapshot.ledger_sequence < last_ledger {
                report.push(
                    RotationStage::Validation,
                    format!(
                        "validation sample {} ledger regressed from {} to {}",
                        sample_index + 1,
                        last_ledger,
                        snapshot.ledger_sequence
                    ),
                    Some(false),
                );
            } else {
                report.push(
                    RotationStage::Validation,
                    format!(
                        "validation sample {} ledger={} peers={} state={}",
                        sample_index + 1,
                        snapshot.ledger_sequence,
                        snapshot.authenticated_peer_count,
                        snapshot.state
                    ),
                    Some(consensus_healthy),
                );
                last_ledger = snapshot.ledger_sequence;
            }

            if !consensus_healthy {
                unhealthy_samples += 1;
                if unhealthy_samples > self.config.max_unhealthy_samples {
                    return Err(Error::ValidationError(format!(
                        "consensus health failed during validation window after {unhealthy_samples} unhealthy sample(s)"
                    )));
                }
            }
        }

        Ok(())
    }
}

/// Cluster-wide default-off daemon.
pub struct KeyRotationDaemon {
    client: Client,
    watch_namespace: Option<String>,
    config: ValidatorKeyRotationConfig,
    job_registry: Arc<JobRegistry>,
}

impl KeyRotationDaemon {
    pub fn new(
        client: Client,
        watch_namespace: Option<String>,
        config: ValidatorKeyRotationConfig,
        job_registry: Arc<JobRegistry>,
    ) -> Self {
        Self {
            client,
            watch_namespace,
            config,
            job_registry,
        }
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        if !self.config.enabled {
            info!("Validator key rotation daemon is disabled");
            return Ok(());
        }

        info!(
            interval_secs = self.config.interval_secs,
            validation_window_secs = self.config.validation_window_secs,
            "Starting validator key rotation daemon"
        );

        loop {
            if let Err(error) = self.rotate_configured_nodes().await {
                error!(error = %error, "Validator key rotation daemon sweep failed");
            }
            sleep(Duration::from_secs(self.config.interval_secs)).await;
        }
    }

    async fn rotate_configured_nodes(&self) -> Result<()> {
        let nodes: Api<StellarNode> = match &self.watch_namespace {
            Some(namespace) => Api::namespaced(self.client.clone(), namespace),
            None => Api::all(self.client.clone()),
        };

        for node in nodes
            .list(&ListParams::default())
            .await
            .map_err(Error::KubeError)?
            .items
        {
            if node.spec.node_type != NodeType::Validator {
                continue;
            }

            let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
            let name = node.name_any();
            let Some(backend) = self.backend_for_node(&node).await? else {
                debug!(
                    node = %name,
                    namespace = %namespace,
                    "Skipping validator key rotation; no supported external seed backend"
                );
                continue;
            };
            let Some(admin) = self.admin_for_node(&node).await? else {
                warn!(
                    node = %name,
                    namespace = %namespace,
                    "Skipping validator key rotation; no running validator pod with an IP"
                );
                continue;
            };

            let job = self.job_registry.register(
                format!("validator-key-rotation/{namespace}/{name}"),
                JobKind::Other(KEY_ROTATION_JOB_KIND.to_string()),
                Some(namespace.clone()),
            );
            job.start();

            let worker =
                KeyRotationWorker::new(&namespace, &name, self.config.clone(), backend, admin);
            match worker.rotate_once().await {
                Ok(report) if report.outcome == RotationOutcome::Completed => {
                    info!(
                        node = %name,
                        namespace = %namespace,
                        backend = %report.backend,
                        candidate_fingerprint = ?report.candidate_fingerprint,
                        "Validator key rotation completed"
                    );
                    job.succeed();
                }
                Ok(report) => {
                    warn!(
                        node = %name,
                        namespace = %namespace,
                        outcome = ?report.outcome,
                        rollback_performed = report.rollback_performed,
                        "Validator key rotation did not promote candidate"
                    );
                    job.fail(format!("{:?}", report.outcome));
                }
                Err(error) => {
                    error!(
                        node = %name,
                        namespace = %namespace,
                        error = %error,
                        "Validator key rotation failed"
                    );
                    job.fail(error.to_string());
                }
            }
        }

        Ok(())
    }

    async fn backend_for_node(
        &self,
        node: &StellarNode,
    ) -> Result<Option<Arc<dyn SecretManagerBackend>>> {
        let source = node
            .spec
            .validator_config
            .as_ref()
            .and_then(|vc| vc.resolve_seed_source());
        let Some(source) = source else {
            return Ok(None);
        };

        if let Some(vault) = source.vault_ref {
            let seed_field = vault.secret_key.unwrap_or_else(|| "seed".to_string());
            let backend = VaultKv2Backend::from_env(vault.secret_path, seed_field)?;
            return Ok(Some(Arc::new(backend)));
        }

        if let Some(external) = source.external_ref {
            let is_aws = external.remote_key.starts_with("arn:aws:secretsmanager:")
                || external
                    .secret_store_ref
                    .name
                    .to_ascii_lowercase()
                    .contains("aws");
            if is_aws {
                let remote_property = external.remote_property;
                let backend =
                    AwsSecretsManagerBackend::from_default_config(AwsSecretsManagerConfig {
                        secret_id: external.remote_key,
                        seed_field: remote_property
                            .clone()
                            .unwrap_or_else(|| DEFAULT_SEED_KEY.to_string()),
                        store_json: remote_property.is_some(),
                        region: self.config.aws_region.clone(),
                    })
                    .await;
                return Ok(Some(Arc::new(backend)));
            }
        }

        Ok(None)
    }

    async fn admin_for_node(
        &self,
        node: &StellarNode,
    ) -> Result<Option<Arc<dyn StellarCoreAdmin>>> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let name = node.name_any();
        let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), &namespace);
        let label_selector =
            format!("app.kubernetes.io/instance={name},stellar.org/node-type=validator");
        let pods = pod_api
            .list(&ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;
        let pod_ip = pods
            .items
            .iter()
            .filter_map(|pod| pod.status.as_ref())
            .filter_map(|status| status.pod_ip.as_deref())
            .next();
        let Some(pod_ip) = pod_ip else {
            return Ok(None);
        };
        let endpoint = format!("http://{pod_ip}:{STELLAR_CORE_HTTP_PORT}");
        Ok(Some(Arc::new(HttpStellarCoreAdmin::new(
            endpoint,
            Duration::from_secs(10),
        )?)))
    }
}

fn validation_sample_count(window_secs: u64, interval_secs: u64) -> u32 {
    if window_secs == 0 || interval_secs == 0 {
        return 1;
    }
    window_secs.div_ceil(interval_secs).max(1) as u32
}

fn parse_consensus_snapshot(info: &Value, authenticated_peer_count: usize) -> ConsensusSnapshot {
    ConsensusSnapshot {
        ledger_sequence: info
            .pointer("/info/ledger/num")
            .and_then(Value::as_u64)
            .or_else(|| info.pointer("/ledger/num").and_then(Value::as_u64))
            .unwrap_or(0),
        ledger_hash: info
            .pointer("/info/ledger/hash")
            .and_then(Value::as_str)
            .or_else(|| info.pointer("/ledger/hash").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        state: info
            .pointer("/info/state")
            .and_then(Value::as_str)
            .or_else(|| info.get("state").and_then(Value::as_str))
            .unwrap_or("Unknown")
            .to_string(),
        node_id: info
            .pointer("/info/node_id")
            .and_then(Value::as_str)
            .or_else(|| info.pointer("/info/nodeID").and_then(Value::as_str))
            .or_else(|| info.pointer("/info/nodeId").and_then(Value::as_str))
            .or_else(|| info.get("node_id").and_then(Value::as_str))
            .map(str::to_string),
        authenticated_peer_count,
        observed_at: Utc::now(),
    }
}

fn parse_peer_count(peers: &Value) -> usize {
    peers
        .get("authenticated_peers")
        .and_then(Value::as_array)
        .or_else(|| peers.get("peers").and_then(Value::as_array))
        .map(Vec::len)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::security::vault::MemorySecretManager;
    use std::collections::VecDeque;
    use tokio::sync::Mutex;

    #[derive(Clone)]
    struct FixedSeedGenerator {
        material: ValidatorSeedMaterial,
    }

    impl SeedGenerator for FixedSeedGenerator {
        fn generate_seed(&self) -> Result<ValidatorSeedMaterial> {
            Ok(self.material.clone())
        }
    }

    #[derive(Clone, Default)]
    struct TestAdmin {
        snapshots: Arc<Mutex<VecDeque<ConsensusSnapshot>>>,
        applied: Arc<Mutex<u32>>,
        rollbacks: Arc<Mutex<u32>>,
    }

    impl TestAdmin {
        fn new(snapshots: Vec<ConsensusSnapshot>) -> Self {
            Self {
                snapshots: Arc::new(Mutex::new(snapshots.into())),
                applied: Arc::new(Mutex::new(0)),
                rollbacks: Arc::new(Mutex::new(0)),
            }
        }
    }

    #[async_trait]
    impl StellarCoreAdmin for TestAdmin {
        async fn consensus_snapshot(&self) -> Result<ConsensusSnapshot> {
            self.snapshots
                .lock()
                .await
                .pop_front()
                .ok_or_else(|| Error::InternalError("test snapshot script exhausted".to_string()))
        }

        async fn apply_candidate(&self, _candidate: &ManagedSeedSecret) -> Result<()> {
            *self.applied.lock().await += 1;
            Ok(())
        }

        async fn rollback_to_previous(&self, _previous: &ManagedSeedSecret) -> Result<()> {
            *self.rollbacks.lock().await += 1;
            Ok(())
        }
    }

    fn healthy(sequence: u64) -> ConsensusSnapshot {
        ConsensusSnapshot {
            ledger_sequence: sequence,
            ledger_hash: format!("hash-{sequence}"),
            state: "Synced!".to_string(),
            authenticated_peer_count: 3,
            node_id: Some("GVALIDATOR".to_string()),
            observed_at: Utc::now(),
        }
    }

    fn unhealthy(sequence: u64) -> ConsensusSnapshot {
        ConsensusSnapshot {
            state: "Catching up".to_string(),
            authenticated_peer_count: 0,
            ..healthy(sequence)
        }
    }

    fn test_config() -> ValidatorKeyRotationConfig {
        ValidatorKeyRotationConfig {
            enabled: true,
            interval_secs: 60,
            validation_window_secs: 0,
            validation_sample_interval_secs: 0,
            min_authenticated_peers: 1,
            max_unhealthy_samples: 0,
            rollback_on_failure: true,
            aws_region: None,
        }
    }

    #[tokio::test]
    async fn promotes_candidate_when_consensus_stays_healthy() {
        let previous = ValidatorSeedMaterial::generate().unwrap();
        let candidate = ValidatorSeedMaterial::generate().unwrap();
        let candidate_seed = candidate.secret_seed().to_string();
        let backend = Arc::new(MemorySecretManager::new(previous));
        let admin = Arc::new(TestAdmin::new(vec![healthy(100), healthy(101)]));
        let worker = KeyRotationWorker::new(
            "stellar",
            "validator-a",
            test_config(),
            backend.clone(),
            admin.clone(),
        )
        .with_seed_generator(Arc::new(FixedSeedGenerator {
            material: candidate.clone(),
        }));

        let report = worker.rotate_once().await.unwrap();

        assert_eq!(report.outcome, RotationOutcome::Completed);
        assert!(!report.rollback_performed);
        assert_eq!(
            backend.current().await.material.public_key,
            candidate.public_key
        );
        assert_eq!(*admin.applied.lock().await, 1);
        assert_eq!(*admin.rollbacks.lock().await, 0);
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains(&candidate_seed));
        assert!(report
            .events
            .iter()
            .any(|event| event.stage == RotationStage::Completed));
    }

    #[tokio::test]
    async fn rolls_back_candidate_when_validation_fails() {
        let previous = ValidatorSeedMaterial::generate().unwrap();
        let previous_public_key = previous.public_key.clone();
        let candidate = ValidatorSeedMaterial::generate().unwrap();
        let backend = Arc::new(MemorySecretManager::new(previous));
        let admin = Arc::new(TestAdmin::new(vec![healthy(100), unhealthy(101)]));
        let worker = KeyRotationWorker::new(
            "stellar",
            "validator-a",
            test_config(),
            backend.clone(),
            admin.clone(),
        )
        .with_seed_generator(Arc::new(FixedSeedGenerator {
            material: candidate,
        }));

        let report = worker.rotate_once().await.unwrap();

        assert_eq!(report.outcome, RotationOutcome::RolledBack);
        assert!(report.rollback_performed);
        assert_eq!(
            backend.current().await.material.public_key,
            previous_public_key
        );
        assert_eq!(*admin.applied.lock().await, 1);
        assert_eq!(*admin.rollbacks.lock().await, 1);
    }

    #[tokio::test]
    async fn aborts_before_secret_write_when_preflight_is_unhealthy() {
        let previous = ValidatorSeedMaterial::generate().unwrap();
        let previous_public_key = previous.public_key.clone();
        let candidate = ValidatorSeedMaterial::generate().unwrap();
        let backend = Arc::new(MemorySecretManager::new(previous));
        let admin = Arc::new(TestAdmin::new(vec![unhealthy(100)]));
        let worker = KeyRotationWorker::new(
            "stellar",
            "validator-a",
            test_config(),
            backend.clone(),
            admin.clone(),
        )
        .with_seed_generator(Arc::new(FixedSeedGenerator {
            material: candidate,
        }));

        let report = worker.rotate_once().await.unwrap();

        assert_eq!(report.outcome, RotationOutcome::Aborted);
        assert!(!report.rollback_performed);
        assert_eq!(
            backend.current().await.material.public_key,
            previous_public_key
        );
        assert!(backend.audit().await.is_empty());
        assert_eq!(*admin.applied.lock().await, 0);
    }

    #[test]
    fn parses_stellar_core_info_and_peers() {
        let info = serde_json::json!({
            "info": {
                "ledger": { "num": 123, "hash": "abc" },
                "state": "Synced!",
                "node_id": "GA"
            }
        });
        let peers = serde_json::json!({
            "authenticated_peers": [{ "id": "GB" }, { "id": "GC" }]
        });

        let snapshot = parse_consensus_snapshot(&info, parse_peer_count(&peers));

        assert_eq!(snapshot.ledger_sequence, 123);
        assert_eq!(snapshot.authenticated_peer_count, 2);
        assert!(snapshot.is_healthy(1));
    }

    #[test]
    fn validation_window_zero_uses_one_sample() {
        assert_eq!(validation_sample_count(0, 5), 1);
        assert_eq!(validation_sample_count(30, 5), 6);
        assert_eq!(validation_sample_count(31, 5), 7);
    }
}
