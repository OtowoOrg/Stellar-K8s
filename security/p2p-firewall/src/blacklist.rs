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

//! IP Blacklist Manager — Kubernetes NetworkPolicy + iptables enforcement.
//!
//! When the analyzer raises a [`ThreatLevel::High`] or [`ThreatLevel::Critical`]
//! verdict, this module:
//!
//! 1. **Updates the in-memory blacklist** with an expiry timestamp.
//! 2. **Applies a Kubernetes `NetworkPolicy`** named
//!    `p2p-firewall-blocklist` in the configured namespace.  The policy adds
//!    a `from` rule with the offending CIDR (`/32`) that is evaluated by the
//!    cluster CNI (Calico, Cilium, Flannel + NetworkPolicy controller, etc.).
//! 3. **Optionally calls `iptables`** via a subprocess for node-level drops.
//!    This is a defence-in-depth layer and requires the firewall to run with
//!    `NET_ADMIN` capability or as root.
//!
//! Expired entries are pruned automatically on every enforcement cycle.

use crate::analyzer::ThreatLevel;
use chrono::{DateTime, Utc};
use k8s_openapi::api::networking::v1::{
    IPBlock, NetworkPolicy, NetworkPolicyIngressRule, NetworkPolicyPeer, NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use thiserror::Error;
use tracing::{error, info, warn};

// ── Types ──────────────────────────────────────────────────────────────────────

/// Reason an IP was blacklisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlacklistReason {
    Flood,
    MalformedPayload,
    HandshakeFailure,
    Combined(Vec<String>),
}

impl std::fmt::Display for BlacklistReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlacklistReason::Flood => write!(f, "flood"),
            BlacklistReason::MalformedPayload => write!(f, "malformed-payload"),
            BlacklistReason::HandshakeFailure => write!(f, "handshake-failure"),
            BlacklistReason::Combined(rs) => write!(f, "{}", rs.join(",")),
        }
    }
}

/// A single blacklist entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlacklistEntry {
    pub ip: IpAddr,
    pub reason: BlacklistReason,
    pub threat_level: ThreatLevel,
    pub blacklisted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub packet_count: u64,
}

impl BlacklistEntry {
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Errors from the blacklist manager.
#[derive(Debug, Error)]
pub enum BlacklistError {
    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),
    #[error("iptables subprocess error: {0}")]
    IptablesError(String),
    #[error("serialization error: {0}")]
    SerdeError(#[from] serde_json::Error),
}

// ── BlacklistManager ───────────────────────────────────────────────────────────

/// Manages the set of blacklisted IPs and syncs enforcement to Kubernetes and iptables.
#[derive(Clone)]
pub struct BlacklistManager {
    inner: Arc<BlacklistManagerInner>,
}

struct BlacklistManagerInner {
    entries: Mutex<HashMap<IpAddr, BlacklistEntry>>,
    kube_client: Option<Client>,
    namespace: String,
    blacklist_duration: Duration,
    network_policy_name: String,
    scp_port: u16,
    use_iptables: bool,
}

impl BlacklistManager {
    /// Create a new blacklist manager.
    ///
    /// `kube_client` is optional — if `None`, Kubernetes NetworkPolicy
    /// enforcement is skipped (useful in test environments).
    pub fn new(
        kube_client: Option<Client>,
        namespace: impl Into<String>,
        blacklist_duration_secs: u64,
        scp_port: u16,
        use_iptables: bool,
    ) -> Self {
        Self {
            inner: Arc::new(BlacklistManagerInner {
                entries: Mutex::new(HashMap::new()),
                kube_client,
                namespace: namespace.into(),
                blacklist_duration: Duration::from_secs(blacklist_duration_secs),
                network_policy_name: "p2p-firewall-blocklist".into(),
                scp_port,
                use_iptables,
            }),
        }
    }

    /// Add or refresh a blacklist entry.  Returns `true` if this is a new entry.
    pub async fn blacklist(
        &self,
        ip: IpAddr,
        reason: BlacklistReason,
        threat_level: ThreatLevel,
        packet_count: u64,
    ) -> Result<bool, BlacklistError> {
        let is_new = {
            let mut entries = self.inner.entries.lock().expect("blacklist lock poisoned");
            let is_new = !entries.contains_key(&ip);

            let now = Utc::now();
            let expires_at = now
                + chrono::Duration::from_std(self.inner.blacklist_duration)
                    .unwrap_or(chrono::Duration::seconds(300));

            entries.insert(
                ip,
                BlacklistEntry {
                    ip,
                    reason: reason.clone(),
                    threat_level,
                    blacklisted_at: now,
                    expires_at,
                    packet_count,
                },
            );
            is_new
        };

        if is_new {
            info!(
                %ip,
                %reason,
                duration_secs = self.inner.blacklist_duration.as_secs(),
                "IP blacklisted"
            );
        } else {
            info!(%ip, "Blacklist entry refreshed");
        }

        // Enforce via Kubernetes NetworkPolicy.
        if let Err(e) = self.enforce_network_policy().await {
            warn!(error = %e, "Failed to update Kubernetes NetworkPolicy");
        }

        // Enforce via iptables (best-effort, requires NET_ADMIN).
        if self.inner.use_iptables {
            if let Err(e) = self.apply_iptables_drop(ip).await {
                warn!(error = %e, %ip, "Failed to apply iptables DROP rule");
            }
        }

        Ok(is_new)
    }

    /// Remove an IP from the blacklist and revoke enforcement rules.
    pub async fn unblacklist(&self, ip: IpAddr) -> Result<(), BlacklistError> {
        let removed = {
            let mut entries = self.inner.entries.lock().expect("blacklist lock poisoned");
            entries.remove(&ip).is_some()
        };

        if removed {
            info!(%ip, "IP removed from blacklist");

            if let Err(e) = self.enforce_network_policy().await {
                warn!(error = %e, "Failed to update NetworkPolicy after unblacklist");
            }

            if self.inner.use_iptables {
                let _ = self.remove_iptables_drop(ip).await;
            }
        }

        Ok(())
    }

    /// Prune all expired entries and revoke their enforcement rules.
    pub async fn prune_expired(&self) -> Result<usize, BlacklistError> {
        let expired_ips: Vec<IpAddr> = {
            let entries = self.inner.entries.lock().expect("blacklist lock poisoned");
            entries
                .values()
                .filter(|e| e.is_expired())
                .map(|e| e.ip)
                .collect()
        };

        let count = expired_ips.len();
        for ip in expired_ips {
            self.unblacklist(ip).await?;
        }

        if count > 0 {
            info!(count, "Pruned expired blacklist entries");
        }

        Ok(count)
    }

    /// Returns `true` if the IP is currently blacklisted (and not expired).
    pub fn is_blacklisted(&self, ip: IpAddr) -> bool {
        let entries = self.inner.entries.lock().expect("blacklist lock poisoned");
        entries.get(&ip).map(|e| !e.is_expired()).unwrap_or(false)
    }

    /// Returns a snapshot of all active (non-expired) blacklist entries.
    pub fn active_entries(&self) -> Vec<BlacklistEntry> {
        let entries = self.inner.entries.lock().expect("blacklist lock poisoned");
        entries
            .values()
            .filter(|e| !e.is_expired())
            .cloned()
            .collect()
    }

    /// Returns the count of currently active blacklisted IPs.
    pub fn active_count(&self) -> usize {
        let entries = self.inner.entries.lock().expect("blacklist lock poisoned");
        entries.values().filter(|e| !e.is_expired()).count()
    }

    // ── Private enforcement helpers ─────────────────────────────────────────

    /// Rebuild and apply the Kubernetes `NetworkPolicy` for all active IPs.
    async fn enforce_network_policy(&self) -> Result<(), BlacklistError> {
        let client = match &self.inner.kube_client {
            Some(c) => c.clone(),
            None => {
                // No Kubernetes client in this environment (e.g., CI / unit tests).
                return Ok(());
            }
        };

        let active: Vec<IpAddr> = {
            let entries = self.inner.entries.lock().expect("blacklist lock poisoned");
            entries
                .values()
                .filter(|e| !e.is_expired())
                .map(|e| e.ip)
                .collect()
        };

        // Build NetworkPolicy spec: deny ingress from all blacklisted CIDRs
        // on the SCP port.
        let except_cidrs: Vec<String> = active.iter().map(|ip| format!("{ip}/32")).collect();

        // Kubernetes NetworkPolicy semantics: we deny specific CIDRs by using
        // an `ipBlock` with `except` on the allow-all base rule.  This means
        // we allow traffic from 0.0.0.0/0 EXCEPT the blacklisted CIDRs.
        let ingress_rule = if except_cidrs.is_empty() {
            // No blocked IPs — allow everything.
            NetworkPolicyIngressRule {
                from: Some(vec![NetworkPolicyPeer {
                    ip_block: Some(IPBlock {
                        cidr: "0.0.0.0/0".into(),
                        except: None,
                    }),
                    ..Default::default()
                }]),
                ports: None,
            }
        } else {
            NetworkPolicyIngressRule {
                from: Some(vec![NetworkPolicyPeer {
                    ip_block: Some(IPBlock {
                        cidr: "0.0.0.0/0".into(),
                        except: Some(except_cidrs.clone()),
                    }),
                    ..Default::default()
                }]),
                ports: None,
            }
        };

        let policy = NetworkPolicy {
            metadata: ObjectMeta {
                name: Some(self.inner.network_policy_name.clone()),
                namespace: Some(self.inner.namespace.clone()),
                labels: Some({
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("app.kubernetes.io/managed-by".into(), "p2p-firewall".into());
                    m.insert("stellar.org/component".into(), "scp-firewall".into());
                    m
                }),
                annotations: Some({
                    let mut a = std::collections::BTreeMap::new();
                    a.insert("p2p-firewall/blocked-ips".into(), except_cidrs.join(","));
                    a.insert("p2p-firewall/updated-at".into(), Utc::now().to_rfc3339());
                    a
                }),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec {
                pod_selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                    match_labels: Some({
                        let mut m = std::collections::BTreeMap::new();
                        m.insert("app".into(), "stellar-core".into());
                        m
                    }),
                    ..Default::default()
                },
                ingress: Some(vec![ingress_rule]),
                policy_types: Some(vec!["Ingress".into()]),
                ..Default::default()
            }),
        };

        let api: Api<NetworkPolicy> = Api::namespaced(client, &self.inner.namespace);
        let ssapply = PatchParams::apply("p2p-firewall").force();
        api.patch(
            &self.inner.network_policy_name,
            &ssapply,
            &Patch::Apply(&policy),
        )
        .await?;

        info!(
            blocked_count = except_cidrs.len(),
            namespace = %self.inner.namespace,
            "Kubernetes NetworkPolicy updated"
        );

        Ok(())
    }

    /// Apply an `iptables` DROP rule for the given IP on the SCP port.
    ///
    /// Requires `NET_ADMIN` capability or root.  Failures are logged but do not
    /// interrupt the main firewall loop.
    async fn apply_iptables_drop(&self, ip: IpAddr) -> Result<(), BlacklistError> {
        let port = self.inner.scp_port;
        let ip_str = ip.to_string();

        // Check if rule already exists to avoid duplicates.
        let check = tokio::process::Command::new("iptables")
            .args([
                "-C",
                "INPUT",
                "-s",
                &ip_str,
                "-p",
                "tcp",
                "--dport",
                &port.to_string(),
                "-j",
                "DROP",
            ])
            .output()
            .await;

        match check {
            Ok(out) if out.status.success() => {
                // Rule already present.
                return Ok(());
            }
            Ok(_) => {
                // Rule not present — insert it.
            }
            Err(e) => {
                return Err(BlacklistError::IptablesError(format!(
                    "iptables check failed: {e}"
                )));
            }
        }

        let insert = tokio::process::Command::new("iptables")
            .args([
                "-I",
                "INPUT",
                "1",
                "-s",
                &ip_str,
                "-p",
                "tcp",
                "--dport",
                &port.to_string(),
                "-j",
                "DROP",
                "-m",
                "comment",
                "--comment",
                "p2p-firewall-blocklist",
            ])
            .output()
            .await
            .map_err(|e| BlacklistError::IptablesError(format!("iptables insert failed: {e}")))?;

        if !insert.status.success() {
            let stderr = String::from_utf8_lossy(&insert.stderr);
            return Err(BlacklistError::IptablesError(format!(
                "iptables INSERT failed: {stderr}"
            )));
        }

        info!(%ip, port, "iptables DROP rule applied");
        Ok(())
    }

    /// Remove an `iptables` DROP rule for the given IP.
    async fn remove_iptables_drop(&self, ip: IpAddr) -> Result<(), BlacklistError> {
        let port = self.inner.scp_port;
        let ip_str = ip.to_string();

        let out = tokio::process::Command::new("iptables")
            .args([
                "-D",
                "INPUT",
                "-s",
                &ip_str,
                "-p",
                "tcp",
                "--dport",
                &port.to_string(),
                "-j",
                "DROP",
                "-m",
                "comment",
                "--comment",
                "p2p-firewall-blocklist",
            ])
            .output()
            .await
            .map_err(|e| BlacklistError::IptablesError(format!("iptables delete failed: {e}")))?;

        if out.status.success() {
            info!(%ip, port, "iptables DROP rule removed");
        }

        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn make_manager() -> BlacklistManager {
        BlacklistManager::new(
            None, // no K8s client in unit tests
            "stellar", 60, // 60s duration
            11625, false, // no iptables in unit tests
        )
    }

    #[tokio::test]
    async fn test_blacklist_and_lookup() {
        let mgr = make_manager();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

        assert!(!mgr.is_blacklisted(ip));

        let is_new = mgr
            .blacklist(ip, BlacklistReason::Flood, ThreatLevel::Critical, 999)
            .await
            .unwrap();
        assert!(is_new);
        assert!(mgr.is_blacklisted(ip));
    }

    #[tokio::test]
    async fn test_blacklist_refresh() {
        let mgr = make_manager();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));

        mgr.blacklist(ip, BlacklistReason::Flood, ThreatLevel::Critical, 1)
            .await
            .unwrap();
        let is_new = mgr
            .blacklist(ip, BlacklistReason::Flood, ThreatLevel::Critical, 2)
            .await
            .unwrap();

        assert!(!is_new, "second blacklist call should not be 'new'");
        assert!(mgr.is_blacklisted(ip));
    }

    #[tokio::test]
    async fn test_unblacklist() {
        let mgr = make_manager();
        let ip = IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1));

        mgr.blacklist(ip, BlacklistReason::MalformedPayload, ThreatLevel::High, 50)
            .await
            .unwrap();
        assert!(mgr.is_blacklisted(ip));

        mgr.unblacklist(ip).await.unwrap();
        assert!(!mgr.is_blacklisted(ip));
    }

    #[tokio::test]
    async fn test_active_count() {
        let mgr = make_manager();

        let ips = [
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)),
            IpAddr::V4(Ipv4Addr::new(3, 3, 3, 3)),
        ];

        for ip in &ips {
            mgr.blacklist(*ip, BlacklistReason::Flood, ThreatLevel::Critical, 100)
                .await
                .unwrap();
        }

        assert_eq!(mgr.active_count(), 3);

        mgr.unblacklist(ips[1]).await.unwrap();
        assert_eq!(mgr.active_count(), 2);
    }

    #[tokio::test]
    async fn test_active_entries_snapshot() {
        let mgr = make_manager();
        let ip = IpAddr::V4(Ipv4Addr::new(99, 99, 99, 99));

        mgr.blacklist(ip, BlacklistReason::HandshakeFailure, ThreatLevel::High, 7)
            .await
            .unwrap();

        let entries = mgr.active_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ip, ip);
        assert_eq!(entries[0].reason, BlacklistReason::HandshakeFailure);
    }

    #[test]
    fn test_blacklist_reason_display() {
        assert_eq!(BlacklistReason::Flood.to_string(), "flood");
        assert_eq!(
            BlacklistReason::MalformedPayload.to_string(),
            "malformed-payload"
        );
        assert_eq!(
            BlacklistReason::HandshakeFailure.to_string(),
            "handshake-failure"
        );
    }
}
