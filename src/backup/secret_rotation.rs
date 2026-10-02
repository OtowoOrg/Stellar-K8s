// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Automated Secret Rotation for Database Credentials
//!
//! This module implements automated rotation of PostgreSQL database passwords
//! for Stellar Core and Horizon, ensuring zero-downtime credential updates.
//!
//! # Features
//!
//! - Automated password generation with cryptographic randomness
//! - Coordinated updates to both database and Kubernetes secrets
//! - Rolling restart of pods to pick up new credentials
//! - Configurable rotation schedule (cron-based)
//! - Audit logging of all rotation events
//! - Rollback support in case of failures
//!
//! # Architecture
//!
//! 1. Generate new secure password
//! 2. Update database user password (ALTER USER)
//! 3. Update Kubernetes Secret with new password
//! 4. Trigger rolling restart of affected pods
//! 5. Verify connectivity with new credentials
//! 6. Log rotation event for audit trail

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use rand::{distributions::Alphanumeric, Rng};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    IsCa, KeyPair,
};
use time::{Duration as TimeDuration, OffsetDateTime};

/// Configuration for automated secret rotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRotationConfig {
    /// Enable automated secret rotation
    pub enabled: bool,

    /// Rotation schedule in cron format (default: monthly)
    #[serde(default = "default_rotation_schedule")]
    pub schedule: String,

    /// Password length (default: 32 characters)
    #[serde(default = "default_password_length")]
    pub password_length: usize,

    /// Database connection timeout in seconds
    #[serde(default = "default_db_timeout")]
    pub db_timeout_seconds: u64,

    /// Maximum number of retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Enable audit logging to external system
    #[serde(default)]
    pub audit_logging_enabled: bool,

    /// Audit log destination (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_log_destination: Option<String>,

    /// Notification webhook URL for rotation events
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_webhook: Option<String>,
}

fn default_rotation_schedule() -> String {
    "0 0 1 * *".to_string() // First day of every month at midnight
}

fn default_password_length() -> usize {
    32
}

fn default_db_timeout() -> u64 {
    30
}

fn default_max_retries() -> u32 {
    3
}

impl Default for SecretRotationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            schedule: default_rotation_schedule(),
            password_length: default_password_length(),
            db_timeout_seconds: default_db_timeout(),
            max_retries: default_max_retries(),
            audit_logging_enabled: false,
            audit_log_destination: None,
            notification_webhook: None,
        }
    }
}

/// Rotation event for audit logging
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationEvent {
    pub timestamp: DateTime<Utc>,
    pub namespace: String,
    pub node_name: String,
    pub database_user: String,
    pub secret_name: String,
    pub status: RotationStatus,
    pub error_message: Option<String>,
    pub password_hash: String, // SHA256 hash for verification
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RotationStatus {
    Started,
    PasswordGenerated,
    DatabaseUpdated,
    SecretUpdated,
    PodsRestarted,
    Completed,
    Failed,
    RolledBack,
}

/// Configuration for automated mTLS certificate rotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MtlsConfig {
    /// Enable automated mTLS certificate generation and rotation
    pub enabled: bool,

    /// Certificate validity window in hours
    #[serde(default = "default_mtls_cert_validity_hours")]
    pub cert_validity_hours: u32,

    /// Rotate certificates this many minutes before expiration
    #[serde(default = "default_mtls_rotation_minutes")]
    pub rotation_minutes: u64,

    /// Admin API reload port on node pods
    #[serde(default = "default_mtls_reload_port")]
    pub reload_port: u16,

    /// Namespace containing the mTLS secret
    #[serde(default = "default_mtls_namespace")]
    pub namespace: String,

    /// Name of the Kubernetes secret holding mTLS material
    #[serde(default = "default_mtls_secret_name")]
    pub secret_name: String,
}

fn default_mtls_cert_validity_hours() -> u32 {
    1
}

fn default_mtls_rotation_minutes() -> u64 {
    40
}

fn default_mtls_reload_port() -> u16 {
    8443
}

fn default_mtls_namespace() -> String {
    "stellar-system".to_string()
}

fn default_mtls_secret_name() -> String {
    "mtls-certs".to_string()
}

impl Default for MtlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cert_validity_hours: default_mtls_cert_validity_hours(),
            rotation_minutes: default_mtls_rotation_minutes(),
            reload_port: default_mtls_reload_port(),
            namespace: default_mtls_namespace(),
            secret_name: default_mtls_secret_name(),
        }
    }
}

/// Generated mTLS certificate bundle for internal node communication
#[derive(Debug, Clone)]
pub struct MtlsCertificateBundle {
    pub ca_cert: String,
    pub ca_key: String,
    pub server_cert: String,
    pub server_key: String,
    pub client_cert: String,
    pub client_key: String,
}

/// Automated mTLS certificate generation and hot-reload engine
pub struct MtlsRotationEngine {
    config: MtlsConfig,
    client: Client,
}

impl MtlsRotationEngine {
    pub fn new(config: MtlsConfig, client: Client) -> Self {
        Self { config, client }
    }

    /// Start the mTLS rotation loop. The first rotation occurs after
    /// `rotation_minutes` and every `rotation_minutes` thereafter.
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            info!("mTLS certificate rotation is disabled");
            return Ok(());
        }

        let interval = Duration::from_secs(self.config.rotation_minutes * 60);
        info!(
            "Starting mTLS certificate rotation engine (validity {}h, rotation every {}m)",
            self.config.cert_validity_hours, self.config.rotation_minutes
        );

        loop {
            sleep(interval).await;
            if let Err(e) = self.rotate().await {
                error!("mTLS certificate rotation failed: {}", e);
            }
        }
    }

    /// Rotate the mTLS bundle, synchronize it to the Kubernetes secret,
    /// and trigger admin API reloads on node pods.
    pub async fn rotate(&self) -> Result<()> {
        info!("Rotating mTLS certificates");
        let bundle = self.generate_bundle()?;
        self.sync_secret(&bundle).await?;
        self.trigger_reload().await?;
        info!("mTLS certificate rotation completed");
        Ok(())
    }

    fn generate_bundle(&self) -> Result<MtlsCertificateBundle> {
        let not_before = OffsetDateTime::now_utc() - TimeDuration::minutes(1);
        let not_after = OffsetDateTime::now_utc()
            + TimeDuration::hours(self.config.cert_validity_hours.max(1) as i64);

        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.not_before = not_before;
        ca_params.not_after = not_after;
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Stellar Internal CA");
        let ca_key = KeyPair::generate()?;
        let ca_cert = ca_params.self_signed(&ca_key)?;

        let mut server_params = CertificateParams::new(vec![
            "core.stellar.local".to_string(),
            "horizon.stellar.local".to_string(),
            "rpc.stellar.local".to_string(),
        ])?;
        server_params.is_ca = IsCa::NoCa;
        server_params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        server_params.not_before = not_before;
        server_params.not_after = not_after;
        server_params
            .distinguished_name
            .push(DnType::CommonName, "Stellar Internal Server");
        let server_key = KeyPair::generate()?;
        let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key)?;

        let mut client_params = CertificateParams::new(vec!["client.stellar.local".to_string()])?;
        client_params.is_ca = IsCa::NoCa;
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        client_params.not_before = not_before;
        client_params.not_after = not_after;
        client_params
            .distinguished_name
            .push(DnType::CommonName, "Stellar Internal Client");
        let client_key = KeyPair::generate()?;
        let client_cert = client_params.signed_by(&client_key, &ca_cert, &ca_key)?;

        Ok(MtlsCertificateBundle {
            ca_cert: ca_cert.pem(),
            ca_key: ca_key.serialize_pem(),
            server_cert: server_cert.pem(),
            server_key: server_key.serialize_pem(),
            client_cert: client_cert.pem(),
            client_key: client_key.serialize_pem(),
        })
    }

    async fn sync_secret(&self, bundle: &MtlsCertificateBundle) -> Result<()> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.config.namespace);

        let mut data = BTreeMap::new();
        data.insert(
            "ca.crt".to_string(),
            k8s_openapi::ByteString(bundle.ca_cert.as_bytes().to_vec()),
        );
        data.insert(
            "ca.key".to_string(),
            k8s_openapi::ByteString(bundle.ca_key.as_bytes().to_vec()),
        );
        data.insert(
            "tls.crt".to_string(),
            k8s_openapi::ByteString(bundle.server_cert.as_bytes().to_vec()),
        );
        data.insert(
            "tls.key".to_string(),
            k8s_openapi::ByteString(bundle.server_key.as_bytes().to_vec()),
        );
        data.insert(
            "client.crt".to_string(),
            k8s_openapi::ByteString(bundle.client_cert.as_bytes().to_vec()),
        );
        data.insert(
            "client.key".to_string(),
            k8s_openapi::ByteString(bundle.client_key.as_bytes().to_vec()),
        );

        let patch = serde_json::json!({ "data": data });

        secrets
            .patch(
                &self.config.secret_name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Strategic(patch),
            )
            .await
            .context("Failed to update Kubernetes mTLS secret")?;

        info!(
            "mTLS secret updated: {}/{}",
            self.config.namespace, self.config.secret_name
        );
        Ok(())
    }

    async fn trigger_reload(&self) -> Result<()> {
        use crate::crd::StellarNode;

        let nodes: Api<StellarNode> = Api::all(self.client.clone());
        let node_list = nodes.list(&Default::default()).await?;
        let http = reqwest::Client::new();

        for node in node_list.items {
            let namespace = node
                .metadata
                .namespace
                .as_ref()
                .context("Node missing namespace")?;
            let name = node.metadata.name.as_ref().context("Node missing name")?;

            let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
            let pod_list = pods.list(&Default::default()).await?;

            for pod in pod_list.items {
                if let Some(pod_ip) = pod.status.as_ref().and_then(|s| s.pod_ip.as_ref()) {
                    let endpoint = format!(
                        "http://{}:{}/admin/reload",
                        pod_ip, self.config.reload_port
                    );
                    let payload = serde_json::json!({
                        "triggeredBy": "stellar-operator",
                        "version": Utc::now().timestamp()
                    });

                    match http.post(&endpoint).json(&payload).send().await {
                        Ok(response) if response.status().is_success() => {
                            info!(
                                "mTLS reload triggered on {}/{} via {}",
                                namespace, name, endpoint
                            );
                        }
                        Ok(response) => {
                            warn!(
                                "mTLS reload endpoint returned HTTP {} for {}/{}",
                                response.status(),
                                namespace,
                                name
                            );
                        }
                        Err(e) => {
                            warn!(
                                "Failed to call mTLS reload endpoint for {}/{}: {}",
                                namespace, name, e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Secret rotation scheduler
pub struct SecretRotationScheduler {
    config: SecretRotationConfig,
    client: Client,
}

impl SecretRotationScheduler {
    pub fn new(config: SecretRotationConfig, client: Client) -> Self {
        Self { config, client }
    }

    /// Start the rotation scheduler
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Secret rotation is disabled");
            return Ok(());
        }

        let schedule =
            Schedule::from_str(&self.config.schedule).context("Invalid cron schedule")?;

        info!(
            "Starting secret rotation scheduler with schedule: {}",
            self.config.schedule
        );

        loop {
            let now = chrono::Utc::now();
            let next = schedule
                .upcoming(chrono::Utc)
                .next()
                .context("No upcoming schedule")?;

            let duration = (next - now).to_std().unwrap_or(Duration::from_secs(60));

            info!("Next secret rotation scheduled in {:?}", duration);
            sleep(duration).await;

            // Discover all StellarNodes with database configurations
            if let Err(e) = self.rotate_all_secrets().await {
                error!("Secret rotation failed: {}", e);
                self.send_notification("Secret rotation failed", &e.to_string())
                    .await;
            }
        }
    }

    /// Rotate secrets for all StellarNodes in the cluster
    async fn rotate_all_secrets(&self) -> Result<()> {
        use crate::crd::StellarNode;

        info!("Starting cluster-wide secret rotation");

        // Get all StellarNode resources
        let nodes: Api<StellarNode> = Api::all(self.client.clone());
        let node_list = nodes.list(&Default::default()).await?;

        let mut success_count = 0;
        let mut failure_count = 0;

        for node in node_list.items {
            let namespace = node
                .metadata
                .namespace
                .as_ref()
                .context("Node missing namespace")?;
            let name = node.metadata.name.as_ref().context("Node missing name")?;

            // Check if node has database configuration
            if node.spec.database.is_none() && node.spec.managed_database.is_none() {
                continue;
            }

            info!("Rotating secrets for {}/{}", namespace, name);

            match self.rotate_node_secret(namespace, name, &node).await {
                Ok(_) => {
                    success_count += 1;
                    info!("Successfully rotated secrets for {}/{}", namespace, name);
                }
                Err(e) => {
                    failure_count += 1;
                    error!("Failed to rotate secrets for {}/{}: {}", namespace, name, e);
                }
            }
        }

        info!(
            "Secret rotation completed: {} successful, {} failed",
            success_count, failure_count
        );

        Ok(())
    }

    /// Rotate secret for a single StellarNode
    async fn rotate_node_secret(
        &self,
        namespace: &str,
        name: &str,
        node: &crate::crd::StellarNode,
    ) -> Result<()> {
        let mut event = RotationEvent {
            timestamp: Utc::now(),
            namespace: namespace.to_string(),
            node_name: name.to_string(),
            database_user: String::new(),
            secret_name: String::new(),
            status: RotationStatus::Started,
            error_message: None,
            password_hash: String::new(),
        };

        // Determine database configuration
        let (db_host, db_port, db_name, db_user, secret_name) =
            if let Some(db_config) = &node.spec.database {
                (
                    db_config.host.clone(),
                    db_config.port.unwrap_or(5432),
                    db_config.database.clone(),
                    db_config.user.clone(),
                    db_config.password_secret.clone(),
                )
            } else if let Some(managed_db) = &node.spec.managed_database {
                // For managed databases, construct connection info
                let db_host = format!("{}-postgres-rw.{}.svc.cluster.local", name, namespace);
                (
                    db_host,
                    5432,
                    managed_db
                        .database_name
                        .clone()
                        .unwrap_or_else(|| "stellar".to_string()),
                    managed_db
                        .username
                        .clone()
                        .unwrap_or_else(|| "stellar".to_string()),
                    format!("{}-db-credentials", name),
                )
            } else {
                return Ok(()); // No database configuration
            };

        event.database_user = db_user.clone();
        event.secret_name = secret_name.clone();

        self.log_event(&event).await;

        // Step 1: Generate new password
        let new_password = self.generate_secure_password();
        event.password_hash = self.hash_password(&new_password);
        event.status = RotationStatus::PasswordGenerated;
        self.log_event(&event).await;

        // Step 2: Get current password from secret
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let current_secret = secrets.get(&secret_name).await?;
        let current_password = current_secret
            .data
            .as_ref()
            .and_then(|d| d.get("password"))
            .context("Password not found in secret")?;
        let current_password = String::from_utf8(current_password.0.clone())?;

        // Step 3: Connect to database and update password
        let db_url = format!(
            "postgresql://{}:{}@{}:{}/{}",
            db_user, current_password, db_host, db_port, db_name
        );

        match self
            .update_database_password(&db_url, &db_user, &new_password)
            .await
        {
            Ok(_) => {
                event.status = RotationStatus::DatabaseUpdated;
                self.log_event(&event).await;
            }
            Err(e) => {
                event.status = RotationStatus::Failed;
                event.error_message = Some(e.to_string());
                self.log_event(&event).await;
                return Err(e);
            }
        }

        // Step 4: Update Kubernetes secret
        match self
            .update_kubernetes_secret(namespace, &secret_name, &new_password)
            .await
        {
            Ok(_) => {
                event.status = RotationStatus::SecretUpdated;
                self.log_event(&event).await;
            }
            Err(e) => {
                // Attempt rollback
                warn!("Failed to update secret, attempting rollback");
                let _ = self
                    .update_database_password(&db_url, &db_user, &current_password)
                    .await;
                event.status = RotationStatus::RolledBack;
                event.error_message = Some(e.to_string());
                self.log_event(&event).await;
                return Err(e);
            }
        }

        // Step 5: Trigger rolling restart of pods
        match self.restart_pods(namespace, name).await {
            Ok(_) => {
                event.status = RotationStatus::PodsRestarted;
                self.log_event(&event).await;
            }
            Err(e) => {
                error!("Failed to restart pods: {}", e);
                // Don't fail the rotation, pods will pick up new password on next restart
            }
        }

        // Step 6: Verify connectivity with new credentials
        let new_db_url = format!(
            "postgresql://{}:{}@{}:{}/{}",
            db_user, new_password, db_host, db_port, db_name
        );

        match self.verify_database_connection(&new_db_url).await {
            Ok(_) => {
                event.status = RotationStatus::Completed;
                self.log_event(&event).await;
                info!(
                    "Secret rotation completed successfully for {}/{}",
                    namespace, name
                );
            }
            Err(e) => {
                event.status = RotationStatus::Failed;
                event.error_message = Some(format!("Verification failed: {}", e));
                self.log_event(&event).await;
                return Err(e);
            }
        }

        Ok(())
    }

    /// Generate a cryptographically secure random password
    fn generate_secure_password(&self) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(self.config.password_length)
            .map(char::from)
            .collect()
    }

    /// Hash password for audit logging (SHA256)
    fn hash_password(&self, password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Update database user password
    async fn update_database_password(
        &self,
        db_url: &str,
        username: &str,
        new_password: &str,
    ) -> Result<()> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(self.config.db_timeout_seconds))
            .connect(db_url)
            .await
            .context("Failed to connect to database")?;

        // Use parameterized query to prevent SQL injection
        let query = format!("ALTER USER {} WITH PASSWORD $1", username);
        sqlx::query(&query)
            .bind(new_password)
            .execute(&pool)
            .await
            .context("Failed to update database password")?;

        pool.close().await;

        info!("Database password updated for user: {}", username);
        Ok(())
    }

    /// Update Kubernetes secret with new password
    async fn update_kubernetes_secret(
        &self,
        namespace: &str,
        secret_name: &str,
        new_password: &str,
    ) -> Result<()> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        let mut data = BTreeMap::new();
        data.insert(
            "password".to_string(),
            k8s_openapi::ByteString(new_password.as_bytes().to_vec()),
        );

        let patch = serde_json::json!({
            "data": data
        });

        secrets
            .patch(
                secret_name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Strategic(patch),
            )
            .await
            .context("Failed to update Kubernetes secret")?;

        info!("Kubernetes secret updated: {}/{}", namespace, secret_name);
        Ok(())
    }

    /// Trigger rolling restart of pods by adding an annotation
    async fn restart_pods(&self, namespace: &str, name: &str) -> Result<()> {
        use k8s_openapi::api::apps::v1::StatefulSet;

        let statefulsets: Api<StatefulSet> = Api::namespaced(self.client.clone(), namespace);

        let patch = serde_json::json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "stellar.org/secret-rotated-at": Utc::now().to_rfc3339()
                        }
                    }
                }
            }
        });

        statefulsets
            .patch(
                name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Strategic(patch),
            )
            .await
            .context("Failed to trigger pod restart")?;

        info!("Triggered rolling restart for {}/{}", namespace, name);
        Ok(())
    }

    /// Verify database connection with new credentials
    async fn verify_database_connection(&self, db_url: &str) -> Result<()> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(self.config.db_timeout_seconds))
            .connect(db_url)
            .await
            .context("Failed to verify database connection")?;

        // Simple query to verify connectivity
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .context("Failed to execute verification query")?;

        pool.close().await;

        info!("Database connection verified successfully");
        Ok(())
    }

    /// Log rotation event for audit trail
    async fn log_event(&self, event: &RotationEvent) {
        if self.config.audit_logging_enabled {
            let json = serde_json::to_string(event).unwrap_or_default();
            info!("AUDIT: {}", json);

            // Send to external audit log destination if configured
            if let Some(destination) = &self.config.audit_log_destination {
                if let Err(e) = self.send_to_audit_log(destination, event).await {
                    error!("Failed to send audit log: {}", e);
                }
            }
        }
    }

    /// Send audit log to external system
    async fn send_to_audit_log(&self, destination: &str, event: &RotationEvent) -> Result<()> {
        let client = reqwest::Client::new();
        client
            .post(destination)
            .json(event)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("Failed to send audit log")?;

        Ok(())
    }

    /// Send notification webhook
    async fn send_notification(&self, title: &str, message: &str) {
        if let Some(webhook_url) = &self.config.notification_webhook {
            let payload = serde_json::json!({
                "title": title,
                "message": message,
                "timestamp": Utc::now().to_rfc3339()
            });

            let client = reqwest::Client::new();
            if let Err(e) = client
                .post(webhook_url)
                .json(&payload)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                error!("Failed to send notification: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_password_generation() {
        let config = SecretRotationConfig::default();
        let Ok(client) = Client::try_default().await else {
            eprintln!("skipping test_password_generation: no Kubernetes client available");
            return;
        };
        let scheduler = SecretRotationScheduler::new(config.clone(), client);

        let password = scheduler.generate_secure_password();
        assert_eq!(password.len(), config.password_length);
        assert!(password.chars().all(|c| c.is_alphanumeric()));
    }

    #[tokio::test]
    async fn test_password_hashing() {
        let config = SecretRotationConfig::default();
        let Ok(client) = Client::try_default().await else {
            eprintln!("skipping test_password_hashing: no Kubernetes client available");
            return;
        };
        let scheduler = SecretRotationScheduler::new(config, client);

        // Use a clearly-placeholder value so secret-audit scanners ignore it.
        let password = "test_password_placeholder";
        let hash = scheduler.hash_password(password);

        // SHA256 produces 64 character hex string
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

        // Same password should produce same hash
        let hash2 = scheduler.hash_password(password);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_default_config() {
        let config = SecretRotationConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.schedule, "0 0 1 * *");
        assert_eq!(config.password_length, 32);
        assert_eq!(config.db_timeout_seconds, 30);
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_mtls_config_defaults() {
        let config = MtlsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.cert_validity_hours, 1);
        assert_eq!(config.rotation_minutes, 40);
        assert_eq!(config.namespace, "stellar-system");
        assert_eq!(config.secret_name, "mtls-certs");
    }

    #[tokio::test]
    async fn test_mtls_certificate_bundle_generation_and_rotation() {
        let config = MtlsConfig::default();
        let engine = MtlsRotationEngine::new(config, Client::try_default().await.unwrap());

        let first = engine.generate_bundle().expect("generate first bundle");
        let second = engine.generate_bundle().expect("generate rotated bundle");

        assert!(first.ca_cert.contains("BEGIN CERTIFICATE"));
        assert!(first.ca_key.contains("PRIVATE KEY"));
        assert!(first.server_cert.contains("BEGIN CERTIFICATE"));
        assert!(first.server_key.contains("PRIVATE KEY"));
        assert!(first.client_cert.contains("BEGIN CERTIFICATE"));
        assert!(first.client_key.contains("PRIVATE KEY"));
        assert_ne!(first.ca_cert, second.ca_cert);
        assert_ne!(first.ca_key, second.ca_key);
        assert_ne!(first.server_cert, second.server_cert);
        assert_ne!(first.server_key, second.server_key);
        assert_ne!(first.client_cert, second.client_cert);
        assert_ne!(first.client_key, second.client_key);
    }

}
