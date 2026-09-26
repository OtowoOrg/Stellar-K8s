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
//! Immutable Infrastructure Verification at Node Boot
//!
//! Verifies node image integrity and expected software bill of materials (SBOM)
//! at boot before the node joins the cluster.
//!
//! # Design
//!
//! Implemented as a systemd unit running before kubelet, failing closed with
//! a clear node condition that explains any refusal.
//!
//! ## Acceptance Criteria (from #1510)
//! - [ ] Tampered node image prevented from joining (all trials)
//! - [ ] Verification adds under 15s to node boot
//! - [ ] Node condition explains any refusal
//! - [ ] Expected-image changes rolled out via the same pipeline

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::bootstrap_verify::{run_bootstrap_verification, CheckResult, CheckSeverity};

/// Expected node image specification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedImageSpec {
    /// Expected container image digest (sha256:...).
    pub image_digest: String,

    /// Expected kernel version pattern.
    #[serde(default)]
    pub kernel_version: String,

    /// Expected OS release identifier.
    #[serde(default)]
    pub os_release: String,

    /// Allowlisted package set (name -> version constraint).
    #[serde(default)]
    pub allowlisted_packages: HashMap<String, String>,

    /// Denylisted packages (must not be present).
    #[serde(default)]
    pub denylisted_packages: Vec<String>,
}

/// Verification result for a single check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub duration_ms: u64,
    pub severity: CheckSeverity,
}

/// Overall node boot verification result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBootVerificationResult {
    pub node_name: String,
    pub timestamp: String,
    pub overall_passed: bool,
    pub total_duration_ms: u64,
    pub checks: Vec<VerificationCheck>,
    pub image_spec_version: String,
}

/// Node condition for Kubernetes API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeCondition {
    pub condition_type: String,
    pub status: String, // "True", "False", "Unknown"
    pub reason: String,
    pub message: String,
    pub last_transition_time: String,
}

/// Verify the booted node image against the expected spec.
pub async fn verify_node_boot(spec: &ExpectedImageSpec) -> NodeBootVerificationResult {
    let start = Instant::now();
    let mut checks = Vec::new();

    // 1. Verify image digest
    checks.push(verify_image_digest(spec).await);

    // 2. Verify kernel version
    checks.push(verify_kernel_version(spec).await);

    // 3. Verify OS release
    checks.push(verify_os_release(spec).await);

    // 4. Verify allowlisted packages (SBOM)
    checks.extend(verify_allowlisted_packages(spec).await);

    // 5. Verify no denylisted packages
    checks.push(verify_denylisted_packages(spec).await);

    // 6. Run existing bootstrap verification (tools, docker, etc.)
    checks.extend(run_bootstrap_verification().into_iter().map(|c| VerificationCheck {
        name: c.name.to_string(),
        passed: c.passed,
        message: c.message,
        duration_ms: 0, // bootstrap doesn't track per-check duration
        severity: c.severity,
    }));

    let overall_passed = checks.iter().all(|c| c.passed);
    let total_duration_ms = start.elapsed().as_millis() as u64;

    NodeBootVerificationResult {
        node_name: hostname::get().unwrap_or_default().to_string_lossy().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        overall_passed,
        total_duration_ms,
        checks,
        image_spec_version: spec.image_digest.clone(),
    }
}

async fn verify_image_digest(spec: &ExpectedImageSpec) -> VerificationCheck {
    let start = Instant::now();
    // Read the image digest from the container runtime
    // For containerd: `crictl inspecti <image>` or read from /var/lib/containerd
    // For Docker: `docker inspect --format='{{.Id}}' <image>`
    // This is a simplified implementation - production would query the CRI

    let expected = spec.image_digest.trim_start_matches("sha256:");
    let actual = read_current_image_digest().await.unwrap_or_default();

    let passed = actual.trim_start_matches("sha256:") == expected;
    let message = if passed {
        format!("Image digest matches: {}", expected)
    } else {
        format!("Image digest mismatch! Expected: {}, Got: {}", expected, actual)
    };

    VerificationCheck {
        name: "image_digest".into(),
        passed,
        message,
        duration_ms: start.elapsed().as_millis() as u64,
        severity: CheckSeverity::Critical,
    }
}

async fn read_current_image_digest() -> Option<String> {
    // Try to read from containerd's image store
    // This is a placeholder - real implementation would use CRI client
    if let Ok(output) = Command::new("crictl").args(["inspecti", "--output", "json", "$(crictl images -q | head -1)"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(start) = stdout.find("\"id\": \"sha256:") {
                let rest = &stdout[start + 9..];
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

async fn verify_kernel_version(spec: &ExpectedImageSpec) -> VerificationCheck {
    let start = Instant::now();
    let expected = &spec.kernel_version;

    let actual = std::fs::read_to_string("/proc/version")
        .ok()
        .and_then(|s| s.split_whitespace().nth(2).map(|s| s.to_string()))
        .unwrap_or_default();

    let passed = expected.is_empty() || actual.starts_with(expected);
    let message = if passed {
        format!("Kernel version OK: {}", actual)
    } else {
        format!("Kernel version mismatch! Expected prefix: {}, Got: {}", expected, actual)
    };

    VerificationCheck {
        name: "kernel_version".into(),
        passed,
        message,
        duration_ms: start.elapsed().as_millis() as u64,
        severity: CheckSeverity::Critical,
    }
}

async fn verify_os_release(spec: &ExpectedImageSpec) -> VerificationCheck {
    let start = Instant::now();
    let expected = &spec.os_release;

    let actual = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VERSION_ID="))
                .map(|l| l.trim_start_matches("VERSION_ID=").trim_matches('"').to_string())
        })
        .unwrap_or_default();

    let passed = expected.is_empty() || actual == expected;
    let message = if passed {
        format!("OS release OK: {}", actual)
    } else {
        format!("OS release mismatch! Expected: {}, Got: {}", expected, actual)
    };

    VerificationCheck {
        name: "os_release".into(),
        passed,
        message,
        duration_ms: start.elapsed().as_millis() as u64,
        severity: CheckSeverity::Critical,
    }
}

async fn verify_allowlisted_packages(spec: &ExpectedImageSpec) -> Vec<VerificationCheck> {
    let mut checks = Vec::new();
    let start = Instant::now();

    // Read installed packages from package manager (rpm/dpkg/apk)
    let installed = read_installed_packages().await;

    for (pkg, version_constraint) in &spec.allowlisted_packages {
        let check_start = Instant::now();
        let installed_version = installed.get(pkg);

        let (passed, message) = match installed_version {
            Some(v) if version_constraint.is_empty() || version_satisfies(v, version_constraint) => (
                true,
                format!("Package {} version {} satisfies {}", pkg, v, version_constraint),
            ),
            Some(v) => (
                false,
                format!("Package {} version {} does not satisfy {}", pkg, v, version_constraint),
            ),
            None => (
                false,
                format!("Required package {} not installed", pkg),
            ),
        };

        checks.push(VerificationCheck {
            name: format!("package_{}", pkg),
            passed,
            message,
            duration_ms: check_start.elapsed().as_millis() as u64,
            severity: CheckSeverity::Critical,
        });
    }

    info!("Allowlisted package verification took {}ms", start.elapsed().as_millis());
    checks
}

async fn verify_denylisted_packages(spec: &ExpectedImageSpec) -> VerificationCheck {
    let start = Instant::now();
    let installed = read_installed_packages().await;

    let mut found = Vec::new();
    for pkg in &spec.denylisted_packages {
        if installed.contains_key(pkg) {
            found.push(pkg.clone());
        }
    }

    let passed = found.is_empty();
    let message = if passed {
        "No denylisted packages found".into()
    } else {
        format!("Denylisted packages found: {}", found.join(", "))
    };

    VerificationCheck {
        name: "denylisted_packages".into(),
        passed,
        message,
        duration_ms: start.elapsed().as_millis() as u64,
        severity: CheckSeverity::Critical,
    }
}

async fn read_installed_packages() -> HashMap<String, String> {
    let mut packages = HashMap::new();

    // Try rpm
    if let Ok(output) = Command::new("rpm").args(["-qa", "--queryformat", "%{NAME} %{VERSION}-%{RELEASE}\n"]).output() {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let parts: Vec<_> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    packages.insert(parts[0].into(), parts[1].into());
                }
            }
        }
    }

    // Try dpkg
    if packages.is_empty() {
        if let Ok(output) = Command::new("dpkg-query").args(["-W", "-f=${Package} ${Version}\n"]).output() {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let parts: Vec<_> = line.splitn(2, ' ').collect();
                    if parts.len() == 2 {
                        packages.insert(parts[0].into(), parts[1].into());
                    }
                }
            }
        }
    }

    // Try apk
    if packages.is_empty() {
        if let Ok(output) = Command::new("apk").args(["info", "-v"]).output() {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let parts: Vec<_> = line.splitn(2, ' ').collect();
                    if parts.len() == 2 {
                        packages.insert(parts[0].into(), parts[1].into());
                    }
                }
            }
        }
    }

    packages
}

fn version_satisfies(installed: &str, constraint: &str) -> bool {
    // Simplified: exact match or prefix match for constraints like "1.2.*"
    if constraint.ends_with('*') {
        let prefix = constraint.trim_end_matches('*');
        installed.starts_with(prefix)
    } else {
        installed == constraint
    }
}

/// Generate Kubernetes node condition from verification result.
pub fn generate_node_condition(result: &NodeBootVerificationResult) -> NodeCondition {
    let (status, reason, message) = if result.overall_passed {
        ("True", "BootVerificationPassed", "All boot verification checks passed".to_string())
    } else {
        let failed: Vec<_> = result.checks.iter().filter(|c| !c.passed).collect();
        let msg = format!("Boot verification failed: {} checks failed", failed.len());
        ("False", "BootVerificationFailed", msg)
    };

    NodeCondition {
        condition_type: "BootVerified".into(),
        status: status.into(),
        reason: reason.into(),
        message,
        last_transition_time: result.timestamp.clone(),
    }
}

/// Write verification result to a file for kubelet/node-problem-detector to pick up.
pub fn write_verification_result(result: &NodeBootVerificationResult, path: &Path) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(result)?;
    fs::write(path, json)
}

/// Systemd unit generator for the boot verification service.
pub fn generate_systemd_unit() -> String {
    r#"[Unit]
Description=Stellar-K8s Node Boot Verification
Documentation=https://github.com/OtowoOrg/Stellar-K8s
DefaultDependencies=no
After=local-fs.target network-online.target
Before=kubelet.service
ConditionPathExists=!/run/node-boot-verified

[Service]
Type=oneshot
ExecStart=/usr/local/bin/stellar-node-boot-verify
RemainAfterExit=yes
TimeoutSec=15
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=kubelet.service
"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expected_image_spec_default() {
        let spec = ExpectedImageSpec::default();
        assert!(spec.image_digest.is_empty());
        assert!(spec.allowlisted_packages.is_empty());
    }

    #[test]
    fn test_version_satisfies_exact() {
        assert!(version_satisfies("1.2.3", "1.2.3"));
        assert!(!version_satisfies("1.2.3", "1.2.4"));
    }

    #[test]
    fn test_version_satisfies_wildcard() {
        assert!(version_satisfies("1.2.3", "1.2.*"));
        assert!(version_satisfies("1.2.10", "1.2.*"));
        assert!(!version_satisfies("1.3.0", "1.2.*"));
    }

    #[test]
    fn test_node_condition_generation() {
        let result = NodeBootVerificationResult {
            node_name: "test-node".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
            overall_passed: true,
            total_duration_ms: 100,
            checks: vec![],
            image_spec_version: "sha256:abc".into(),
        };
        let condition = generate_node_condition(&result);
        assert_eq!(condition.status, "True");
        assert_eq!(condition.condition_type, "BootVerified");

        let failed_result = NodeBootVerificationResult {
            overall_passed: false,
            checks: vec![VerificationCheck { passed: false, ..Default::default() }],
            ..result
        };
        let condition = generate_node_condition(&failed_result);
        assert_eq!(condition.status, "False");
        assert_eq!(condition.reason, "BootVerificationFailed");
    }
}

// Need Default for VerificationCheck in tests
impl Default for VerificationCheck {
    fn default() -> Self {
        Self {
            name: "test".into(),
            passed: true,
            message: "test".into(),
            duration_ms: 0,
            severity: CheckSeverity::Critical,
        }
    }
}