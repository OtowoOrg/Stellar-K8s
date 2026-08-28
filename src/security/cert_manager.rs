//! Certificate Management with Automatic Rotation
//!
//! Provides certificate lifecycle management, automatic renewal,
//! and expiry monitoring for TLS certificates.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Certificate management errors
#[derive(Error, Debug)]
pub enum CertManagerError {
    #[error("Certificate not found: {0}")]
    CertNotFound(String),
    #[error("Certificate expired: {0}")]
    CertExpired(String),
    #[error("Renewal failed: {0}")]
    RenewalFailed(String),
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Certificate status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CertStatus {
    Active,
    ExpiringSoon,
    Expired,
    Renewing,
    Failed,
}

/// Certificate info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateInfo {
    pub name: String,
    pub domain: String,
    pub issuer: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub status: CertStatus,
    pub serial_number: String,
    pub fingerprint: String,
    pub auto_renew: bool,
    pub renewal_threshold_days: u32,
}

impl CertificateInfo {
    /// Check if certificate is expiring soon
    pub fn is_expiring_soon(&self, threshold_days: u32) -> bool {
        let now = Utc::now();
        let threshold = self.not_after - ChronoDuration::days(threshold_days as i64);
        now >= threshold
    }

    /// Get days until expiration
    pub fn days_until_expiry(&self) -> i64 {
        let now = Utc::now();
        (self.not_after - now).num_days()
    }

    /// Get certificate status based on expiry
    pub fn calculate_status(&self, warning_days: u32) -> CertStatus {
        let days = self.days_until_expiry();
        if days < 0 {
            CertStatus::Expired
        } else if days < warning_days as i64 {
            CertStatus::ExpiringSoon
        } else {
            CertStatus::Active
        }
    }
}

/// Certificate renewal config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewalConfig {
    pub enabled: bool,
    pub threshold_days: u32,
    pub max_retries: u32,
    pub retry_delay_seconds: u32,
    pub notification_webhook: Option<String>,
    pub notification_email: Option<String>,
}

impl Default for RenewalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_days: 30,
            max_retries: 3,
            retry_delay_seconds: 300,
            notification_webhook: None,
            notification_email: None,
        }
    }
}

/// Inter-service mTLS configuration for Stellar services (issue #1281)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterServiceMtlsConfig {
    pub enabled: bool,
    pub issuer_name: String,
    pub namespace: String,
    pub stellar_core_service: String,
    pub horizon_service: String,
    pub companion_services: Vec<String>,
    pub cert_duration_hours: u32,
    pub renew_before_hours: u32,
}

impl Default for InterServiceMtlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            issuer_name: "stellar-inter-service-ca".to_string(),
            namespace: "stellar-system".to_string(),
            stellar_core_service: "stellar-core".to_string(),
            horizon_service: "horizon".to_string(),
            companion_services: vec!["soroban-rpc".to_string(), "ingestion-worker".to_string()],
            cert_duration_hours: 2160, // 90 days
            renew_before_hours: 360,   // 15 days
        }
    }
}

/// Cert-manager Custom Resource manifest generator for inter-service mTLS
pub struct CertManagerResourceGenerator;

impl CertManagerResourceGenerator {
    /// Generate a cert-manager Certificate manifest for a given service
    pub fn generate_certificate_manifest(
        config: &InterServiceMtlsConfig,
        service_name: &str,
    ) -> String {
        format!(
            "apiVersion: cert-manager.io/v1\n\
             kind: Certificate\n\
             metadata:\n\
             \x20 name: {service_name}-mtls-cert\n\
             \x20 namespace: {namespace}\n\
             spec:\n\
             \x20 secretName: {service_name}-mtls-secret\n\
             \x20 duration: {duration}h\n\
             \x20 renewBefore: {renew_before}h\n\
             \x20 isCA: false\n\
             \x20 privateKey:\n\
             \x20\x20 algorithm: ECDSA\n\
             \x20\x20 size: 256\n\
             \x20 dnsNames:\n\
             \x20 - {service_name}\n\
             \x20 - {service_name}.{namespace}\n\
             \x20 - {service_name}.{namespace}.svc.cluster.local\n\
             \x20 issuerRef:\n\
             \x20\x20 name: {issuer}\n\
             \x20\x20 kind: Issuer\n\
             \x20\x20 group: cert-manager.io\n",
            service_name = service_name,
            namespace = config.namespace,
            duration = config.cert_duration_hours,
            renew_before = config.renew_before_hours,
            issuer = config.issuer_name,
        )
    }
}

/// Certificate manager
pub struct CertManager {
    certificates: Arc<RwLock<HashMap<String, CertificateInfo>>>,
    renewal_config: RenewalConfig,
    renewal_history: Arc<RwLock<Vec<RenewalRecord>>>,
}

/// Renewal record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewalRecord {
    pub cert_name: String,
    pub renewed_at: DateTime<Utc>,
    pub success: bool,
    pub old_expiry: DateTime<Utc>,
    pub new_expiry: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl CertManager {
    pub fn new(renewal_config: RenewalConfig) -> Self {
        Self {
            certificates: Arc::new(RwLock::new(HashMap::new())),
            renewal_config,
            renewal_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Add or update certificate
    pub async fn add_certificate(&self, cert: CertificateInfo) {
        let mut certs = self.certificates.write().await;
        certs.insert(cert.name.clone(), cert);
    }

    /// Get certificate info
    pub async fn get_certificate(&self, name: &str) -> Option<CertificateInfo> {
        let certs = self.certificates.read().await;
        certs.get(name).cloned()
    }

    /// List all certificates
    pub async fn list_certificates(&self) -> Vec<CertificateInfo> {
        let certs = self.certificates.read().await;
        certs.values().cloned().collect()
    }

    /// Check certificates expiring soon
    pub async fn get_expiring_soon(&self, threshold_days: u32) -> Vec<CertificateInfo> {
        let certs = self.certificates.read().await;
        certs
            .values()
            .filter(|c| c.is_expiring_soon(threshold_days))
            .cloned()
            .collect()
    }

    /// Check for expired certificates
    pub async fn get_expired(&self) -> Vec<CertificateInfo> {
        let certs = self.certificates.read().await;
        certs
            .values()
            .filter(|c| c.days_until_expiry() < 0)
            .cloned()
            .collect()
    }

    /// Renew certificate
    pub async fn renew_certificate(&self, name: &str) -> Result<CertificateInfo, CertManagerError> {
        let mut certs = self.certificates.write().await;
        let cert = certs
            .get_mut(name)
            .ok_or_else(|| CertManagerError::CertNotFound(name.to_string()))?;

        if cert.days_until_expiry() > self.renewal_config.threshold_days as i64 {
            return Err(CertManagerError::RenewalFailed(
                "Certificate not due for renewal".to_string(),
            ));
        }

        let old_expiry = cert.not_after;
        let cert_name = cert.name.clone();

        // Attempt renewal
        let mut success = false;
        let mut new_expiry = None;
        let mut error = None;

        for attempt in 0..self.renewal_config.max_retries {
            tracing::info!(
                "Attempting certificate renewal for {} (attempt {}/{})",
                name,
                attempt + 1,
                self.renewal_config.max_retries
            );

            // In production, this would call cert-manager or ACME provider
            // For now, simulate renewal
            match self.simulate_renewal(cert).await {
                Ok(expiry) => {
                    success = true;
                    new_expiry = Some(expiry);
                    cert.not_after = expiry;
                    cert.status = CertStatus::Active;
                    break;
                }
                Err(e) => {
                    error = Some(e.clone());
                    if attempt < self.renewal_config.max_retries - 1 {
                        tokio::time::sleep(tokio::time::Duration::from_secs(
                            self.renewal_config.retry_delay_seconds as u64,
                        ))
                        .await;
                    }
                }
            }
        }

        // Record renewal
        let record = RenewalRecord {
            cert_name: cert_name.clone(),
            renewed_at: Utc::now(),
            success,
            old_expiry,
            new_expiry,
            error: error.clone(),
        };

        let mut history = self.renewal_history.write().await;
        history.push(record);

        if success {
            Ok(cert.clone())
        } else {
            cert.status = CertStatus::Failed;
            Err(CertManagerError::RenewalFailed(
                error.unwrap_or_else(|| "Unknown error".to_string()),
            ))
        }
    }

    /// Simulate certificate renewal (replace with actual implementation)
    async fn simulate_renewal(&self, cert: &CertificateInfo) -> Result<DateTime<Utc>, String> {
        // In production, this would:
        // 1. Call cert-manager API or ACME provider
        // 2. Wait for certificate issuance
        // 3. Update secret in Kubernetes
        // 4. Reload Ingress/Gateway

        // Simulate 90-day certificate
        let new_expiry = Utc::now() + ChronoDuration::days(90);
        Ok(new_expiry)
    }

    /// Get renewal history
    pub async fn get_renewal_history(&self) -> Vec<RenewalRecord> {
        let history = self.renewal_history.read().await;
        history.clone()
    }

    /// Get certificate metrics
    pub async fn get_metrics(&self) -> CertMetrics {
        let certs = self.certificates.read().await;
        let history = self.renewal_history.read().await;

        let total = certs.len();
        let active = certs
            .values()
            .filter(|c| matches!(c.status, CertStatus::Active))
            .count();
        let expiring_soon = certs
            .values()
            .filter(|c| c.is_expiring_soon(self.renewal_config.threshold_days))
            .count();
        let expired = certs.values().filter(|c| c.days_until_expiry() < 0).count();
        let renewals_attempted = history.len();
        let renewals_succeeded = history.iter().filter(|r| r.success).count();

        CertMetrics {
            total_certificates: total,
            active_certificates: active,
            expiring_soon,
            expired_certificates: expired,
            renewals_attempted,
            renewals_succeeded,
            renewal_success_rate: if renewals_attempted > 0 {
                (renewals_succeeded as f64 / renewals_attempted as f64) * 100.0
            } else {
                100.0
            },
        }
    }
}

/// Certificate metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertMetrics {
    pub total_certificates: usize,
    pub active_certificates: usize,
    pub expiring_soon: usize,
    pub expired_certificates: usize,
    pub renewals_attempted: usize,
    pub renewals_succeeded: usize,
    pub renewal_success_rate: f64,
}

impl CertMetrics {
    /// Convert to Prometheus metrics
    pub fn to_prometheus(&self) -> String {
        format!(
            "# TYPE stellar_cert_total gauge\n\
             stellar_cert_total {}\n\
             # TYPE stellar_cert_active gauge\n\
             stellar_cert_active {}\n\
             # TYPE stellar_cert_expiring_soon gauge\n\
             stellar_cert_expiring_soon {}\n\
             # TYPE stellar_cert_expired gauge\n\
             stellar_cert_expired {}\n\
             # TYPE stellar_cert_renewals_total counter\n\
             stellar_cert_renewals_total {}\n\
             # TYPE stellar_cert_renewals_succeeded counter\n\
             stellar_cert_renewals_succeeded {}\n\
             # TYPE stellar_cert_renewal_success_rate gauge\n\
             stellar_cert_renewal_success_rate {:.2}\n",
            self.total_certificates,
            self.active_certificates,
            self.expiring_soon,
            self.expired_certificates,
            self.renewals_attempted,
            self.renewals_succeeded,
            self.renewal_success_rate,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_certificate_info_expiry() {
        let cert = CertificateInfo {
            name: "test-cert".to_string(),
            domain: "example.com".to_string(),
            issuer: "Let's Encrypt".to_string(),
            not_before: Utc::now() - ChronoDuration::days(30),
            not_after: Utc::now() + ChronoDuration::days(60),
            status: CertStatus::Active,
            serial_number: "1234567890".to_string(),
            fingerprint: "abc123".to_string(),
            auto_renew: true,
            renewal_threshold_days: 30,
        };

        assert!(!cert.is_expiring_soon(30));
        assert_eq!(cert.days_until_expiry(), 60);
    }

    #[test]
    fn test_certificate_expiring_soon() {
        let cert = CertificateInfo {
            name: "test-cert".to_string(),
            domain: "example.com".to_string(),
            issuer: "Let's Encrypt".to_string(),
            not_before: Utc::now() - ChronoDuration::days(60),
            not_after: Utc::now() + ChronoDuration::days(15),
            status: CertStatus::Active,
            serial_number: "1234567890".to_string(),
            fingerprint: "abc123".to_string(),
            auto_renew: true,
            renewal_threshold_days: 30,
        };

        assert!(cert.is_expiring_soon(30));
        assert_eq!(cert.days_until_expiry(), 15);
    }

    #[tokio::test]
    async fn test_cert_manager_add_and_get() {
        let manager = CertManager::new(RenewalConfig::default());

        let cert = CertificateInfo {
            name: "test-cert".to_string(),
            domain: "example.com".to_string(),
            issuer: "Let's Encrypt".to_string(),
            not_before: Utc::now(),
            not_after: Utc::now() + ChronoDuration::days(90),
            status: CertStatus::Active,
            serial_number: "1234567890".to_string(),
            fingerprint: "abc123".to_string(),
            auto_renew: true,
            renewal_threshold_days: 30,
        };

        manager.add_certificate(cert).await;
        let retrieved = manager.get_certificate("test-cert").await;

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().domain, "example.com");
    }

    #[tokio::test]
    async fn test_cert_manager_expiring_soon() {
        let manager = CertManager::new(RenewalConfig::default());

        let cert = CertificateInfo {
            name: "expiring-cert".to_string(),
            domain: "example.com".to_string(),
            issuer: "Let's Encrypt".to_string(),
            not_before: Utc::now() - ChronoDuration::days(60),
            not_after: Utc::now() + ChronoDuration::days(15),
            status: CertStatus::Active,
            serial_number: "1234567890".to_string(),
            fingerprint: "abc123".to_string(),
            auto_renew: true,
            renewal_threshold_days: 30,
        };

        manager.add_certificate(cert).await;
        let expiring = manager.get_expiring_soon(30).await;

        assert_eq!(expiring.len(), 1);
        assert_eq!(expiring[0].name, "expiring-cert");
    }
}
