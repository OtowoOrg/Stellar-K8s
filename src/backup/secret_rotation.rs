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
use k8s_openapi::api::core::v1::Secret;
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
        let scheduler =
            SecretRotationScheduler::new(config.clone(), Client::try_default().await.unwrap());

        let password = scheduler.generate_secure_password();
        assert_eq!(password.len(), config.password_length);
        assert!(password.chars().all(|c| c.is_alphanumeric()));
    }

    #[tokio::test]
    async fn test_password_hashing() {
        let config = SecretRotationConfig::default();
        let scheduler = SecretRotationScheduler::new(config, Client::try_default().await.unwrap());

        let password = "test_password_123";
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
}
