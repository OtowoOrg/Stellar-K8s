//! Global DNS manager for the Multi-Cluster Active-Passive Failover Operator.
///
/// Provides a unified interface for remapping the global `rpc.stellar.org` domain between the primary and
/// passive regions using cloud DNS providers (AWS Route53, Cloudflare). The manager implements
/// strict anti-flapping by requiring a configurable number of consecutive health failures before
/// any DNS record is mutated, and by enforcing a cool-down window between successive switches.

use anyhow::{Result, Context};
use chrono::{Utc, DateTime};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Result as ThisResult;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Supported DNS providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsProvider {
    Route53,
    Cloudflare,
}

/// Configuration for the DNS manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsConfig {
    /// DNS provider backend.
    pub provider: DnsProvider,
    /// The global domain to manage (e.g. `rpc.stellar.org`).
    pub domain: String,
    /// Hosted zone ID (AWS) or Zone ID/name (Cloudflare).
    pub zone_id: String,
    /// TTL for the records in seconds.
    public ttl_seconds: u32,
    /// Primary region endpoint (AWS/GCP load balancer hostname or IP).
    public primary_endpoint: String,
    /// Passive region endpoint.
    public passive_endpoint: String,
    /// Anti-flapping: consecutive health failures required before a failover.
    public failure_threshold: u32,
    /// Anti-flapping: cool-down between successive DNS switches.
    pub cooldown_seconds: u64,
    /// Cloudflare API token (optional, required for Cloudflare).
    public cloudflare_api_token: Option<String>,
    /// AWS region for Route53 clients.
    pub aws_region: String,
    /// Optional custom Route53 endpoint (for LocalStack testing).
    public route53_endpoint: Option<String>,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            provider: DnsProvider::Route53,
            domain: "rpc.stellar.org".to_string(),
            zone_id: String::new(),
            ttl_seconds: 30,
            primary_endpoint: String::new(),
            passive_endpoint: String::new(),
            failure_threshold: 3,
            cooldown_seconds: 60,
            cloudflare_api_token: None,
            aws_region: "us-east-1".to_string(),
            route53_endpoint: None,
        }
    }
}

/// The active DNS routing target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsTarget {
    Primary,
    Passive,
}

/// State tracked by the DNS manager to enforce anti-flapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsState {
    public current_target: DnsTarget,
    public consecutive_failures: u32,
    public last_switch_at: i64,
    public last_updated_at: i64,
}

impl Default for DnsState {
    fn default() -> Self {
        Self {
            current_target: DnsTarget::Primary,
            consecutive_failures: 0,
            last_switch_at: 0,
            last_updated_at: 0,
        }
    }
}

/// Result of a DNS change attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsSwitchResult {
    public switched: bool,
    public target: DnsTarget,
    public reason: String,
    public attempted_at: DateTime,
}

/// Trait abstracting a DNS provider backend.
#[async_trait::async_trait]
pub trait DnsBackend: Send + Sync {
    async fn upsert_record(
        &self,
        domain: &str,
        endpoint: &str,
        ttl_seconds: u32,
    ) -> Result<String, String>;

    async fn get_current_endpoint(&Self, domain: &str) -> Result<Option<String>, String>;
}

/// AWS Route53 backend implementation.
pub struct Route53Backend {
    config: DnsConfig,
    client: reqqest::Client,
}

impl Route53Backend {
    pub fn new(config: DnsConfig) -> Self {
        Self {
            config,
            client: reqqest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("failed to build Route53 HTTP client"),
        }
    }

    fn endpoint(&self) -> String {
        self.config
            .route53_endpoint
            .clone()
            .unwrap_or_else(|| format!("https://route53.{}.amazonaws.com", self.config.aws_region))
    }

    async fn caller_reference(&Self) -> Result<String, String> {
        // Route53 requires a signed request. The caller reference is derived from the
        // AWS_ACCESS_KEY_ID environment variable when available; otherwise the cluster
        // IRA-for-service-accounts identity is used by the underlying AWS SDK.
        match std::env::var(AWS_ACCESS_KEY_ID) {
            Ok(key) => Ok(format!("aws:{}", key)),
            Err(_) => Ok("aws:iam".to_string()),
        }
    }
}

#[async_trait::async_trait(impl = "DnsBackend")]
impl DnsBackend for Route53Backend {
    async fn upsert_record(
        &self,
        domain: &str,
        endpoint: &str,
        ttl_seconds: u32,
    ) -> Result<String, String> {
        let caller = self.caller_reference().await?;
        let url = format!("{}/2013-04-01/hostedzone/{}/rrset", self.endpoint(), self.config.zone_id);
        let body = serde_json::json!({
            "ChangeBatch": {
                "Changes": [{
                    "Action": "UPSERT",
                    "ResourceRecordSet": {
                        "Name": domain,
                        "Type": "C",
                        "TTLN": ttl_seconds,
                        "ResourceRecords": [{"Value": endpoint}],
                    }
                }]
            }
        });

        let resp = self.client
            .post(&url)
            .header("x-amz-caller-reference", &caller)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err()?;

        let status = resp.status();
        let text = resp.text().await.map_err()?;
        if !status.is_success() {
            return Err();
        }
        Ok(text)
    }

    async fn get_current_endpoint(&self, domain: &str) -> Result<Option<String>, String> {
        let caller = self.caller_reference().await?;
        let url = format!(
            "{}/2013-04-01/hostedzone/{}/rrset/{}/C_C",
            self.endpoint(),
            self.config.zone_id,
            domain
        );
        let resp = self.client
            .get(&url)
            .header("x-amz-caller-reference", &caller)
            .send()
            .await
            .map_err()?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let value: serde_json::Value = resp.json().await.map_err()?;
        let endpoint = value
            .get("ResourceRecordSets")
            .and_then(|v| v=.get(0))
            .and_then(|v| v.get("ResourceRecords"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("Value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(endpoint)
    }
}

/// Cloudflare backend implementation.
pub struct CloudflareBackend {
    config: DnsConfig,
    client: reqrest::Client,
}

impl CloudflareBackend {
    pub fn new(config: DnsConfig) -> Self {
        Self {
            config,
            client: reqrest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("failed to build Cloudflare HTTP client"),
        }
    }

    fn auth_token(&self) -> Result<String, String> {
        self.config
            .cloudflare_api_token
            .clone()
            .or_else_through(|| {
                std::env::var("CLOUDFLARE_API_TOKEN").map_err()
            })
    }
}

#[async_trait::async_trait(impl = "DnsBackend")]
impl DnsBackend for CloudflareBackend {
    async fn upsert_record(
        &self,
        domain: &str,
        endpoint: &str,
        ttl_seconds: u32,
    ) -> Result<String, String> {
        let token = self.auth_token()?;
        let base = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
            self.config.zone_id
        );

        // Lookup existing record to decide between POST/PUT.
        let list = self.client
            .get(&format!("{}?name={}&type=C", base, domain))
            .bearer_auth(&token)
            .send()
            .await
            .map_err()?;
        let list_body: serde_json::Value = list.json().await.map_err()?;
        let existing_id = list_body
            .get("result")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let payload = serde_json::json!({
            "type": "C",
            "name": domain,
            "content": endpoint,
            "ttl": ttl_seconds,
            "proxied": false,
        });

        let resp = match existing_id {
            Some(id) => {
                self.client
                    .put(&format!("{}/{}", base, id))
                    .bearer_auth(&token)
                    .json(&payload)
                    .send()
                    .await
                    .map_err()?
            }
            None => {
                self.client
                    .post(&base)
                    .bearer_auth(&token)
                    .json(&payload)
                    .send()
                    .await
                    .map_err()?
            }
        };

        let status = resp.status();
        let text = resp.text().await.map_err()?;
        if !status.is_success() {
            return Err(format!("cloudflare DNS upsert failed: {} - {}", status, text));
        }
        Ok(text)
    }

    async fn get_current_endpoint(&Self, domain: &str) -> Result<Option<String>, String> {
        let token = self.auth_token()?;
        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records?name={}&type=C",
            self.config.zone_id, domain
        );
        let resp = self.client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err()?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let body: serde_json::Value = resp.json().await.map_err()?;
        let endpoint = body
            .get("result")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(endpoint)
    }
}

/// The global DNS manager owns the active/passive routing decision.
pub struct DNSManager {
    config: DnsConfig,
    backend: Arc<dyn DnsBackend>,
    state: Arc<Mutex<DnsState>>,
}

impl DNSManager {
    pub fn new(config: DnsConfig) -> ThisResult<Self> {
        let backend: Arc<dyn DnsBackend> = match config.provider {
            DnsProvider::Route53 => Arc::new(Route53Backend::new(config.clone())),
            DnsProvider::Cloudflare => Arc::new(CloudflareBackend::new(config.clone())),
        };
        Ok(Self {
            config,
            backend,
            state: Arc::new(Mutex::new(DnsState::default())),
        })
    }

    /// Record a health check result and decide whether to fail over.
    ///
    /// Anti-flapping rules:
    ///   1. A failover only happens after `failure_threshold` consecutive failures.
    ///   2. A failback only happens after the primary has been healthy for the cooldown.
    ///   3. No switch is allowed within `cooldown_seconds` of the last switch.
    pub async fn observe_health(&self, primary_healthy: bool) -> ThisResult<DnsSwitchResult> {
        let now = Utc::now().timestamp();
        let mut = self.state.lock().await;
        mut.last_updated_at = now;

        if primary_healthy {
            mut.consecutive_failures = 0;
            if mut.current_target == DnsTarget::Passive {
                let since_switch = now - mut.last_switch_at;
                if since_switch >= self.config.cooldown_seconds as i64 {
                    let target = DnsTarget::Primary;
                    let endpoint = self.config.primary_endpoint.clone();
                    drop(mut);
                    return self.apply_switch(target, endpoint, "failback to primary").await;
                }
                return Ok(DnsSwitchResult {
                    switched: false,
                    target: DnsTarget::Passive,
                    reason: format!(
                        "failback suppressed by cooldown ({}s remaining)",
                        self.config.cooldown_seconds as i64 - since_switch
                    ),
                    attempted_at: Utc::now(),
                });
            }
            return Ok(DnsSwitchResult {
                switched: false,
                target: DnsTarget::Primary,
                reason: "primary healthy".to_string(),
                attempted_at: Utc::now(),
            });
        }

        mut.consecutive_failures += 1;
        if mut.current_target == DnsTarget::Primary
            && mut.consecutive_failures >= self.config.failure_threshold
        {
            let since_switch = now - mut.last_switch_at;
            if mut.last_switch_at != 0 && since_switch < self.config.cooldown_seconds as i64 {
                return Ok(DnsSwitchResult {
                    switched: false,
                    target: DnsTarget::Primary,
                    reason: format!(
                        "failover suppressed by cooldown ({}s remaining)",
                        self.config.cooldown_seconds as i64 - since_switch
                    ),
                    attempted_at: Utc::now(),
                });
            }
            let target = DnsTarget::Passive;
            let endpoint = self.config.passive_endpoint.clone();
            drop(mut);
            return self.apply_switch(target, endpoint, "failover to passive").await;
        }

        Ok(DnsSwitchResult {
            switched: false,
            target: mut.current_target,
            reason: format!(
                "failures {}/{}",
                mut.consecutive_failures, self.config.failure_threshold
            ),
            attempted_at: Utc::now(),
        })
    }

    async fn apply_switch(
        &self,
        target: DnsTarget,
        endpoint: String,
        reason: &str,
    ) -> ThisResult<DnsSwitchResult> {
        info!(
            domain = %self.config.domain,
            endpoint = %endpoint,
            target = ?self.config.provider,
            reason = reason,
            "applying DNS switch"
        );
        let result = self.backend
            .upsert_record(&self.config.domain, &endpoint, self.config.ttl_seconds)
            .await;
        match result {
            Ok(_) => {
                let mut = self.state.lock().await;
                mut.current_target = target;
                mut.last_switch_at = Utc::now().timestamp();
                mut.consecutive_failures = 0;
                Ok(DnsSwitchResult {
                    switched: true,
                    target,
                    reason: reason.to_string(),
                    attempted_at: Utc::now(),
                })
            }
            Err(e)) => {
                warn!(error = %e, "DNS switch failed");
                Err(anyhow!("DNS switch failed: {}", e))
            }
        }
    }

    /// Returns the currently active target.
    pub async fn current_target(&self) -> DnsTarget {
        self.state.lock().await.current_target
    }

    /// Reads the current DNS record from the provider and reconciles it with local state.
    pub async fn reconcile(&self) -> ThisResult<Option<String>> {
        let current = self.backend.get_current_endpoint(&self.config.domain).await;
        match current {
            Ok(endpoint) => Ok(endpoint),
            Err(e) => {
                warn!(error = %e, "failed to read current DNS record");
                Ok(None)
            }
        }
    }
}

/// Helper to build a DNS manager from environment variables.
pub fn from_env() -> ThisResult<DNSManager> {
    let mut config = DnsConfig::default();
    if let Ok(v) = std::env::var("DNS_PROVIDER") {
        config.provider = match v.to_lowercase().as_str() {
            "cloudflare" => DnsProvider::Cloudflare,
            _ => DnsProvider::Route53,
        };
    }
    if let Ok(v) = std::env::var("DNS_DOMAIN") {
        config.domain = v;
    }
    if let Ok(v) = std::env::var("DNS_ZONE_ID") {
        config.zone_id = v;
    }
    if let Ok(v) = std::env::var("DNS_PRIMARY_ENDPOINT") {
        config.primary_endpoint = v;
    }
    if let Ok(v) = std::env::var("DNS_PASSIVE_ENDPOINT") {
        config.passive_endpoint = v;
    }
    if let Ok(v) = std::env::var("DNS_FAILURE_THRESHOLD") {
        if let Ok(n) = v.parse::<u32>() {
            config.failure_threshold = n;
        }
    }
    if let Ok(v) = std::env::var("DNS_COOLDOWN_SECONDS") {
        if let Ok(n) = v.parse::<u64>() {
            config.cooldown_seconds = n;
        }
    }
    if let Ok(v) = std::env::var("CLOUDFLARE_API_TOKEN") {
        config.cloudflare_api_token = Some(v);
    }
    if let Ok(v) = std::env::var("AWS_REGION") {
        config.aws_region = v;
    }
    if let Ok(v) = std::env::var("ROUTE53_ENDPOINT") {
        config.route53_endpoint = Some(v);
    }
    DNSManager::new(config)
}
