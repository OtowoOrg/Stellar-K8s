//! Background CVE Scanner
//!
//! Implements a background scanner that:
//! 1. Periodically scans all images used by operator-managed pods.
//! 2. Reports findings as Prometheus metrics and Kubernetes Events.
//! 3. Alerts on Critical vulnerabilities in production namespaces.
//!
//! # Integration
//!
//! The scanner runs as a background task spawned by the operator at startup.
//! It uses the Trivy HTTP API (or Grype if configured) to scan images.
//!
//! # Prometheus Metrics
//!
//! - `stellar_cve_vulnerabilities_total{image, severity}` — count per image/severity
//! - `stellar_cve_scan_timestamp_seconds{image}` — last scan time
//! - `stellar_cve_vulnerable_pods_total{namespace, severity}` — vulnerable pods per namespace
//! - `stellar_cve_critical_alerts_total` — total critical alerts fired

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::time::Duration;

use k8s_openapi::api::core::v1::{Event, ObjectReference, Pod};
use kube::{
    api::{Api, ListParams, ObjectMeta, PostParams},
    Client, ResourceExt,
};
use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use tokio::time::sleep;
use tracing::{error, info, warn};

use super::cve::{CVECount, RegistryScannerClient, VulnerabilitySeverity};
use crate::error::Result;

// ---------------------------------------------------------------------------
// Prometheus metrics
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CveScanLabels {
    pub image: String,
    pub severity: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CveImageLabels {
    pub image: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CvePodLabels {
    pub namespace: String,
    pub severity: String,
}

/// Total vulnerabilities per image and severity.
pub static CVE_VULNERABILITIES_TOTAL: Lazy<Family<CveScanLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Unix timestamp of the last successful scan per image.
pub static CVE_SCAN_TIMESTAMP: Lazy<Family<CveImageLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Number of vulnerable pods per namespace and severity.
pub static CVE_VULNERABLE_PODS_TOTAL: Lazy<Family<CvePodLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Total critical alerts fired.
pub static CVE_CRITICAL_ALERTS_TOTAL: Lazy<Counter<u64, AtomicU64>> = Lazy::new(Counter::default);

// ---------------------------------------------------------------------------
// Scanner configuration
// ---------------------------------------------------------------------------

/// Configuration for the background CVE scanner.
#[derive(Clone, Debug)]
pub struct CveScannerConfig {
    /// Trivy/Grype HTTP API endpoint.
    pub scanner_endpoint: String,

    /// How often to scan all images (seconds). Default: 3600 (1 hour).
    pub scan_interval_secs: u64,

    /// Namespaces to scan. Empty = all namespaces.
    pub namespaces: Vec<String>,

    /// Whether to fire Kubernetes Events for findings.
    pub emit_k8s_events: bool,

    /// Whether to alert on Critical vulnerabilities.
    pub alert_on_critical: bool,
}

impl Default for CveScannerConfig {
    fn default() -> Self {
        Self {
            scanner_endpoint: "http://trivy-api.security-scanning:8080".to_string(),
            scan_interval_secs: 3600,
            namespaces: vec![],
            emit_k8s_events: true,
            alert_on_critical: true,
        }
    }
}

/// Summary of a scan for one pod.
#[derive(Debug, Clone)]
pub struct PodScanSummary {
    pub namespace: String,
    pub pod_name: String,
    pub image: String,
    pub cve_count: CVECount,
    pub has_critical: bool,
}

// ---------------------------------------------------------------------------
// Background scanner loop
// ---------------------------------------------------------------------------

/// Spawn the background CVE scanner. Returns immediately; scanning runs in background.
pub fn spawn_background_scanner(client: Client, config: CveScannerConfig) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_scan_cycle(&client, &config).await {
                error!("CVE scan cycle failed: {}", e);
            }
            sleep(Duration::from_secs(config.scan_interval_secs)).await;
        }
    });
}

/// Run one full scan cycle across all managed pods.
pub async fn run_scan_cycle(client: &Client, config: &CveScannerConfig) -> Result<()> {
    info!("Starting CVE scan cycle");

    let scanner = RegistryScannerClient::new(config.scanner_endpoint.clone(), None);

    // Collect all pods across configured namespaces.
    let pods = collect_pods(client, &config.namespaces).await?;
    info!("Scanning {} pods for CVEs", pods.len());

    let mut summaries = Vec::new();

    for pod in &pods {
        let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
        let pod_name = pod.name_any();

        // Extract unique images from the pod spec.
        let images = extract_pod_images(pod);

        for image in images {
            match scanner.scan_image(&image).await {
                Ok(result) => {
                    // Update Prometheus metrics.
                    update_cve_metrics(&image, &result.cve_count);

                    let summary = PodScanSummary {
                        namespace: namespace.clone(),
                        pod_name: pod_name.clone(),
                        image: image.clone(),
                        cve_count: result.cve_count.clone(),
                        has_critical: result.has_critical,
                    };

                    if result.has_critical && config.alert_on_critical {
                        fire_critical_alert(client, pod, &image, &result.cve_count).await;
                    }

                    if config.emit_k8s_events && result.cve_count.total() > 0 {
                        emit_cve_event(client, pod, &image, &result.cve_count).await;
                    }

                    summaries.push(summary);
                }
                Err(e) => {
                    warn!(
                        "Failed to scan image {} in pod {}/{}: {}",
                        image, namespace, pod_name, e
                    );
                }
            }
        }
    }

    // Update per-namespace vulnerable pod counts.
    update_namespace_metrics(&summaries);

    info!(
        "CVE scan cycle complete: {} pods scanned, {} with critical vulnerabilities",
        summaries.len(),
        summaries.iter().filter(|s| s.has_critical).count()
    );

    Ok(())
}

/// Collect all pods from the specified namespaces (or all namespaces if empty).
async fn collect_pods(client: &Client, namespaces: &[String]) -> Result<Vec<Pod>> {
    let mut all_pods = Vec::new();

    if namespaces.is_empty() {
        // All namespaces.
        let pods_api: Api<Pod> = Api::all(client.clone());
        let pods = pods_api
            .list(&ListParams::default().labels("app.kubernetes.io/managed-by=stellar-operator"))
            .await?;
        all_pods.extend(pods.items);
    } else {
        for ns in namespaces {
            let pods_api: Api<Pod> = Api::namespaced(client.clone(), ns);
            let pods = pods_api
                .list(
                    &ListParams::default().labels("app.kubernetes.io/managed-by=stellar-operator"),
                )
                .await?;
            all_pods.extend(pods.items);
        }
    }

    Ok(all_pods)
}

/// Extract all unique container images from a pod.
fn extract_pod_images(pod: &Pod) -> Vec<String> {
    let mut images = Vec::new();

    if let Some(spec) = &pod.spec {
        for container in &spec.containers {
            if let Some(image) = &container.image {
                if !images.contains(image) {
                    images.push(image.clone());
                }
            }
        }
        for container in spec.init_containers.iter().flatten() {
            if let Some(image) = &container.image {
                if !images.contains(image) {
                    images.push(image.clone());
                }
            }
        }
    }

    images
}

/// Update Prometheus CVE metrics for a scanned image.
fn update_cve_metrics(image: &str, counts: &CVECount) {
    let now = chrono::Utc::now().timestamp();
    CVE_SCAN_TIMESTAMP
        .get_or_create(&CveImageLabels {
            image: image.to_string(),
        })
        .set(now);

    for (severity, count) in [
        ("CRITICAL", counts.critical as i64),
        ("HIGH", counts.high as i64),
        ("MEDIUM", counts.medium as i64),
        ("LOW", counts.low as i64),
    ] {
        CVE_VULNERABILITIES_TOTAL
            .get_or_create(&CveScanLabels {
                image: image.to_string(),
                severity: severity.to_string(),
            })
            .set(count);
    }
}

/// Update per-namespace vulnerable pod counts.
fn update_namespace_metrics(summaries: &[PodScanSummary]) {
    // Group by namespace.
    let mut ns_critical: BTreeMap<String, i64> = BTreeMap::new();
    let mut ns_high: BTreeMap<String, i64> = BTreeMap::new();

    for s in summaries {
        if s.cve_count.critical > 0 {
            *ns_critical.entry(s.namespace.clone()).or_default() += 1;
        }
        if s.cve_count.high > 0 {
            *ns_high.entry(s.namespace.clone()).or_default() += 1;
        }
    }

    for (ns, count) in ns_critical {
        CVE_VULNERABLE_PODS_TOTAL
            .get_or_create(&CvePodLabels {
                namespace: ns,
                severity: "CRITICAL".to_string(),
            })
            .set(count);
    }
    for (ns, count) in ns_high {
        CVE_VULNERABLE_PODS_TOTAL
            .get_or_create(&CvePodLabels {
                namespace: ns,
                severity: "HIGH".to_string(),
            })
            .set(count);
    }
}

/// Fire a critical CVE alert: log at ERROR level and increment the alert counter.
async fn fire_critical_alert(client: &Client, pod: &Pod, image: &str, counts: &CVECount) {
    let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
    let pod_name = pod.name_any();

    CVE_CRITICAL_ALERTS_TOTAL.inc();

    error!(
        namespace = %namespace,
        pod = %pod_name,
        image = %image,
        critical_cves = counts.critical,
        high_cves = counts.high,
        "CRITICAL CVE ALERT: Pod is running an image with {} critical vulnerabilities. \
         Immediate remediation required.",
        counts.critical
    );

    // Also emit a Warning Kubernetes Event.
    emit_cve_event(client, pod, image, counts).await;
}

/// Emit a Kubernetes Event on the pod describing the CVE findings.
async fn emit_cve_event(client: &Client, pod: &Pod, image: &str, counts: &CVECount) {
    let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
    let pod_name = pod.name_any();

    let severity_label = if counts.critical > 0 {
        "Critical"
    } else if counts.high > 0 {
        "High"
    } else {
        "Medium"
    };

    let message = format!(
        "CVE scan found vulnerabilities in image '{}': {} critical, {} high, {} medium, {} low. \
         Review and update the image to a patched version.",
        image, counts.critical, counts.high, counts.medium, counts.low
    );

    let event_type = if counts.critical > 0 {
        "Warning"
    } else {
        "Normal"
    };

    let now = chrono::Utc::now();
    let event = Event {
        metadata: ObjectMeta {
            name: Some(format!(
                "{}-cve-{}-{}",
                pod_name,
                severity_label.to_lowercase(),
                now.timestamp()
            )),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        action: Some("CVEScan".to_string()),
        event_time: None,
        first_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
        last_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
        involved_object: ObjectReference {
            api_version: Some("v1".to_string()),
            kind: Some("Pod".to_string()),
            name: Some(pod_name.clone()),
            namespace: Some(namespace.clone()),
            uid: pod.metadata.uid.clone(),
            ..Default::default()
        },
        message: Some(message),
        reason: Some(format!("CVE{}", severity_label)),
        reporting_component: Some("stellar-operator/cve-scanner".to_string()),
        reporting_instance: Some(
            std::env::var("POD_NAME").unwrap_or_else(|_| "stellar-operator".to_string()),
        ),
        source: Some(k8s_openapi::api::core::v1::EventSource {
            component: Some("stellar-operator".to_string()),
            host: None,
        }),
        type_: Some(event_type.to_string()),
        count: Some(1),
        series: None,
        related: None,
    };

    let events_api: Api<Event> = Api::namespaced(client.clone(), &namespace);
    if let Err(e) = events_api.create(&PostParams::default(), &event).await {
        warn!(
            "Failed to emit CVE event for pod {}/{}: {}",
            namespace, pod_name, e
        );
    }
}

// ---------------------------------------------------------------------------
// CLI: list vulnerable pods
// ---------------------------------------------------------------------------

/// List all pods with CVE findings above the given minimum severity.
/// Used by the `kubectl stellar cve list` CLI command.
pub async fn list_vulnerable_pods(
    client: &Client,
    config: &CveScannerConfig,
    min_severity: VulnerabilitySeverity,
) -> Result<Vec<PodScanSummary>> {
    let scanner = RegistryScannerClient::new(config.scanner_endpoint.clone(), None);
    let pods = collect_pods(client, &config.namespaces).await?;

    let mut results = Vec::new();

    for pod in &pods {
        let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
        let pod_name = pod.name_any();
        let images = extract_pod_images(pod);

        for image in images {
            if let Ok(scan) = scanner.scan_image(&image).await {
                let qualifies = match min_severity {
                    VulnerabilitySeverity::Critical => scan.cve_count.critical > 0,
                    VulnerabilitySeverity::High => {
                        scan.cve_count.critical > 0 || scan.cve_count.high > 0
                    }
                    VulnerabilitySeverity::Medium => {
                        scan.cve_count.critical > 0
                            || scan.cve_count.high > 0
                            || scan.cve_count.medium > 0
                    }
                    _ => scan.cve_count.total() > 0,
                };

                if qualifies {
                    results.push(PodScanSummary {
                        namespace: namespace.clone(),
                        pod_name: pod_name.clone(),
                        image: image.clone(),
                        cve_count: scan.cve_count,
                        has_critical: scan.has_critical,
                    });
                }
            }
        }
    }

    Ok(results)
}

/// Register CVE scanner metrics into an existing Prometheus registry.
pub fn register_cve_metrics(registry: &mut prometheus_client::registry::Registry) {
    registry.register(
        "stellar_cve_vulnerabilities_total",
        "Number of CVE vulnerabilities found per image and severity level.",
        CVE_VULNERABILITIES_TOTAL.clone(),
    );
    registry.register(
        "stellar_cve_scan_timestamp_seconds",
        "Unix timestamp of the last successful CVE scan per image.",
        CVE_SCAN_TIMESTAMP.clone(),
    );
    registry.register(
        "stellar_cve_vulnerable_pods_total",
        "Number of pods with CVE vulnerabilities per namespace and severity.",
        CVE_VULNERABLE_PODS_TOTAL.clone(),
    );
    registry.register(
        "stellar_cve_critical_alerts_total",
        "Total number of critical CVE alerts fired since operator start.",
        CVE_CRITICAL_ALERTS_TOTAL.clone(),
    );
}
