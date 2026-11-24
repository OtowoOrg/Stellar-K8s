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
//! Registry pull-path vulnerability gate (epic #1521).
//!
//! The gate sits on the in-cluster registry's pull path, so it keeps working
//! when admission webhooks are degraded. Policy comes from the
//! [`StellarRegistry`] CR (`spec.admission.pullGate` and
//! `spec.scanning.maxCriticalCves`).
//!
//! - [`PullGate::handle_push`] scans a pushed digest synchronously and stores
//!   the report before the push is acknowledged.
//! - [`PullGate::authorize_pull`] denies, in `enforce` mode, digests that were
//!   never scanned or exceed the critical-CVE threshold. The denial carries a
//!   link to the scan report and renders as an OCI distribution error
//!   ([`PullDecision::oci_response`]).
//! - [`PullGate::report`] makes scan reports queryable per digest.
//! - [`rewrite_to_local`] points every image reference at the local registry
//!   so all in-cluster pulls are served (and cached) locally;
//!   [`CacheStats`] tracks the cache hit rate.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use crate::controller::cve::{
    CVECount, CVEDetectionResult, RegistryScannerClient, VulnerabilitySeverity,
};
use crate::crd::stellar_registry::{PullGateMode, StellarRegistry};

/// Vulnerability scan report for one digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub digest: String,
    pub image: String,
    pub scanned_at: DateTime<Utc>,
    pub cve_count: CVECount,
    pub critical_cves: Vec<String>,
    pub report_url: String,
}

/// Scans an image reference (pinned by digest).
#[async_trait]
pub trait DigestScanner: Send + Sync {
    async fn scan(&self, image_ref: &str) -> Result<CVEDetectionResult, String>;
}

#[async_trait]
impl DigestScanner for RegistryScannerClient {
    async fn scan(&self, image_ref: &str) -> Result<CVEDetectionResult, String> {
        self.scan_image(image_ref).await.map_err(|e| e.to_string())
    }
}

/// Outcome of a pull authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "decision")]
pub enum PullDecision {
    Allow,
    /// Audit mode: the pull would have been denied.
    AllowAudited {
        reason: String,
    },
    Deny {
        reason: String,
        report_url: Option<String>,
    },
}

impl PullDecision {
    pub fn is_denied(&self) -> bool {
        matches!(self, PullDecision::Deny { .. })
    }

    /// HTTP status and OCI distribution-spec error body for the registry
    /// response. `None` means the request should be proxied as normal.
    pub fn oci_response(&self, digest: &str) -> Option<(u16, serde_json::Value)> {
        match self {
            PullDecision::Deny { reason, report_url } => Some((
                403,
                serde_json::json!({
                    "errors": [{
                        "code": "DENIED",
                        "message": reason,
                        "detail": { "digest": digest, "reportUrl": report_url }
                    }]
                }),
            )),
            _ => None,
        }
    }
}

/// Pull-path gate state for one registry.
pub struct PullGate {
    pub mode: PullGateMode,
    pub max_critical_cves: u32,
    /// Base URL scan reports are served from (`<base>/<digest>`).
    pub report_base_url: String,
    registry_endpoint: String,
    reports: RwLock<HashMap<String, ScanReport>>,
}

impl PullGate {
    pub fn new(
        mode: PullGateMode,
        max_critical_cves: u32,
        registry_endpoint: &str,
        report_base_url: &str,
    ) -> Self {
        Self {
            mode,
            max_critical_cves,
            report_base_url: report_base_url.trim_end_matches('/').to_string(),
            registry_endpoint: registry_endpoint.to_string(),
            reports: RwLock::new(HashMap::new()),
        }
    }

    pub fn from_registry(registry: &StellarRegistry, report_base_url: &str) -> Self {
        Self::new(
            registry.spec.admission.pull_gate,
            registry.spec.scanning.max_critical_cves,
            &registry.spec.endpoint,
            report_base_url,
        )
    }

    fn report_url(&self, digest: &str) -> String {
        format!("{}/{digest}", self.report_base_url)
    }

    /// Scan a pushed manifest before acknowledging the push. On scanner
    /// failure no report is stored, so the digest stays unscanned (and is
    /// denied in enforce mode) and the error is returned to the pusher.
    pub async fn handle_push<S: DigestScanner>(
        &self,
        scanner: &S,
        repository: &str,
        digest: &str,
    ) -> Result<ScanReport, String> {
        let image = format!("{}/{repository}@{digest}", self.registry_endpoint);
        let result = scanner.scan(&image).await?;
        let report = ScanReport {
            digest: digest.to_string(),
            image,
            scanned_at: result.scan_timestamp,
            cve_count: result.cve_count.clone(),
            critical_cves: result
                .vulnerabilities
                .iter()
                .filter(|v| v.severity == VulnerabilitySeverity::Critical)
                .map(|v| v.cve_id.clone())
                .collect(),
            report_url: self.report_url(digest),
        };
        self.reports
            .write()
            .await
            .insert(digest.to_string(), report.clone());
        Ok(report)
    }

    /// Scan report for a digest.
    pub async fn report(&self, digest: &str) -> Option<ScanReport> {
        self.reports.read().await.get(digest).cloned()
    }

    /// Every stored scan report. Used to carry reports across a policy
    /// change, which builds a new gate.
    pub async fn export_reports(&self) -> Vec<ScanReport> {
        self.reports.read().await.values().cloned().collect()
    }

    /// Adopt reports from a previous gate. Existing reports win, and each
    /// imported report's link is rewritten to this gate's report base URL.
    pub async fn import_reports(&self, reports: Vec<ScanReport>) {
        let mut stored = self.reports.write().await;
        for mut report in reports {
            report.report_url = self.report_url(&report.digest);
            stored.entry(report.digest.clone()).or_insert(report);
        }
    }

    /// Number of digests with a stored scan report.
    pub async fn report_count(&self) -> usize {
        self.reports.read().await.len()
    }

    /// Decide whether a manifest pull by `digest` may proceed. Tag pulls must
    /// be resolved to a digest by the caller first.
    pub async fn authorize_pull(&self, digest: &str) -> PullDecision {
        if self.mode == PullGateMode::Off {
            return PullDecision::Allow;
        }
        let violation = match self.reports.read().await.get(digest) {
            None => Some((format!("{digest} has not been scanned"), None)),
            Some(r) if r.cve_count.critical > self.max_critical_cves => Some((
                format!(
                    "{digest} has {} critical CVE(s) (max {}): {}",
                    r.cve_count.critical,
                    self.max_critical_cves,
                    r.critical_cves.join(", ")
                ),
                Some(r.report_url.clone()),
            )),
            Some(_) => None,
        };
        match (violation, self.mode) {
            (None, _) => PullDecision::Allow,
            (Some((reason, _)), PullGateMode::Audit) => PullDecision::AllowAudited { reason },
            (Some((reason, report_url)), _) => PullDecision::Deny { reason, report_url },
        }
    }
}

/// Split a registry API path `/v2/<name>/manifests/<reference>`.
pub fn parse_manifest_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/v2/")?;
    let (name, reference) = rest.rsplit_once("/manifests/")?;
    (!name.is_empty() && !reference.is_empty()).then_some((name, reference))
}

/// Rewrite an image reference so it is pulled through the local registry,
/// keeping the upstream host as a path prefix (pull-through cache layout).
/// References already pointing at `local_registry` are returned unchanged.
pub fn rewrite_to_local(image: &str, local_registry: &str) -> String {
    if image.starts_with(&format!("{local_registry}/")) {
        return image.to_string();
    }
    let first = image.split('/').next().unwrap_or("");
    let has_host =
        image.contains('/') && (first.contains('.') || first.contains(':') || first == "localhost");
    let (host, path) = if has_host {
        image.split_once('/').unwrap()
    } else if image.contains('/') {
        ("docker.io", image)
    } else {
        return format!("{local_registry}/docker.io/library/{image}");
    };
    format!("{local_registry}/{host}/{path}")
}

/// Pull-through cache counters.
#[derive(Debug, Default)]
pub struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
}

impl CacheStats {
    pub fn record(&self, hit: bool) {
        let c = if hit { &self.hits } else { &self.misses };
        c.fetch_add(1, Ordering::Relaxed);
    }

    /// Fraction of pulls served from cache; `1.0` before any pull.
    pub fn hit_rate(&self) -> f64 {
        let h = self.hits.load(Ordering::Relaxed);
        let m = self.misses.load(Ordering::Relaxed);
        if h + m == 0 {
            1.0
        } else {
            h as f64 / (h + m) as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::cve::Vulnerability;

    struct Seeded {
        critical: Vec<&'static str>,
        fail: bool,
    }

    #[async_trait]
    impl DigestScanner for Seeded {
        async fn scan(&self, image_ref: &str) -> Result<CVEDetectionResult, String> {
            if self.fail {
                return Err("scanner unavailable".into());
            }
            let vulnerabilities: Vec<Vulnerability> = self
                .critical
                .iter()
                .map(|id| Vulnerability {
                    cve_id: id.to_string(),
                    severity: VulnerabilitySeverity::Critical,
                    package: "openssl".into(),
                    installed_version: "1.0".into(),
                    fixed_version: Some("1.1".into()),
                    description: String::new(),
                })
                .collect();
            Ok(CVEDetectionResult {
                current_image: image_ref.into(),
                cve_count: CVECount {
                    critical: vulnerabilities.len() as u32,
                    ..Default::default()
                },
                has_critical: !vulnerabilities.is_empty(),
                vulnerabilities,
                patched_version: None,
                scan_timestamp: Utc::now(),
            })
        }
    }

    fn gate(mode: PullGateMode) -> PullGate {
        PullGate::new(mode, 0, "registry.local", "https://registry.local/reports/")
    }

    #[tokio::test]
    async fn seeded_critical_cve_is_denied_with_report() {
        let g = gate(PullGateMode::Enforce);
        let scanner = Seeded {
            critical: vec!["CVE-2024-0001"],
            fail: false,
        };
        let report = g
            .handle_push(&scanner, "app/api", "sha256:bad")
            .await
            .unwrap();
        assert_eq!(report.image, "registry.local/app/api@sha256:bad");
        assert_eq!(report.critical_cves, vec!["CVE-2024-0001"]);

        let d = g.authorize_pull("sha256:bad").await;
        assert!(d.is_denied());
        let (status, body) = d.oci_response("sha256:bad").unwrap();
        assert_eq!(status, 403);
        assert_eq!(body["errors"][0]["code"], "DENIED");
        assert_eq!(
            body["errors"][0]["detail"]["reportUrl"],
            "https://registry.local/reports/sha256:bad"
        );
        assert!(body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("CVE-2024-0001"));
        assert_eq!(g.report("sha256:bad").await.unwrap(), report);
    }

    #[tokio::test]
    async fn unscanned_pull_denied_in_enforce_only() {
        assert!(gate(PullGateMode::Enforce)
            .authorize_pull("sha256:new")
            .await
            .is_denied());
        assert!(matches!(
            gate(PullGateMode::Audit).authorize_pull("sha256:new").await,
            PullDecision::AllowAudited { .. }
        ));
        assert_eq!(
            gate(PullGateMode::Off).authorize_pull("sha256:new").await,
            PullDecision::Allow
        );
    }

    #[tokio::test]
    async fn clean_image_allowed_and_scanner_failure_leaves_unscanned() {
        let g = gate(PullGateMode::Enforce);
        let clean = Seeded {
            critical: vec![],
            fail: false,
        };
        g.handle_push(&clean, "app/api", "sha256:ok").await.unwrap();
        assert_eq!(g.authorize_pull("sha256:ok").await, PullDecision::Allow);

        let broken = Seeded {
            critical: vec![],
            fail: true,
        };
        assert!(g.handle_push(&broken, "app/api", "sha256:x").await.is_err());
        assert!(g.report("sha256:x").await.is_none());
        assert!(g.authorize_pull("sha256:x").await.is_denied());
    }

    #[test]
    fn parses_manifest_paths() {
        assert_eq!(
            parse_manifest_path("/v2/org/app/manifests/sha256:abc"),
            Some(("org/app", "sha256:abc"))
        );
        assert_eq!(parse_manifest_path("/v2/org/app/blobs/sha256:abc"), None);
    }

    #[test]
    fn rewrites_every_reference_to_local_registry() {
        let l = "registry.local";
        assert_eq!(
            rewrite_to_local("nginx:1.27", l),
            "registry.local/docker.io/library/nginx:1.27"
        );
        assert_eq!(
            rewrite_to_local("bitnami/redis", l),
            "registry.local/docker.io/bitnami/redis"
        );
        assert_eq!(
            rewrite_to_local("ghcr.io/stellar/stellar-k8s:1.3.7", l),
            "registry.local/ghcr.io/stellar/stellar-k8s:1.3.7"
        );
        assert_eq!(
            rewrite_to_local("registry.local/x/y", l),
            "registry.local/x/y"
        );
    }

    #[test]
    fn cache_hit_rate() {
        let s = CacheStats::default();
        assert_eq!(s.hit_rate(), 1.0);
        for i in 0..100 {
            s.record(i % 20 != 0);
        }
        assert!((s.hit_rate() - 0.95).abs() < 1e-9);
    }
}
