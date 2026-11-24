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
//! Kubernetes Compliance Scanning with kube-bench and Custom Policies
//!
//! This module provides automated CIS Kubernetes benchmark scanning,
//! custom policy validation, and compliance report generation.

use chrono::{DateTime, Utc};
use kube::runtime::controller::Action;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Compliance scan configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceScanConfig {
    /// CIS benchmark version to use
    pub cis_version: String,
    /// Benchmark profiles to run (e.g., "master", "worker", "etcd")
    pub profiles: Vec<String>,
    /// Custom policy paths
    pub custom_policies: Vec<String>,
    /// Scan interval in seconds
    pub scan_interval_seconds: u64,
    /// Namespaces to scan (empty = all)
    pub namespaces: Vec<String>,
    /// Exclude namespaces
    pub excluded_namespaces: Vec<String>,
    /// Enable auto-remediation
    pub auto_remediate: bool,
    /// Severity threshold for alerts
    pub alert_severity: SeverityThreshold,
    /// Output format for reports
    pub report_formats: Vec<ReportFormat>,
}

/// Severity threshold for compliance findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SeverityThreshold {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Report output formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportFormat {
    Json,
    Yaml,
    Csv,
    Pdf,
    Sarif,
}

/// Compliance finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceFinding {
    /// Unique finding ID
    pub id: String,
    /// Check ID from benchmark
    pub check_id: String,
    /// Check description
    pub description: String,
    /// Severity level
    pub severity: Severity,
    /// Affected resource
    pub resource: Option<ResourceRef>,
    /// Remediation steps
    pub remediation: Option<String>,
    /// References/links
    pub references: Vec<String>,
    /// Timestamp of finding
    pub timestamp: DateTime<Utc>,
    /// Framework this finding belongs to
    pub framework: String,
    /// Control ID
    pub control_id: Option<String>,
    /// Whether this finding has been remediated
    pub remediated: bool,
    /// Remediation timestamp
    pub remediated_at: Option<DateTime<Utc>>,
}

/// Severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Resource reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

/// Compliance scan result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceScanResult {
    /// Scan ID
    pub scan_id: String,
    /// Scan start time
    pub started_at: DateTime<Utc>,
    /// Scan completion time
    pub completed_at: DateTime<Utc>,
    /// Scan configuration used
    pub config: ComplianceScanConfig,
    /// All findings
    pub findings: Vec<ComplianceFinding>,
    /// Summary statistics
    pub summary: ScanSummary,
    /// Scan status
    pub status: ScanStatus,
}

/// Scan summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub total_checks: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub by_severity: HashMap<Severity, u32>,
    pub by_framework: HashMap<String, u32>,
}

/// Scan status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// CIS benchmark scanner
pub struct CisBenchmarkScanner {
    config: ComplianceScanConfig,
    client: Client,
    results: Arc<RwLock<HashMap<String, ComplianceScanResult>>>,
}

impl CisBenchmarkScanner {
    /// Create a new CIS benchmark scanner
    pub fn new(config: ComplianceScanConfig, client: Client) -> Arc<Self> {
        Arc::new(Self {
            config,
            client,
            results: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Run a full CIS benchmark scan
    pub async fn scan(&self) -> Result<ComplianceScanResult, anyhow::Error> {
        let scan_id = format!("scan-{}", Utc::now().format("%Y%m%d-%H%M%S"));
        let started_at = Utc::now();

        info!("Starting CIS benchmark scan: {}", scan_id);

        let mut findings = Vec::new();

        // Run kube-bench if available
        let kube_bench_findings = self.run_kube_bench().await?;
        findings.extend(kube_bench_findings);

        // Run custom policy checks
        let custom_findings = self.run_custom_policies().await?;
        findings.extend(custom_findings);

        // Run manifest validation
        let manifest_findings = self.validate_manifests().await?;
        findings.extend(manifest_findings);

        // Run runtime checks
        let runtime_findings = self.run_runtime_checks().await?;
        findings.extend(runtime_findings);

        let completed_at = Utc::now();

        let summary = self.generate_summary(&findings);
        let status = if findings.iter().any(|f| f.severity >= Severity::Critical) {
            ScanStatus::Failed
        } else {
            ScanStatus::Completed
        };

        let result = ComplianceScanResult {
            scan_id: scan_id.clone(),
            started_at,
            completed_at,
            config: self.config.clone(),
            findings,
            summary,
            status,
        };

        // Store result
        self.results
            .write()
            .await
            .insert(scan_id.clone(), result.clone());

        info!(
            "CIS benchmark scan completed: {} - {} checks, {} passed, {} failed",
            scan_id, result.summary.total_checks, result.summary.passed, result.summary.failed
        );

        Ok(result)
    }

    /// Run kube-bench scanner
    async fn run_kube_bench(&self) -> Result<Vec<ComplianceFinding>, anyhow::Error> {
        let mut findings = Vec::new();

        // Check if kube-bench is available
        let output = tokio::process::Command::new("kube-bench")
            .arg("--version")
            .output()
            .await;

        if output.is_err() {
            warn!("kube-bench not available, skipping CIS benchmark scan");
            return Ok(findings);
        }

        // Run kube-bench for each profile
        for profile in &self.config.profiles {
            let output = tokio::process::Command::new("kube-bench")
                .arg("--benchmark")
                .arg(&self.config.cis_version)
                .arg("--targets")
                .arg(profile)
                .arg("--json")
                .output()
                .await?;

            if output.status.success() {
                let json_output = String::from_utf8_lossy(&output.stdout);
                let profile_findings = self.parse_kube_bench_output(&json_output, profile)?;
                findings.extend(profile_findings);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("kube-bench failed for profile {}: {}", profile, stderr);
            }
        }

        Ok(findings)
    }

    /// Parse kube-bench JSON output
    fn parse_kube_bench_output(
        &self,
        json: &str,
        profile: &str,
    ) -> Result<Vec<ComplianceFinding>, anyhow::Error> {
        let mut findings = Vec::new();

        #[derive(Deserialize)]
        struct KubeBenchOutput {
            controls: Vec<KubeBenchControl>,
        }

        #[derive(Deserialize)]
        struct KubeBenchControl {
            id: String,
            description: String,
            tests: Vec<KubeBenchTest>,
        }

        #[derive(Deserialize)]
        struct KubeBenchTest {
            desc: String,
            result: String, // "PASS", "FAIL", "WARN", "INFO"
            remediation: Option<String>,
        }

        let output: KubeBenchOutput = serde_json::from_str(json)?;

        for control in output.controls {
            for test in control.tests {
                let severity = match test.result.as_str() {
                    "FAIL" => Severity::High,
                    "WARN" => Severity::Medium,
                    "INFO" => Severity::Low,
                    "PASS" => Severity::Info,
                    _ => Severity::Info,
                };

                let finding = ComplianceFinding {
                    id: format!("kube-bench-{}-{}", control.id, Uuid::new_v4()),
                    check_id: control.id.clone(),
                    description: format!("{}: {}", control.description, test.desc),
                    severity,
                    resource: None,
                    remediation: test.remediation,
                    references: vec![format!(
                        "https://www.cisecurity.org/benchmark/{}",
                        control.id
                    )],
                    timestamp: Utc::now(),
                    framework: format!(
                        "CIS Kubernetes {}",
                        control.id.split('.').next().unwrap_or("benchmark")
                    ),
                    control_id: Some(control.id.clone()),
                    remediated: false,
                    remediated_at: None,
                };

                findings.push(finding);
            }
        }

        Ok(findings)
    }

    /// Run custom policy checks
    async fn run_custom_policies(&self) -> Result<Vec<ComplianceFinding>, anyhow::Error> {
        let mut findings = Vec::new();

        for policy_path in &self.config.custom_policies {
            let policy_findings = self.evaluate_opa_policy(policy_path).await?;
            findings.extend(policy_findings);
        }

        Ok(findings)
    }

    /// Evaluate OPA/Rego policy
    async fn evaluate_opa_policy(
        &self,
        policy_path: &str,
    ) -> Result<Vec<ComplianceFinding>, anyhow::Error> {
        // Placeholder for OPA policy evaluation
        info!("Evaluating custom policy: {}", policy_path);
        Ok(vec![])
    }

    /// Validate Kubernetes manifests for compliance
    async fn validate_manifests(&self) -> Result<Vec<ComplianceFinding>, anyhow::Error> {
        let mut findings = Vec::new();

        // Check for common misconfigurations in cluster resources
        let pod_api: Api<k8s_openapi::api::core::v1::Pod> = Api::all(self.client.clone());
        let pods = pod_api.list(&ListParams::default()).await?;

        for pod in pods.items {
            let pod_name = pod.name_any();
            let namespace = pod.namespace().unwrap_or_default();

            // Check for privileged containers
            if let Some(spec) = &pod.spec {
                for container in &spec.containers {
                    if container
                        .security_context
                        .as_ref()
                        .and_then(|sc| sc.privileged)
                        .unwrap_or(false)
                    {
                        findings.push(ComplianceFinding {
                            id: format!("privileged-container-{}-{}", pod_name, Uuid::new_v4()),
                            check_id: "CUSTOM-001".to_string(),
                            description: format!("Container {} in pod {} runs as privileged", container.name, pod_name),
                            severity: Severity::High,
                            resource: Some(ResourceRef {
                                api_version: "v1".to_string(),
                                kind: "Pod".to_string(),
                                name: pod_name.clone(),
                                namespace: Some(namespace.clone()),
                            }),
                            remediation: Some("Remove privileged: true from container security context".to_string()),
                            references: vec!["https://kubernetes.io/docs/concepts/security/pod-security-standards/".to_string()],
                            timestamp: Utc::now(),
                            framework: "Custom".to_string(),
                            control_id: Some("CUSTOM-001".to_string()),
                            remediated: false,
                            remediated_at: None,
                        });
                    }
                }

                // Check for hostPath volumes
                if let Some(volumes) = &spec.volumes {
                    for volume in volumes {
                        if volume.host_path.is_some() {
                            findings.push(ComplianceFinding {
                                id: format!("hostpath-volume-{}-{}", pod_name, Uuid::new_v4()),
                                check_id: "CUSTOM-002".to_string(),
                                description: format!("Pod {} uses hostPath volume", pod_name),
                                severity: Severity::Medium,
                                resource: Some(ResourceRef {
                                    api_version: "v1".to_string(),
                                    kind: "Pod".to_string(),
                                    name: pod_name.clone(),
                                    namespace: Some(namespace.clone()),
                                }),
                                remediation: Some(
                                    "Remove hostPath volumes or use projected volumes".to_string(),
                                ),
                                references: vec![
                                    "https://kubernetes.io/docs/concepts/storage/volumes/#hostpath"
                                        .to_string(),
                                ],
                                timestamp: Utc::now(),
                                framework: "Custom".to_string(),
                                control_id: Some("CUSTOM-002".to_string()),
                                remediated: false,
                                remediated_at: None,
                            });
                        }
                    }
                }
            }

            // Check for hostNetwork
            if pod
                .spec
                .as_ref()
                .and_then(|s| s.host_network)
                .unwrap_or(false)
            {
                findings.push(ComplianceFinding {
                    id: format!("host-network-{}-{}", pod_name, Uuid::new_v4()),
                    check_id: "CUSTOM-003".to_string(),
                    description: format!("Pod {} uses hostNetwork", pod_name),
                    severity: Severity::High,
                    resource: Some(ResourceRef {
                        api_version: "v1".to_string(),
                        kind: "Pod".to_string(),
                        name: pod_name.clone(),
                        namespace: Some(namespace.clone()),
                    }),
                    remediation: Some("Remove hostNetwork: true from pod spec".to_string()),
                    references: vec![
                        "https://kubernetes.io/docs/concepts/security/pod-security-standards/"
                            .to_string(),
                    ],
                    timestamp: Utc::now(),
                    framework: "Custom".to_string(),
                    control_id: Some("CUSTOM-003".to_string()),
                    remediated: false,
                    remediated_at: None,
                });
            }
        }

        Ok(findings)
    }

    /// Run runtime checks
    async fn run_runtime_checks(&self) -> Result<Vec<ComplianceFinding>, anyhow::Error> {
        let mut findings = Vec::new();

        // Check node configurations
        // Check network policies
        // Check RBAC configurations
        // etc.

        Ok(findings)
    }

    /// Generate summary statistics
    fn generate_summary(&self, findings: &[ComplianceFinding]) -> ScanSummary {
        let mut summary = ScanSummary {
            total_checks: findings.len() as u32,
            passed: 0,
            failed: 0,
            skipped: 0,
            by_severity: HashMap::new(),
            by_framework: HashMap::new(),
        };

        for finding in findings {
            match finding.severity {
                Severity::Info => *summary.by_severity.entry(Severity::Info).or_insert(0) += 1,
                Severity::Low => *summary.by_severity.entry(Severity::Low).or_insert(0) += 1,
                Severity::Medium => *summary.by_severity.entry(Severity::Medium).or_insert(0) += 1,
                Severity::High => *summary.by_severity.entry(Severity::High).or_insert(0) += 1,
                Severity::Critical => {
                    *summary.by_severity.entry(Severity::Critical).or_insert(0) += 1
                }
            }

            *summary
                .by_framework
                .entry(finding.framework.clone())
                .or_insert(0) += 1;

            if matches!(finding.severity, Severity::High | Severity::Critical) {
                summary.failed += 1;
            } else {
                summary.passed += 1;
            }
        }

        summary
    }

    /// Get all scan results
    pub async fn get_results(&self) -> HashMap<String, ComplianceScanResult> {
        self.results.read().await.clone()
    }

    /// Get latest scan result
    pub async fn get_latest_result(&self) -> Option<ComplianceScanResult> {
        self.results
            .read()
            .await
            .values()
            .max_by_key(|r| r.started_at)
            .cloned()
    }

    /// Export scan result in specified format
    pub fn export_result(
        &self,
        result: &ComplianceScanResult,
        format: ReportFormat,
    ) -> Result<String, anyhow::Error> {
        match format {
            ReportFormat::Json => Ok(serde_json::to_string_pretty(result)?),
            ReportFormat::Yaml => Ok(serde_yaml::to_string(result)?),
            ReportFormat::Csv => self.export_csv(result),
            ReportFormat::Sarif => self.export_sarif(result),
            ReportFormat::Pdf => Err(anyhow::anyhow!("PDF export not implemented")),
        }
    }

    fn export_csv(&self, result: &ComplianceScanResult) -> Result<String, anyhow::Error> {
        let mut csv = String::new();
        csv.push_str("id,check_id,description,severity,framework,control_id,resource_kind,resource_name,resource_namespace,remediation,timestamp\n");

        for finding in &result.findings {
            let resource_kind = finding
                .resource
                .as_ref()
                .map(|r| r.kind.as_str())
                .unwrap_or("");
            let resource_name = finding
                .resource
                .as_ref()
                .map(|r| r.name.as_str())
                .unwrap_or("");
            let resource_namespace = finding
                .resource
                .as_ref()
                .and_then(|r| r.namespace.as_deref())
                .unwrap_or("");

            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                finding.id,
                finding.check_id,
                finding.description.replace(',', ";"),
                format!("{:?}", finding.severity),
                finding.framework,
                finding.control_id.as_deref().unwrap_or(""),
                resource_kind,
                finding
                    .resource
                    .as_ref()
                    .map(|r| r.name.as_str())
                    .unwrap_or(""),
                resource_namespace,
                finding
                    .remediation
                    .as_deref()
                    .unwrap_or("")
                    .replace(',', ";"),
                finding.timestamp.to_rfc3339(),
            ));
        }

        Ok(csv)
    }

    fn export_sarif(&self, result: &ComplianceScanResult) -> Result<String, anyhow::Error> {
        #[derive(Serialize)]
        struct SarifReport {
            version: String,
            runs: Vec<SarifRun>,
        }

        #[derive(Serialize)]
        struct SarifRun {
            tool: SarifTool,
            results: Vec<SarifResult>,
        }

        #[derive(Serialize)]
        struct SarifTool {
            driver: SarifDriver,
        }

        #[derive(Serialize)]
        struct SarifDriver {
            name: String,
            version: String,
            information_uri: String,
            rules: Vec<SarifRule>,
        }

        #[derive(Serialize)]
        struct SarifRule {
            id: String,
            name: String,
            short_description: SarifDescription,
            full_description: SarifDescription,
            default_configuration: SarifConfig,
        }

        #[derive(Serialize)]
        struct SarifDescription {
            text: String,
        }

        #[derive(Serialize)]
        struct SarifConfig {
            level: String,
        }

        #[derive(Serialize)]
        struct SarifResult {
            rule_id: String,
            level: String,
            message: SarifMessage,
            locations: Vec<SarifLocation>,
        }

        #[derive(Serialize)]
        struct SarifMessage {
            text: String,
        }

        #[derive(Serialize)]
        struct SarifLocation {
            physical_location: SarifPhysicalLocation,
        }

        #[derive(Serialize)]
        struct SarifPhysicalLocation {
            artifact_location: SarifArtifactLocation,
        }

        #[derive(Serialize)]
        struct SarifArtifactLocation {
            uri: String,
        }

        let rules: Vec<SarifRule> = result
            .findings
            .iter()
            .map(|f| SarifRule {
                id: f.check_id.clone(),
                name: f.check_id.clone(),
                short_description: SarifDescription {
                    text: f.description.clone(),
                },
                full_description: SarifDescription {
                    text: f.description.clone(),
                },
                default_configuration: SarifConfig {
                    level: format!("{:?}", f.severity).to_lowercase(),
                },
            })
            .collect();

        let results: Vec<SarifResult> = result
            .findings
            .iter()
            .map(|f| SarifResult {
                rule_id: f.check_id.clone(),
                level: match f.severity {
                    Severity::Critical => "error",
                    Severity::High => "error",
                    Severity::Medium => "warning",
                    Severity::Low => "note",
                    Severity::Info => "note",
                }
                .to_string(),
                message: SarifMessage {
                    text: f.description.clone(),
                },
                locations: f
                    .resource
                    .as_ref()
                    .map(|r| {
                        vec![SarifLocation {
                            physical_location: SarifPhysicalLocation {
                                artifact_location: SarifArtifactLocation {
                                    uri: format!("{}/{}/{}", r.api_version, r.kind, r.name),
                                },
                            },
                        }]
                    })
                    .unwrap_or_default(),
            })
            .collect();

        let report = SarifReport {
            version: "2.1.0".to_string(),
            runs: vec![SarifRun {
                tool: SarifTool {
                    driver: SarifDriver {
                        name: "stellar-k8s-compliance".to_string(),
                        version: "1.0.0".to_string(),
                        information_uri: "https://github.com/stellar-k8s/stellar-k8s".to_string(),
                        rules,
                    },
                },
                results,
            }],
        };

        Ok(serde_json::to_string_pretty(&report)?)
    }
}

/// Compliance monitor for continuous scanning
pub struct ComplianceMonitor {
    scanner: Arc<CisBenchmarkScanner>,
    config: ComplianceScanConfig,
}

impl ComplianceMonitor {
    pub fn new(scanner: Arc<CisBenchmarkScanner>, config: ComplianceScanConfig) -> Arc<Self> {
        Arc::new(Self { scanner, config })
    }

    /// Start continuous compliance monitoring
    pub async fn start(&self) {
        let mut interval =
            tokio::time::interval(Duration::from_secs(self.config.scan_interval_seconds));

        loop {
            interval.tick().await;

            info!("Running scheduled compliance scan...");

            match self.scanner.scan().await {
                Ok(result) => {
                    info!(
                        "Scheduled scan completed: {} checks, {} failed",
                        result.summary.total_checks, result.summary.failed
                    );

                    // Check for critical findings
                    if result
                        .findings
                        .iter()
                        .any(|f| f.severity >= Severity::Critical)
                    {
                        warn!("Critical compliance findings detected in scheduled scan!");
                        // Could trigger alerts here
                    }
                }
                Err(e) => {
                    error!("Scheduled compliance scan failed: {}", e);
                }
            }
        }
    }
}

impl Default for ComplianceScanConfig {
    fn default() -> Self {
        Self {
            cis_version: "1.15".to_string(),
            profiles: vec!["master".to_string(), "worker".to_string()],
            custom_policies: vec![],
            scan_interval_seconds: 3600, // 1 hour
            namespaces: vec![],
            excluded_namespaces: vec!["kube-system".to_string(), "kube-public".to_string()],
            auto_remediate: false,
            alert_severity: SeverityThreshold::High,
            report_formats: vec![ReportFormat::Json, ReportFormat::Csv],
        }
    }
}

use anyhow;
use reqwest;
use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
    }

    #[test]
    fn test_severity_threshold() {
        assert!(SeverityThreshold::Critical > SeverityThreshold::High);
        assert!(SeverityThreshold::High > SeverityThreshold::Medium);
    }

    #[test]
    fn test_scan_summary() {
        let mut summary = ScanSummary {
            total_checks: 100,
            passed: 90,
            failed: 10,
            skipped: 0,
            by_severity: HashMap::new(),
            by_framework: HashMap::new(),
        };

        summary.by_severity.insert(Severity::High, 5);
        summary.by_severity.insert(Severity::Medium, 5);

        assert_eq!(summary.total_checks, 100);
        assert_eq!(summary.failed, 10);
    }
}
