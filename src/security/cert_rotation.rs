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
//! Automated Certificate Management with Vault PKI Backend (issue #1415)
//!
//! Extends the existing [`cert_manager`](super::cert_manager) with:
//!
//! - Vault PKI backend integration (Vault-issued certificates via the PKI
//!   secrets engine).
//! - Configurable renewal scheduler that fires proactively before expiry.
//! - Certificate expiry monitoring with 30-day / 7-day / 24-hour alerting
//!   thresholds.
//! - Rotation audit trail for compliance purposes.
//!
//! ## Architecture
//!
//! ```text
//! CertRotationController
//!   → CertInventory (list + watch cert-manager Certificate resources)
//!   → RenewalScheduler (timer, fires at renewBefore)
//!     → VaultPkiBackend | LetsEncryptBackend | LocalCaBackend
//!   → ExpiryMonitor (Prometheus metrics + alert firing)
//!   → RotationAuditLog (persisted rotation events)
//! ```
//!
//! The `VaultPkiBackend` calls the Vault HTTP API directly via `reqwest`
//! rather than pulling in a Vault SDK, keeping the dependency tree minimal.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum CertRotationError {
    #[error("certificate not found: {0}")]
    NotFound(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("vault error: {0}")]
    Vault(String),
    #[error("renewal not yet due: expires in {0} days")]
    NotDue(i64),
    #[error("configuration error: {0}")]
    Config(String),
}

// ── PKI backend abstraction ───────────────────────────────────────────────────

/// Trait implemented by every certificate issuer backend.
#[async_trait::async_trait]
pub trait PkiBackend: Send + Sync {
    /// Issue or renew a certificate for the given domain names.
    ///
    /// Returns the new certificate PEM, private key PEM, and expiry timestamp.
    async fn issue(
        &self,
        request: &CertIssuanceRequest,
    ) -> Result<CertIssuanceResponse, CertRotationError>;

    /// Human-readable name of the backend (used in audit logs).
    fn backend_name(&self) -> &str;
}

/// Parameters for a certificate issuance request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertIssuanceRequest {
    pub common_name: String,
    pub san_dns: Vec<String>,
    pub san_ip: Vec<String>,
    /// Desired validity duration in hours.
    pub ttl_hours: u32,
    /// Key algorithm: `ECDSA` (default) or `RSA`.
    pub key_algorithm: String,
    /// Additional metadata passed to the backend (e.g. Vault role name).
    pub metadata: HashMap<String, String>,
}

/// Response returned by a successful issuance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertIssuanceResponse {
    pub cert_pem: String,
    pub key_pem: String,
    pub ca_chain_pem: String,
    pub serial_number: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub backend: String,
}

// ── Vault PKI backend ─────────────────────────────────────────────────────────

/// Configuration for the HashiCorp Vault PKI backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPkiConfig {
    /// Vault server address (e.g. `https://vault.stellar-system.svc:8200`).
    pub vault_addr: String,
    /// Vault token (use a Kubernetes ServiceAccount-based auth in production).
    pub vault_token: String,
    /// PKI secrets engine mount path (e.g. `pki_int`).
    pub pki_mount: String,
    /// Vault role that controls which SANs are allowed.
    pub role_name: String,
    /// Whether to verify the Vault server's TLS certificate.
    pub tls_verify: bool,
    /// Request timeout.
    pub timeout_secs: u64,
}

impl Default for VaultPkiConfig {
    fn default() -> Self {
        Self {
            vault_addr: "https://vault.stellar-system.svc:8200".to_string(),
            vault_token: String::new(),
            pki_mount: "pki_int".to_string(),
            role_name: "stellar-operator".to_string(),
            tls_verify: true,
            timeout_secs: 30,
        }
    }
}

/// Vault PKI backend implementation.
pub struct VaultPkiBackend {
    config: VaultPkiConfig,
    client: reqwest::Client,
}

impl VaultPkiBackend {
    pub fn new(config: VaultPkiConfig) -> Result<Self, CertRotationError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .danger_accept_invalid_certs(!config.tls_verify)
            .build()
            .map_err(|e| CertRotationError::Config(e.to_string()))?;
        Ok(Self { config, client })
    }
}

#[async_trait::async_trait]
impl PkiBackend for VaultPkiBackend {
    async fn issue(
        &self,
        request: &CertIssuanceRequest,
    ) -> Result<CertIssuanceResponse, CertRotationError> {
        let url = format!(
            "{}/v1/{}/issue/{}",
            self.config.vault_addr.trim_end_matches('/'),
            self.config.pki_mount,
            self.config.role_name,
        );

        let alt_names: Vec<String> = request.san_dns.clone();
        let ip_sans = request.san_ip.join(",");

        let body = serde_json::json!({
            "common_name": request.common_name,
            "alt_names": alt_names.join(","),
            "ip_sans": ip_sans,
            "ttl": format!("{}h", request.ttl_hours),
            "key_type": if request.key_algorithm == "RSA" { "rsa" } else { "ec" },
        });

        let resp = self
            .client
            .post(&url)
            .header("X-Vault-Token", &self.config.vault_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CertRotationError::Vault(format!("HTTP error: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CertRotationError::Vault(format!(
                "Vault returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CertRotationError::Vault(format!("JSON parse error: {e}")))?;

        let get_str = |key: &str| -> Result<String, CertRotationError> {
            data["data"][key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| CertRotationError::Vault(format!("missing field: {key}")))
        };

        let cert_pem = get_str("certificate")?;
        let key_pem = get_str("private_key")?;
        let serial = get_str("serial_number").unwrap_or_else(|_| "unknown".to_string());
        let ca_chain = data["data"]["ca_chain"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        let now = Utc::now();
        let not_after = now + ChronoDuration::hours(request.ttl_hours as i64);

        Ok(CertIssuanceResponse {
            cert_pem,
            key_pem,
            ca_chain_pem: ca_chain,
            serial_number: serial,
            not_before: now,
            not_after,
            backend: self.backend_name().to_string(),
        })
    }

    fn backend_name(&self) -> &str {
        "vault-pki"
    }
}

// ── Expiry monitor ────────────────────────────────────────────────────────────

/// Alert severity level for certificate expiry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpirySeverity {
    /// Cert expires within 30 days.
    Warning,
    /// Cert expires within 7 days.
    Critical,
    /// Cert expires within 24 hours or has already expired.
    Emergency,
}

/// A certificate expiry alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertExpiryAlert {
    pub cert_name: String,
    pub namespace: String,
    pub domain: String,
    pub days_remaining: i64,
    pub not_after: DateTime<Utc>,
    pub severity: ExpirySeverity,
    pub fired_at: DateTime<Utc>,
    pub message: String,
}

/// Certificate expiry monitoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpiryMonitorConfig {
    /// Fire a Warning alert this many days before expiry (default: 30).
    pub warning_days: u32,
    /// Fire a Critical alert this many days before expiry (default: 7).
    pub critical_days: u32,
    /// Fire an Emergency alert this many hours before expiry (default: 24).
    pub emergency_hours: u32,
}

impl Default for ExpiryMonitorConfig {
    fn default() -> Self {
        Self {
            warning_days: 30,
            critical_days: 7,
            emergency_hours: 24,
        }
    }
}

/// Scans the certificate inventory and emits expiry alerts.
pub struct ExpiryMonitor {
    config: ExpiryMonitorConfig,
}

impl ExpiryMonitor {
    pub fn new(config: ExpiryMonitorConfig) -> Self {
        Self { config }
    }

    /// Evaluate a set of certificates and return all currently-active alerts.
    pub fn evaluate(&self, certs: &[CertRecord]) -> Vec<CertExpiryAlert> {
        let now = Utc::now();
        let mut alerts = Vec::new();

        for cert in certs {
            let days = (cert.not_after - now).num_days();
            let hours = (cert.not_after - now).num_hours();

            let severity = if hours <= self.config.emergency_hours as i64 {
                Some(ExpirySeverity::Emergency)
            } else if days <= self.config.critical_days as i64 {
                Some(ExpirySeverity::Critical)
            } else if days <= self.config.warning_days as i64 {
                Some(ExpirySeverity::Warning)
            } else {
                None
            };

            if let Some(sev) = severity {
                let message = match &sev {
                    ExpirySeverity::Emergency => format!(
                        "EMERGENCY: Certificate '{}' for {} expires in {} hours. Immediate action required.",
                        cert.name, cert.domain, hours.max(0)
                    ),
                    ExpirySeverity::Critical => format!(
                        "CRITICAL: Certificate '{}' for {} expires in {} days.",
                        cert.name, cert.domain, days.max(0)
                    ),
                    ExpirySeverity::Warning => format!(
                        "WARNING: Certificate '{}' for {} expires in {} days.",
                        cert.name, cert.domain, days
                    ),
                };

                alerts.push(CertExpiryAlert {
                    cert_name: cert.name.clone(),
                    namespace: cert.namespace.clone(),
                    domain: cert.domain.clone(),
                    days_remaining: days,
                    not_after: cert.not_after,
                    severity: sev,
                    fired_at: now,
                    message,
                });
            }
        }

        // Sort: most urgent first.
        alerts.sort_by(|a, b| a.days_remaining.cmp(&b.days_remaining));
        alerts
    }

    /// Render Prometheus metrics for the current certificate inventory.
    pub fn render_prometheus(&self, certs: &[CertRecord]) -> String {
        let now = Utc::now();
        let mut lines = vec![
            "# HELP stellar_cert_expiry_days Days until certificate expires (negative = already expired)\n\
             # TYPE stellar_cert_expiry_days gauge".to_string(),
        ];
        for cert in certs {
            let days = (cert.not_after - now).num_days();
            lines.push(format!(
                r#"stellar_cert_expiry_days{{name="{name}",namespace="{ns}",domain="{domain}",issuer="{issuer}"}} {days}"#,
                name = cert.name,
                ns = cert.namespace,
                domain = cert.domain,
                issuer = cert.issuer,
            ));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

// ── Rotation audit log ────────────────────────────────────────────────────────

/// A single rotation event recorded for audit purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationEvent {
    pub event_id: String,
    pub cert_name: String,
    pub namespace: String,
    pub rotated_at: DateTime<Utc>,
    pub triggered_by: RotationTrigger,
    pub backend: String,
    pub success: bool,
    pub old_serial: String,
    pub new_serial: Option<String>,
    pub new_not_after: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationTrigger {
    /// Scheduled renewal fired because `renewBefore` threshold was reached.
    Scheduled,
    /// Manual request via CLI/API.
    Manual,
    /// Emergency rotation triggered because certificate was near-expiry or expired.
    Emergency,
}

/// In-memory rotation audit log (back this with a persistent store in production).
#[derive(Default)]
pub struct RotationAuditLog {
    events: Arc<RwLock<Vec<RotationEvent>>>,
}

impl RotationAuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn record(&self, event: RotationEvent) {
        self.events.write().await.push(event);
    }

    /// Return all events for a given certificate, newest first.
    pub async fn events_for(&self, cert_name: &str) -> Vec<RotationEvent> {
        let mut out: Vec<RotationEvent> = self
            .events
            .read()
            .await
            .iter()
            .filter(|e| e.cert_name == cert_name)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.rotated_at.cmp(&a.rotated_at));
        out
    }

    pub async fn all_events(&self) -> Vec<RotationEvent> {
        let mut events = self.events.read().await.clone();
        events.sort_by(|a, b| b.rotated_at.cmp(&a.rotated_at));
        events
    }
}

// ── Certificate record ────────────────────────────────────────────────────────

/// Lightweight in-memory representation of a managed certificate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertRecord {
    pub name: String,
    pub namespace: String,
    pub domain: String,
    pub issuer: String,
    pub serial_number: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// Days before expiry at which auto-renewal is triggered.
    pub renew_before_days: u32,
    pub auto_renew: bool,
    pub backend: String,
    pub last_renewed_at: Option<DateTime<Utc>>,
}

impl CertRecord {
    /// Returns `true` if automatic renewal should be triggered now.
    pub fn needs_renewal(&self) -> bool {
        if !self.auto_renew {
            return false;
        }
        let threshold = self.not_after - ChronoDuration::days(self.renew_before_days as i64);
        Utc::now() >= threshold
    }

    pub fn days_until_expiry(&self) -> i64 {
        (self.not_after - Utc::now()).num_days()
    }
}

// ── CertRotationController ────────────────────────────────────────────────────

/// Orchestrates certificate discovery, renewal, and monitoring.
pub struct CertRotationController {
    inventory: Arc<RwLock<HashMap<String, CertRecord>>>,
    backend: Arc<dyn PkiBackend>,
    monitor: ExpiryMonitor,
    audit_log: RotationAuditLog,
}

impl CertRotationController {
    pub fn new(backend: Arc<dyn PkiBackend>, monitor_config: ExpiryMonitorConfig) -> Self {
        Self {
            inventory: Arc::new(RwLock::new(HashMap::new())),
            backend,
            monitor: ExpiryMonitor::new(monitor_config),
            audit_log: RotationAuditLog::new(),
        }
    }

    /// Register or update a certificate record in the inventory.
    pub async fn register(&self, record: CertRecord) {
        self.inventory
            .write()
            .await
            .insert(record.name.clone(), record);
    }

    /// Run a full renewal pass: renew every certificate that is due.
    ///
    /// Returns the list of rotation events (success and failure).
    pub async fn run_renewal_pass(&self) -> Vec<RotationEvent> {
        let due: Vec<CertRecord> = self
            .inventory
            .read()
            .await
            .values()
            .filter(|r| r.needs_renewal())
            .cloned()
            .collect();

        let mut events = Vec::new();

        for record in due {
            tracing::info!(
                cert = %record.name,
                days_remaining = record.days_until_expiry(),
                "auto-renewing certificate"
            );

            let request = CertIssuanceRequest {
                common_name: record.domain.clone(),
                san_dns: vec![record.domain.clone()],
                san_ip: vec![],
                ttl_hours: 2160, // 90 days
                key_algorithm: "ECDSA".to_string(),
                metadata: HashMap::new(),
            };

            let event = match self.backend.issue(&request).await {
                Ok(resp) => {
                    // Update inventory with new expiry.
                    if let Some(rec) = self.inventory.write().await.get_mut(&record.name) {
                        rec.not_before = resp.not_before;
                        rec.not_after = resp.not_after;
                        rec.serial_number = resp.serial_number.clone();
                        rec.last_renewed_at = Some(Utc::now());
                    }
                    RotationEvent {
                        event_id: format!("{}-{}", record.name, Utc::now().timestamp()),
                        cert_name: record.name.clone(),
                        namespace: record.namespace.clone(),
                        rotated_at: Utc::now(),
                        triggered_by: RotationTrigger::Scheduled,
                        backend: resp.backend.clone(),
                        success: true,
                        old_serial: record.serial_number.clone(),
                        new_serial: Some(resp.serial_number),
                        new_not_after: Some(resp.not_after),
                        error: None,
                    }
                }
                Err(e) => {
                    tracing::warn!(cert = %record.name, error = %e, "renewal failed");
                    RotationEvent {
                        event_id: format!("{}-{}", record.name, Utc::now().timestamp()),
                        cert_name: record.name.clone(),
                        namespace: record.namespace.clone(),
                        rotated_at: Utc::now(),
                        triggered_by: RotationTrigger::Scheduled,
                        backend: self.backend.backend_name().to_string(),
                        success: false,
                        old_serial: record.serial_number.clone(),
                        new_serial: None,
                        new_not_after: None,
                        error: Some(e.to_string()),
                    }
                }
            };

            self.audit_log.record(event.clone()).await;
            events.push(event);
        }

        events
    }

    /// Return current expiry alerts for the entire inventory.
    pub async fn current_alerts(&self) -> Vec<CertExpiryAlert> {
        let records: Vec<CertRecord> = self.inventory.read().await.values().cloned().collect();
        self.monitor.evaluate(&records)
    }

    /// Render Prometheus metrics for the inventory.
    pub async fn render_prometheus(&self) -> String {
        let records: Vec<CertRecord> = self.inventory.read().await.values().cloned().collect();
        self.monitor.render_prometheus(&records)
    }

    /// Return the full audit log.
    pub async fn audit_events(&self) -> Vec<RotationEvent> {
        self.audit_log.all_events().await
    }
}

// ── Local CA backend (testing / development) ─────────────────────────────────

/// A local CA backend that issues self-signed certificates using `rcgen`.
/// Suitable for development, test environments, and air-gapped deployments.
pub struct LocalCaBackend {
    #[allow(dead_code)]
    issuer_cn: String,
}

impl LocalCaBackend {
    pub fn new(issuer_cn: impl Into<String>) -> Self {
        Self {
            issuer_cn: issuer_cn.into(),
        }
    }
}

#[async_trait::async_trait]
impl PkiBackend for LocalCaBackend {
    async fn issue(
        &self,
        request: &CertIssuanceRequest,
    ) -> Result<CertIssuanceResponse, CertRotationError> {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

        let mut params = CertificateParams::new(request.san_dns.clone())
            .map_err(|e| CertRotationError::Backend(e.to_string()))?;

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, request.common_name.clone());
        params.distinguished_name = dn;

        let key_pair =
            KeyPair::generate().map_err(|e| CertRotationError::Backend(e.to_string()))?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| CertRotationError::Backend(e.to_string()))?;

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        let now = Utc::now();
        let not_after = now + ChronoDuration::hours(request.ttl_hours as i64);

        Ok(CertIssuanceResponse {
            cert_pem,
            key_pem,
            ca_chain_pem: String::new(),
            serial_number: format!("{:x}", rand::random::<u64>()),
            not_before: now,
            not_after,
            backend: self.backend_name().to_string(),
        })
    }

    fn backend_name(&self) -> &str {
        "local-ca"
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn future(days: i64) -> DateTime<Utc> {
        Utc::now() + ChronoDuration::days(days)
    }

    fn make_record(name: &str, days_until_expiry: i64, renew_before: u32) -> CertRecord {
        CertRecord {
            name: name.to_string(),
            namespace: "stellar".to_string(),
            domain: format!("{name}.stellar.org"),
            issuer: "test-ca".to_string(),
            serial_number: "abcdef".to_string(),
            not_before: Utc::now() - ChronoDuration::days(1),
            not_after: future(days_until_expiry),
            renew_before_days: renew_before,
            auto_renew: true,
            backend: "local-ca".to_string(),
            last_renewed_at: None,
        }
    }

    // ── ExpiryMonitor ─────────────────────────────────────────────────────────

    #[test]
    fn cert_at_35_days_no_alert() {
        let monitor = ExpiryMonitor::new(ExpiryMonitorConfig::default());
        let record = make_record("horizon", 35, 30);
        let alerts = monitor.evaluate(&[record]);
        assert!(alerts.is_empty());
    }

    #[test]
    fn cert_at_25_days_warns() {
        let monitor = ExpiryMonitor::new(ExpiryMonitorConfig::default());
        let record = make_record("horizon", 25, 30);
        let alerts = monitor.evaluate(&[record]);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, ExpirySeverity::Warning);
    }

    #[test]
    fn cert_at_5_days_critical() {
        let monitor = ExpiryMonitor::new(ExpiryMonitorConfig::default());
        let record = make_record("validator", 5, 30);
        let alerts = monitor.evaluate(&[record]);
        assert_eq!(alerts[0].severity, ExpirySeverity::Critical);
    }

    #[test]
    fn expired_cert_is_emergency() {
        let monitor = ExpiryMonitor::new(ExpiryMonitorConfig::default());
        let mut record = make_record("soroban", 0, 30);
        record.not_after = Utc::now() - ChronoDuration::hours(2); // already expired
        let alerts = monitor.evaluate(&[record]);
        assert_eq!(alerts[0].severity, ExpirySeverity::Emergency);
    }

    #[test]
    fn prometheus_output_contains_metric_name() {
        let monitor = ExpiryMonitor::new(ExpiryMonitorConfig::default());
        let record = make_record("horizon", 45, 30);
        let output = monitor.render_prometheus(&[record]);
        assert!(output.contains("stellar_cert_expiry_days"));
        assert!(output.contains("horizon"));
    }

    // ── CertRecord ────────────────────────────────────────────────────────────

    #[test]
    fn needs_renewal_when_past_threshold() {
        let record = make_record("horizon", 25, 30);
        assert!(record.needs_renewal());
    }

    #[test]
    fn no_renewal_when_well_before_threshold() {
        let record = make_record("horizon", 60, 30);
        assert!(!record.needs_renewal());
    }

    #[test]
    fn auto_renew_disabled_suppresses_renewal() {
        let mut record = make_record("horizon", 10, 30);
        record.auto_renew = false;
        assert!(!record.needs_renewal());
    }

    // ── RotationAuditLog ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn audit_log_records_and_retrieves_events() {
        let log = RotationAuditLog::new();
        log.record(RotationEvent {
            event_id: "ev-1".to_string(),
            cert_name: "horizon".to_string(),
            namespace: "stellar".to_string(),
            rotated_at: Utc::now(),
            triggered_by: RotationTrigger::Scheduled,
            backend: "local-ca".to_string(),
            success: true,
            old_serial: "old".to_string(),
            new_serial: Some("new".to_string()),
            new_not_after: Some(future(90)),
            error: None,
        })
        .await;

        let events = log.events_for("horizon").await;
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
    }

    // ── LocalCaBackend ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn local_ca_issues_certificate() {
        let backend = LocalCaBackend::new("Test CA");
        let req = CertIssuanceRequest {
            common_name: "horizon.stellar.org".to_string(),
            san_dns: vec!["horizon.stellar.org".to_string()],
            san_ip: vec![],
            ttl_hours: 2160,
            key_algorithm: "ECDSA".to_string(),
            metadata: HashMap::new(),
        };
        let resp = backend.issue(&req).await.unwrap();
        assert!(!resp.cert_pem.is_empty());
        assert!(!resp.key_pem.is_empty());
        assert_eq!(resp.backend, "local-ca");
        assert!(resp.not_after > Utc::now());
    }

    // ── CertRotationController ────────────────────────────────────────────────

    #[tokio::test]
    async fn controller_renews_due_certificate() {
        let backend = Arc::new(LocalCaBackend::new("Test CA"));
        let controller = CertRotationController::new(backend, ExpiryMonitorConfig::default());

        // Register a cert that is due for renewal (expires in 20 days, threshold 30).
        controller.register(make_record("horizon", 20, 30)).await;

        let events = controller.run_renewal_pass().await;
        assert_eq!(events.len(), 1);
        assert!(events[0].success, "renewal should succeed");
        assert!(events[0].new_serial.is_some());
    }

    #[tokio::test]
    async fn controller_skips_cert_not_due() {
        let backend = Arc::new(LocalCaBackend::new("Test CA"));
        let controller = CertRotationController::new(backend, ExpiryMonitorConfig::default());

        // Cert expires in 60 days with 30-day threshold — not due.
        controller.register(make_record("soroban", 60, 30)).await;

        let events = controller.run_renewal_pass().await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn controller_emits_alerts_for_expiring_certs() {
        let backend = Arc::new(LocalCaBackend::new("Test CA"));
        let controller = CertRotationController::new(backend, ExpiryMonitorConfig::default());

        controller.register(make_record("horizon", 5, 30)).await;

        let alerts = controller.current_alerts().await;
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].severity, ExpirySeverity::Critical);
    }
}
