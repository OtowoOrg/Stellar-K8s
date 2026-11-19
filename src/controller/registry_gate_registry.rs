// Copyright 2026 Stellar-K8s Contributors
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
//! Process-wide registry of pull-path gates, one per `StellarRegistry` (#2000).
//!
//! The gate in [`crate::controller::registry_gate`] is policy plus scan state
//! for one registry. This module is what lets the rest of the operator reach
//! it:
//!
//! - the `StellarRegistry` reconciler calls [`GateRegistry::sync`] so the gate
//!   always reflects the CR's `spec.admission.pullGate` and
//!   `spec.scanning.maxCriticalCves`, keeping stored scan reports when the
//!   policy changes;
//! - the REST API (`rest_api::registry_gate_handlers`) reads gates to answer
//!   pull-authorization requests and to serve scan reports, which is where the
//!   `reportUrl` in a denial points.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use kube::ResourceExt;
use tokio::sync::RwLock;

use crate::controller::registry_gate::{PullGate, ScanReport};
use crate::crd::stellar_registry::StellarRegistry;

/// Environment variable holding the externally reachable base URL scan reports
/// are served from (`<base>/<digest>`).
pub const REPORT_BASE_URL_ENV: &str = "REGISTRY_REPORT_BASE_URL";

/// Used when [`REPORT_BASE_URL_ENV`] is unset: the operator's own REST API.
pub const DEFAULT_REPORT_BASE_URL: &str =
    "http://stellar-operator.stellar-system.svc:9090/api/v1/registry/scan-reports";

/// A configured base URL with surrounding whitespace and trailing slashes
/// removed, or `None` when it is blank.
fn normalize_report_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Base URL for scan report links in denials.
pub fn report_base_url() -> String {
    std::env::var(REPORT_BASE_URL_ENV)
        .ok()
        .and_then(|v| normalize_report_base_url(&v))
        .unwrap_or_else(|| DEFAULT_REPORT_BASE_URL.to_string())
}

/// A gate together with the scanner its pushes are scanned by.
struct GateEntry {
    gate: Arc<PullGate>,
    /// `spec.scanning.endpoint`, or `None` when scanning is disabled.
    scanner_endpoint: Option<String>,
}

/// Pull-path gates keyed by `<namespace>/<name>` of the `StellarRegistry`.
#[derive(Default)]
pub struct GateRegistry {
    gates: RwLock<HashMap<String, GateEntry>>,
}

impl GateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry key for a `StellarRegistry`.
    pub fn key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    /// Build the gate for `registry` from its current spec and install it,
    /// carrying over the scan reports of the gate it replaces.
    pub async fn sync(&self, registry: &StellarRegistry, report_base_url: &str) -> Arc<PullGate> {
        let namespace = registry
            .namespace()
            .unwrap_or_else(|| "default".to_string());
        let key = Self::key(&namespace, &registry.name_any());

        let gate = Arc::new(PullGate::from_registry(registry, report_base_url));
        let scanning = &registry.spec.scanning;
        let scanner_endpoint = (scanning.enabled && !scanning.endpoint.trim().is_empty())
            .then(|| scanning.endpoint.clone());

        let mut gates = self.gates.write().await;
        if let Some(previous) = gates.get(&key) {
            gate.import_reports(previous.gate.export_reports().await)
                .await;
        }
        gates.insert(
            key,
            GateEntry {
                gate: gate.clone(),
                scanner_endpoint,
            },
        );
        gate
    }

    pub async fn get(&self, namespace: &str, name: &str) -> Option<Arc<PullGate>> {
        self.gates
            .read()
            .await
            .get(&Self::key(namespace, name))
            .map(|entry| entry.gate.clone())
    }

    /// Scanner endpoint pushes to this registry are scanned by, if scanning is
    /// enabled.
    pub async fn scanner_endpoint(&self, namespace: &str, name: &str) -> Option<String> {
        self.gates
            .read()
            .await
            .get(&Self::key(namespace, name))
            .and_then(|entry| entry.scanner_endpoint.clone())
    }

    /// Drop the gate of a deleted `StellarRegistry`.
    pub async fn remove(&self, namespace: &str, name: &str) {
        self.gates.write().await.remove(&Self::key(namespace, name));
    }

    /// Scan report for a digest from any registry. Digests are content
    /// addresses, so the same digest has the same report everywhere it was
    /// scanned; the first match is returned.
    pub async fn find_report(&self, digest: &str) -> Option<ScanReport> {
        let gates: Vec<Arc<PullGate>> = self
            .gates
            .read()
            .await
            .values()
            .map(|entry| entry.gate.clone())
            .collect();
        for gate in gates {
            if let Some(report) = gate.report(digest).await {
                return Some(report);
            }
        }
        None
    }

    pub async fn len(&self) -> usize {
        self.gates.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.gates.read().await.is_empty()
    }
}

/// The operator-wide gate registry.
pub fn global() -> &'static GateRegistry {
    static GLOBAL: OnceLock<GateRegistry> = OnceLock::new();
    GLOBAL.get_or_init(GateRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::registry_gate::DigestScanner;
    use crate::crd::stellar_registry::PullGateMode;
    use async_trait::async_trait;

    struct CleanScanner;

    #[async_trait]
    impl DigestScanner for CleanScanner {
        async fn scan(
            &self,
            image_ref: &str,
        ) -> Result<crate::controller::cve::CVEDetectionResult, String> {
            Ok(crate::controller::cve::CVEDetectionResult {
                current_image: image_ref.to_string(),
                vulnerabilities: vec![],
                patched_version: None,
                scan_timestamp: chrono::Utc::now(),
                cve_count: crate::controller::cve::CVECount::default(),
                has_critical: false,
            })
        }
    }

    fn gate(mode: PullGateMode) -> PullGate {
        PullGate::new(mode, 0, "registry.local:5000", "http://reports.local/r")
    }

    #[test]
    fn keys_are_namespace_slash_name() {
        assert_eq!(GateRegistry::key("stellar", "main"), "stellar/main");
    }

    #[tokio::test]
    async fn import_carries_reports_and_rewrites_links() {
        let old = gate(PullGateMode::Enforce);
        old.handle_push(&CleanScanner, "app", "sha256:abc")
            .await
            .unwrap();

        let new = PullGate::new(
            PullGateMode::Enforce,
            0,
            "registry.local:5000",
            "http://other.local/reports/",
        );
        new.import_reports(old.export_reports().await).await;

        let report = new.report("sha256:abc").await.unwrap();
        assert_eq!(report.report_url, "http://other.local/reports/sha256:abc");
        assert_eq!(new.report_count().await, 1);
    }

    #[tokio::test]
    async fn imported_reports_do_not_overwrite_newer_ones() {
        let old = gate(PullGateMode::Enforce);
        old.handle_push(&CleanScanner, "app", "sha256:abc")
            .await
            .unwrap();
        let stale = old.report("sha256:abc").await.unwrap();

        let new = gate(PullGateMode::Enforce);
        let fresh = new
            .handle_push(&CleanScanner, "app", "sha256:abc")
            .await
            .unwrap();
        new.import_reports(vec![stale]).await;

        assert_eq!(
            new.report("sha256:abc").await.unwrap().scanned_at,
            fresh.scanned_at
        );
    }

    #[tokio::test]
    async fn find_report_searches_every_gate() {
        let registry = GateRegistry::new();
        assert!(registry.is_empty().await);
        assert!(registry.find_report("sha256:none").await.is_none());
    }

    #[test]
    fn base_url_is_trimmed_and_blank_is_rejected() {
        assert_eq!(
            normalize_report_base_url("  https://reports.example/r// ").as_deref(),
            Some("https://reports.example/r")
        );
        assert_eq!(normalize_report_base_url("   "), None);
        assert_eq!(normalize_report_base_url("/"), None);
    }

    #[test]
    fn default_base_url_points_at_the_scan_report_route() {
        assert!(DEFAULT_REPORT_BASE_URL.ends_with("/api/v1/registry/scan-reports"));
    }
}
