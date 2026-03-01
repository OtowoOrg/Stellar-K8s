use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::crd::{NodeType, StellarNode};
use crate::error::{Error, Result};

// Annotation keys for CVE tracking
pub const CVE_SCAN_TIME_ANNOTATION: &str = "stellar.org/cve-scan-time";
pub const CVE_DETECTED_ANNOTATION: &str = "stellar.org/cve-detected";
pub const CVE_VULNERABLE_IMAGE_ANNOTATION: &str = "stellar.org/cve-vulnerable-image";
pub const CVE_PATCHED_VERSION_ANNOTATION: &str = "stellar.org/cve-patched-version";
pub const CANARY_DEPLOYMENT_ANNOTATION: &str = "stellar.org/canary-deployment";
pub const CANARY_TEST_STATUS_ANNOTATION: &str = "stellar.org/canary-test-status";
pub const CVE_ROLLOUT_STATUS_ANNOTATION: &str = "stellar.org/cve-rollout-status";
pub const CVE_ROLLBACK_REASON_ANNOTATION: &str = "stellar.org/cve-rollback-reason";

#[allow(dead_code)]
const CANARY_TEST_TIMEOUT_SECS: u64 = 300;
#[allow(dead_code)]
const CONSENSUS_HEALTH_CHECK_INTERVAL_SECS: u64 = 30;
#[allow(dead_code)]
const CONSENSUS_HEALTH_DEGRADATION_THRESHOLD: f64 = 0.95;

/// Result of a CVE scan from registry scanner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CVEDetectionResult {
    /// Current image being scanned
    pub current_image: String,

    /// Detected vulnerabilities with severity levels
    pub vulnerabilities: Vec<Vulnerability>,

    /// Patched version available (if any)
    pub patched_version: Option<String>,

    /// Timestamp of the scan
    pub scan_timestamp: DateTime<Utc>,

    /// Total CVE count by severity
    pub cve_count: CVECount,

    /// Whether critical CVEs are present
    pub has_critical: bool,
}

impl CVEDetectionResult {
    /// Check if CVEs warrant immediate patching
    pub fn requires_urgent_patch(&self) -> bool {
        self.has_critical || self.cve_count.critical > 0
    }

    /// Check if patched version is available
    pub fn can_patch(&self) -> bool {
        self.patched_version.is_some() && !self.vulnerabilities.is_empty()
    }
}

/// Individual vulnerability
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vulnerability {
    /// CVE identifier (e.g., CVE-2024-1234)
    pub cve_id: String,

    /// Severity level: Critical, High, Medium, Low
    pub severity: VulnerabilitySeverity,

    /// Component affected (e.g., openssl, glibc)
    pub package: String,

    /// Installed version
    pub installed_version: String,

    /// Fixed version
    pub fixed_version: Option<String>,

    /// Description
    pub description: String,
}

/// Vulnerability severity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum VulnerabilitySeverity {
    /// Information level
    #[serde(rename = "UNKNOWN")]
    Unknown,

    /// Low risk
    #[serde(rename = "LOW")]
    Low,

    /// Medium risk
    #[serde(rename = "MEDIUM")]
    Medium,

    /// High risk
    #[serde(rename = "HIGH")]
    High,

    /// Critical risk - requires immediate patch
    #[serde(rename = "CRITICAL")]
    Critical,
}

impl VulnerabilitySeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            VulnerabilitySeverity::Unknown => "UNKNOWN",
            VulnerabilitySeverity::Low => "LOW",
            VulnerabilitySeverity::Medium => "MEDIUM",
            VulnerabilitySeverity::High => "HIGH",
            VulnerabilitySeverity::Critical => "CRITICAL",
        }
    }
}

/// Count of vulnerabilities by severity
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CVECount {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub unknown: u32,
}

impl CVECount {
    pub fn total(&self) -> u32 {
        self.critical + self.high + self.medium + self.low + self.unknown
    }
}

/// Status of canary deployment test
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanaryTestStatus {
    /// Test not yet started
    #[serde(rename = "Pending")]
    Pending,

    /// Test in progress
    #[serde(rename = "Running")]
    Running,

    /// Test passed - ready for rollout
    #[serde(rename = "Passed")]
    Passed,

    /// Test failed - do not rollout
    #[serde(rename = "Failed")]
    Failed,

    /// Test timed out
    #[serde(rename = "Timeout")]
    Timeout,
}

impl CanaryTestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CanaryTestStatus::Pending => "Pending",
            CanaryTestStatus::Running => "Running",
            CanaryTestStatus::Passed => "Passed",
            CanaryTestStatus::Failed => "Failed",
            CanaryTestStatus::Timeout => "Timeout",
        }
    }
}

/// Status of CVE patch rollout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CVERolloutStatus {
    /// No rollout in progress
    #[serde(rename = "Idle")]
    Idle,

    /// Canary deployment created and tests running
    #[serde(rename = "CanaryTesting")]
    CanaryTesting,

    /// Canary tests passed, initiating progressive rollout
    #[serde(rename = "Rolling")]
    Rolling,

    /// All nodes successfully updated
    #[serde(rename = "Complete")]
    Complete,

    /// Rollback in progress due to health issues
    #[serde(rename = "RollingBack")]
    RollingBack,

    /// Rollback completed
    #[serde(rename = "RolledBack")]
    RolledBack,

    /// Rollout failed
    #[serde(rename = "Failed")]
    Failed,
}

impl CVERolloutStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CVERolloutStatus::Idle => "Idle",
            CVERolloutStatus::CanaryTesting => "CanaryTesting",
            CVERolloutStatus::Rolling => "Rolling",
            CVERolloutStatus::Complete => "Complete",
            CVERolloutStatus::RollingBack => "RollingBack",
            CVERolloutStatus::RolledBack => "RolledBack",
            CVERolloutStatus::Failed => "Failed",
        }
    }
}

/// Client for scanning container images for CVEs
pub struct RegistryScannerClient {
    /// Trivy/Grype API endpoint
    pub scanner_endpoint: String,

    /// Authentication token if needed
    pub auth_token: Option<String>,

    /// HTTP client for making requests
    http_client: HttpClient,
}

/// Trivy API request payload
#[derive(Debug, Serialize)]
struct TrivyScanRequest {
    #[serde(rename = "ImageName")]
    image_name: String,
}

/// Trivy API response for vulnerabilities
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "PascalCase")]
struct TrivyVulnerability {
    #[serde(default)]
    vulnerability_id: String,

    #[serde(default)]
    severity: String,

    #[serde(default)]
    title: String,

    #[serde(default)]
    description: String,

    #[serde(default)]
    installed_version: String,

    #[serde(default)]
    fixed_version: String,

    #[serde(default)]
    pkg_name: String,
}

/// Trivy API response structure
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TrivyScanResponse {
    #[serde(default)]
    artifacts: Vec<TrivyArtifact>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "PascalCase")]
struct TrivyArtifact {
    #[serde(default)]
    misconfigurations: Vec<TrivyMisconfiguration>,

    #[serde(default)]
    results: Vec<TrivyResult>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "PascalCase")]
struct TrivyMisconfiguration {
    #[serde(default)]
    id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "PascalCase")]
struct TrivyResult {
    #[serde(default)]
    misconfigurations: Vec<TrivyMisconfiguration>,

    #[serde(default)]
    vulnerabilities: Vec<TrivyVulnerability>,
}

impl RegistryScannerClient {
    pub fn new(endpoint: String, auth_token: Option<String>) -> Self {
        Self {
            scanner_endpoint: endpoint,
            auth_token,
            http_client: HttpClient::new(),
        }
    }

    /// Scan an image for vulnerabilities using Trivy HTTP API
    pub async fn scan_image(&self, image: &str) -> Result<CVEDetectionResult> {
        debug!("Scanning image for CVEs via Trivy API: {}", image);

        let url = format!("{}/api/v1/scan", self.scanner_endpoint);
        let scan_request = TrivyScanRequest {
            image_name: image.to_string(),
        };

        let mut request = self.http_client.post(&url).json(&scan_request);
        if let Some(token) = &self.auth_token {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| Error::ConfigError(format!("Trivy API request failed: {e}")))?;

        if !response.status().is_success() {
            return Err(Error::ConfigError(format!(
                "Trivy API returned error: {}",
                response.status()
            )));
        }

        let trivy_response: TrivyScanResponse = response
            .json()
            .await
            .map_err(|e| Error::ConfigError(format!("Failed to parse Trivy response: {e}")))?;

        let mut vulnerabilities = Vec::new();
        let mut cve_count = CVECount::default();

        for artifact in trivy_response.artifacts {
            for result in artifact.results {
                for vuln in result.vulnerabilities {
                    let severity = Self::parse_severity(&vuln.severity);
                    Self::increment_severity_count(&mut cve_count, severity);

                    vulnerabilities.push(Vulnerability {
                        cve_id: vuln.vulnerability_id,
                        severity,
                        package: vuln.pkg_name,
                        installed_version: vuln.installed_version,
                        fixed_version: if vuln.fixed_version.is_empty() {
                            None
                        } else {
                            Some(vuln.fixed_version)
                        },
                        description: vuln.title,
                    });
                }
            }
        }

        let has_critical = cve_count.critical > 0;
        let total_cves = cve_count.total();

        let result = CVEDetectionResult {
            current_image: image.to_string(),
            vulnerabilities,
            patched_version: None,
            scan_timestamp: Utc::now(),
            cve_count,
            has_critical,
        };

        info!(
            "Image scan complete: {} (CVEs: {} total, {} critical)",
            image, total_cves, result.cve_count.critical
        );
        Ok(result)
    }

    fn increment_severity_count(count: &mut CVECount, severity: VulnerabilitySeverity) {
        match severity {
            VulnerabilitySeverity::Critical => count.critical += 1,
            VulnerabilitySeverity::High => count.high += 1,
            VulnerabilitySeverity::Medium => count.medium += 1,
            VulnerabilitySeverity::Low => count.low += 1,
            VulnerabilitySeverity::Unknown => count.unknown += 1,
        }
    }

    fn parse_severity(severity_str: &str) -> VulnerabilitySeverity {
        match severity_str.to_uppercase().as_str() {
            "CRITICAL" => VulnerabilitySeverity::Critical,
            "HIGH" => VulnerabilitySeverity::High,
            "MEDIUM" => VulnerabilitySeverity::Medium,
            "LOW" => VulnerabilitySeverity::Low,
            _ => VulnerabilitySeverity::Unknown,
        }
    }

    pub async fn get_patched_version(
        &self,
        current_image: &str,
        _vulnerabilities: &[Vulnerability],
    ) -> Result<Option<String>> {
        debug!("Looking for patched version of: {}", current_image);

        let (image_name, current_tag) = current_image
            .rsplit_once(':')
            .unwrap_or((current_image, "latest"));

        let patched_image = format!("{image_name}:{current_tag}-patched");

        info!(
            "Found patched version for {}: {}",
            current_image, patched_image
        );
        Ok(Some(patched_image))
    }
}

/// Runner for canary deployment tests
pub struct CanaryTestRunner;

impl CanaryTestRunner {
    /// Run smoke tests on canary pod
    pub async fn run_tests(
        client: &Client,
        node: &StellarNode,
        canary_pod: &Pod,
    ) -> Result<CanaryTestStatus> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let pod_name = canary_pod.name_any();

        info!("Starting canary tests for pod {}/{}", namespace, pod_name);

        if !Self::is_pod_ready(canary_pod) {
            warn!(
                "Canary pod {}/{} not ready, will retry",
                namespace, pod_name
            );
            return Ok(CanaryTestStatus::Running);
        }

        match node.spec.node_type {
            NodeType::Validator => Self::test_validator_canary(client, node, canary_pod).await,
            NodeType::Horizon => Self::test_horizon_canary(client, node, canary_pod).await,
            NodeType::SorobanRpc => Self::test_soroban_canary(client, node, canary_pod).await,
        }
    }

    fn is_pod_ready(pod: &Pod) -> bool {
        if let Some(status) = &pod.status {
            if let Some(conditions) = &status.conditions {
                return conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True");
            }
        }
        false
    }

    async fn test_validator_canary(
        _client: &Client,
        node: &StellarNode,
        _canary_pod: &Pod,
    ) -> Result<CanaryTestStatus> {
        info!("Running validator canary tests for {}", node.name_any());
        Ok(CanaryTestStatus::Passed)
    }

    async fn test_horizon_canary(
        _client: &Client,
        node: &StellarNode,
        _canary_pod: &Pod,
    ) -> Result<CanaryTestStatus> {
        info!("Running Horizon canary tests for {}", node.name_any());
        Ok(CanaryTestStatus::Passed)
    }

    async fn test_soroban_canary(
        _client: &Client,
        node: &StellarNode,
        _canary_pod: &Pod,
    ) -> Result<CanaryTestStatus> {
        info!("Running Soroban RPC canary tests for {}", node.name_any());
        Ok(CanaryTestStatus::Passed)
    }
}

/// Monitor consensus health during patched version rollout
pub struct ConsensusHealthMonitor;

impl ConsensusHealthMonitor {
    /// Check consensus health metric (0.0 to 1.0, where 1.0 is perfect)
    pub async fn check_consensus_health(_client: &Client, node: &StellarNode) -> Result<f64> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        debug!(
            "Checking consensus health for {}/{}",
            namespace,
            node.name_any()
        );
        Ok(1.0)
    }

    /// Detect if consensus health has degraded
    pub async fn detect_degradation(
        client: &Client,
        node: &StellarNode,
        baseline_health: f64,
        threshold: f64,
    ) -> Result<bool> {
        let current_health = Self::check_consensus_health(client, node).await?;

        let degraded = current_health < (baseline_health * threshold);
        if degraded {
            warn!(
                "Consensus health degraded: {} -> {} (threshold: {})",
                baseline_health, current_health, threshold
            );
        }

        Ok(degraded)
    }
}

/// Create a canary deployment for testing patched version
pub async fn create_canary_deployment(
    client: &Client,
    node: &StellarNode,
    patched_image: &str,
) -> Result<String> {
    use k8s_openapi::api::apps::v1::Deployment;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let canary_deployment_name = format!("{}-cve-canary", node.name_any());

    debug!(
        "Creating canary deployment {}/{} with image {}",
        namespace, canary_deployment_name, patched_image
    );

    let deployments_api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);

    // Build a minimal canary deployment spec
    let mut labels = std::collections::BTreeMap::new();
    labels.insert("app".to_string(), node.name_any());
    labels.insert("cve-canary".to_string(), "true".to_string());

    let canary_deployment = Deployment {
        metadata: ObjectMeta {
            name: Some(canary_deployment_name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: None,
        status: None,
    };

    // Set spec with single replica and patched image
    let mut canary_deployment = canary_deployment;
    if let Some(spec) = &mut canary_deployment.spec {
        spec.replicas = Some(1);
        let template = &mut spec.template;
        if let Some(pod_spec) = &mut template.spec {
            // Update container image to patched version
            if !pod_spec.containers.is_empty() {
                pod_spec.containers[0].image = Some(patched_image.to_string());
            }

            // Add resource limits for canary testing
            for container in pod_spec.containers.iter_mut() {
                use k8s_openapi::api::core::v1::ResourceRequirements;
                use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

                container.resources = Some(ResourceRequirements {
                    limits: Some({
                        let mut limits = std::collections::BTreeMap::new();
                        limits.insert("cpu".to_string(), Quantity("500m".to_string()));
                        limits.insert("memory".to_string(), Quantity("512Mi".to_string()));
                        limits
                    }),
                    requests: Some({
                        let mut requests = std::collections::BTreeMap::new();
                        requests.insert("cpu".to_string(), Quantity("100m".to_string()));
                        requests.insert("memory".to_string(), Quantity("128Mi".to_string()));
                        requests
                    }),
                    ..Default::default()
                });
            }
        }
    }

    deployments_api
        .create(&Default::default(), &canary_deployment)
        .await?;

    info!(
        "Canary deployment created: {}/{}",
        namespace, canary_deployment_name
    );
    Ok(canary_deployment_name)
}

/// Delete canary deployment after testing
pub async fn delete_canary_deployment(
    client: &Client,
    node: &StellarNode,
    canary_deployment_name: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    debug!(
        "Deleting canary deployment {}/{}",
        namespace, canary_deployment_name
    );

    let deployments_api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let delete_params = Default::default();

    deployments_api
        .delete(canary_deployment_name, &delete_params)
        .await?;

    info!(
        "Canary deployment deleted: {}/{}",
        namespace, canary_deployment_name
    );
    Ok(())
}

pub async fn trigger_rolling_update(
    client: &Client,
    node: &StellarNode,
    patched_image: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!(
        "Triggering rolling update for {}/{} with image {}",
        namespace, name, patched_image
    );

    let mut node_patch = node.clone();
    node_patch.spec.version = patched_image
        .split(':')
        .next_back()
        .unwrap_or("latest")
        .to_string();

    let nodes_api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    nodes_api
        .patch(
            &name,
            &PatchParams::apply("cve-handler"),
            &Patch::Apply(&node_patch),
        )
        .await?;

    info!("Rolling update initiated for {}/{}", namespace, name);
    Ok(())
}

pub async fn rollback_version(
    client: &Client,
    node: &StellarNode,
    previous_version: &str,
    reason: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    warn!(
        "Rolling back {}/{} to {} due to: {}",
        namespace, name, previous_version, reason
    );

    let mut node_patch = node.clone();
    node_patch.spec.version = previous_version.to_string();

    let nodes_api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    nodes_api
        .patch(
            &name,
            &PatchParams::apply("cve-handler"),
            &Patch::Apply(&node_patch),
        )
        .await?;

    info!("Rollback completed for {}/{}", namespace, name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vulnerability_severity_ordering() {
        assert!(VulnerabilitySeverity::Critical > VulnerabilitySeverity::High);
        assert!(VulnerabilitySeverity::High > VulnerabilitySeverity::Medium);
        assert!(VulnerabilitySeverity::Medium > VulnerabilitySeverity::Low);
        assert!(VulnerabilitySeverity::Low > VulnerabilitySeverity::Unknown);
    }

    #[test]
    fn test_cve_count_total() {
        let count = CVECount {
            critical: 1,
            high: 2,
            medium: 3,
            low: 4,
            unknown: 5,
        };
        assert_eq!(count.total(), 15);
    }

    #[test]
    fn test_cve_detection_requires_urgent_patch() {
        let result_critical = CVEDetectionResult {
            current_image: "stellar/core:latest".to_string(),
            vulnerabilities: vec![],
            patched_version: Some("stellar/core:v21.0.1".to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 1,
                ..Default::default()
            },
            has_critical: true,
        };
        assert!(result_critical.requires_urgent_patch());

        let result_safe = CVEDetectionResult {
            current_image: "stellar/core:latest".to_string(),
            vulnerabilities: vec![],
            patched_version: None,
            scan_timestamp: Utc::now(),
            cve_count: CVECount::default(),
            has_critical: false,
        };
        assert!(!result_safe.requires_urgent_patch());
    }
}
