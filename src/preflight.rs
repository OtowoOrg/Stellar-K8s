//! Operator startup self-test and diagnostics
//!
//! Runs a suite of preflight checks before the operator begins reconciling.
//! If critical checks fail, the operator exits with a descriptive error.
//!
//! # Two modes
//!
//! * **Local / onboarding** — call [`run_local_preflight`] to validate that
//!   all required CLI tools are installed before any cluster work starts.
//!   This is intentionally fast and printable so it can be wired into
//!   `make dev-setup` or an onboarding script.
//!
//! * **Cluster / runtime** — call [`run_preflight_checks`] (async) to probe
//!   the live Kubernetes API for CRDs, RBAC, namespaces, and lease access.

use kube::{
    api::{Api, ListParams},
    client::Client,
};
use serde::Deserialize;
use std::process::Command;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::error::{Error, Result};

/// Labels required by issue automation before opening new issues.
pub const REQUIRED_GH_LABELS: &[&str] = &["ci", "security", "stellar-wave"];

/// Tools that must be present for local development and CI to function.
/// Each entry is `(binary, install_hint)`.
pub const REQUIRED_LOCAL_TOOLS: &[(&str, &str)] = &[
    ("docker",  "Install Docker Engine: https://docs.docker.com/engine/install/"),
    ("kind",    "Install kind: https://kind.sigs.k8s.io/docs/user/quick-start/#installation"),
    ("kubectl", "Install kubectl: https://kubernetes.io/docs/tasks/tools/"),
    ("helm",    "Install Helm 3: https://helm.sh/docs/intro/install/"),
    ("cargo",   "Install Rust via rustup: https://rustup.rs/"),
];

const GH_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

/// Severity of a preflight check result
#[derive(Debug, Clone, PartialEq)]
pub enum CheckSeverity {
    /// Failure means the operator cannot function correctly
    Critical,
    /// Failure is a warning but the operator can still run
    Warning,
}

/// Result of a single preflight check
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: &'static str,
    pub passed: bool,
    pub severity: CheckSeverity,
    pub message: String,
}

impl CheckResult {
    fn pass(name: &'static str, severity: CheckSeverity, msg: impl Into<String>) -> Self {
        Self {
            name,
            passed: true,
            severity,
            message: msg.into(),
        }
    }

    fn fail(name: &'static str, severity: CheckSeverity, msg: impl Into<String>) -> Self {
        Self {
            name,
            passed: false,
            severity,
            message: msg.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

/// Run local development preflight checks.
///
/// Validates that every required CLI tool is present in `PATH` and prints an
/// actionable message for each missing one.  This is designed to run in under
/// a second so it is suitable as the first step of `make dev-setup` or any
/// onboarding script.
///
/// # Errors
/// Returns an error listing every missing tool so the developer can fix all
/// gaps in one pass rather than discovering them one by one.
///
/// # Example
/// ```no_run
/// use stellar_k8s::preflight::run_local_preflight;
/// run_local_preflight().expect("all required tools must be installed");
/// ```
pub fn run_local_preflight() -> Result<()> {
    run_local_preflight_with_tools(REQUIRED_LOCAL_TOOLS)
}

/// Inner implementation — accepts an explicit tool list so tests can inject
/// a subset without depending on the full `REQUIRED_LOCAL_TOOLS` array.
pub fn run_local_preflight_with_tools(tools: &[(&str, &str)]) -> Result<()> {
    info!("=== Local Development Preflight ===");

    let mut missing: Vec<String> = Vec::new();

    for (binary, hint) in tools {
        match check_tool_available(binary) {
            Ok(version) => {
                info!("  [PASS] {} — {}", binary, version.trim());
            }
            Err(_) => {
                error!("  [FAIL] {} not found in PATH", binary);
                error!("         → {}", hint);
                missing.push(format!("{binary}: {hint}"));
            }
        }
    }

    if missing.is_empty() {
        info!("=== Local preflight passed — all required tools present ===");
        Ok(())
    } else {
        Err(Error::ConfigError(format!(
            "Local preflight failed — {} tool(s) missing.\n\nInstall instructions:\n{}",
            missing.len(),
            missing
                .iter()
                .map(|m| format!("  • {m}"))
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }
}

/// Returns the first line of `<binary> --version` output on success, or an
/// error if the binary is not found / exits non-zero.
fn check_tool_available(binary: &str) -> std::result::Result<String, ()> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|_| ())?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Some tools print version to stderr (e.g. older kubectl)
        let version_line = if stdout.trim().is_empty() { &stderr } else { &stdout };
        Ok(version_line.lines().next().unwrap_or("").to_string())
    } else {
        Err(())
    }
}

/// Fast-fail preflight for GitHub CLI auth and label readiness.
///
/// This check is intentionally independent of Kubernetes connectivity so
/// issue-automation failures are caught early and explained clearly.
pub fn run_gh_label_preflight(repo: Option<&str>) -> Result<()> {
    let Some(repo) = repo.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(());
    };

    let deadline = Instant::now() + GH_PREFLIGHT_TIMEOUT;

    check_gh_auth(deadline)?;

    ensure_required_labels(repo, REQUIRED_GH_LABELS, deadline)?;

    Ok(())
}

fn check_gh_auth(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error::ConfigError(format!(
            "GitHub preflight timed out after {}s while checking auth",
            GH_PREFLIGHT_TIMEOUT.as_secs()
        )));
    }

    let output = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::ConfigError(
                    "GitHub CLI ('gh') was not found in PATH. Install from https://cli.github.com/"
                        .to_string(),
                )
            } else {
                Error::ConfigError(format!("failed to run `gh auth status`: {e}"))
            }
        })?;

    if Instant::now() >= deadline {
        return Err(Error::ConfigError(format!(
            "GitHub preflight timed out after {}s while checking auth",
            GH_PREFLIGHT_TIMEOUT.as_secs()
        )));
    }

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::ConfigError(format!(
            "GitHub auth preflight failed: {detail}. Run `gh auth login` and retry."
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
    if stdout.contains("not logged in") || stdout.contains("no token") {
        return Err(Error::ConfigError(
            "GitHub auth preflight failed: no active gh session found. Run `gh auth login` and retry.".to_string(),
        ));
    }

    Ok(())
}

fn ensure_required_labels(repo: &str, required: &[&str], deadline: Instant) -> Result<()> {
    if required.is_empty() {
        return Ok(());
    }

    if Instant::now() >= deadline {
        return Err(Error::ConfigError(format!(
            "GitHub preflight timed out after {}s while checking labels",
            GH_PREFLIGHT_TIMEOUT.as_secs()
        )));
    }

    let output = Command::new("gh")
        .args([
            "label", "list", "--repo", repo, "--json", "name", "--limit", "200",
        ])
        .output()
        .map_err(|e| Error::ConfigError(format!("failed to run `gh label list`: {e}")))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::ConfigError(format!(
            "GitHub label preflight failed for {repo}: {detail}"
        )));
    }

    let existing_labels = parse_label_names(&output.stdout)?;
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|label| !existing_labels.iter().any(|l| l == label))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    let mut unresolved = Vec::new();
    for label in missing {
        if Instant::now() >= deadline {
            return Err(Error::ConfigError(format!(
                "GitHub preflight timed out after {}s while creating missing labels",
                GH_PREFLIGHT_TIMEOUT.as_secs()
            )));
        }

        let status = Command::new("gh")
            .args([
                "label", "create", label, "--repo", repo, "--color", "ededed",
            ])
            .status()
            .map_err(|e| {
                Error::ConfigError(format!("failed to run `gh label create {label}`: {e}"))
            })?;

        if !status.success() {
            unresolved.push(label.to_string());
        }
    }

    if unresolved.is_empty() {
        Ok(())
    } else {
        Err(Error::ConfigError(format!(
            "GitHub label preflight failed for {repo}. Missing labels that could not be created: {}",
            unresolved.join(", ")
        )))
    }
}

fn parse_label_names(json_bytes: &[u8]) -> Result<Vec<String>> {
    let parsed: Vec<GhLabel> = serde_json::from_slice(json_bytes)
        .map_err(|e| Error::ConfigError(format!("failed to parse label JSON: {e}")))?;
    Ok(parsed
        .into_iter()
        .map(|l| l.name)
        .filter(|n| !n.is_empty())
        .collect())
}

/// Run all preflight checks and return the results.
pub async fn run_preflight_checks(client: &Client, namespace: &str) -> Vec<CheckResult> {
    let mut results = Vec::new();

    results.push(check_crd_installed(client).await);
    results.push(check_rbac_permissions(client, namespace).await);
    results.push(check_namespace_exists(client, namespace).await);
    results.push(check_leader_election_lease(client, namespace).await);

    results
}

/// Print a human-readable diagnostic summary to the log.
pub fn print_diagnostic_summary(results: &[CheckResult]) {
    info!("=== Operator Preflight Diagnostics ===");
    for r in results {
        let status = if r.passed { "PASS" } else { "FAIL" };
        let severity = match r.severity {
            CheckSeverity::Critical => "CRITICAL",
            CheckSeverity::Warning => "WARNING",
        };
        if r.passed {
            info!("  [{}] {} - {}", status, r.name, r.message);
        } else {
            match r.severity {
                CheckSeverity::Critical => {
                    error!("  [{}][{}] {} - {}", status, severity, r.name, r.message)
                }
                CheckSeverity::Warning => {
                    warn!("  [{}][{}] {} - {}", status, severity, r.name, r.message)
                }
            }
        }
    }

    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let critical_failures: Vec<_> = results
        .iter()
        .filter(|r| !r.passed && r.severity == CheckSeverity::Critical)
        .collect();

    info!(
        "=== Preflight Summary: {}/{} checks passed, {} critical failure(s) ===",
        passed,
        total,
        critical_failures.len()
    );
}

/// Evaluate results and return an error if any critical check failed.
pub fn evaluate_results(results: &[CheckResult]) -> Result<()> {
    let critical_failures: Vec<_> = results
        .iter()
        .filter(|r| !r.passed && r.severity == CheckSeverity::Critical)
        .collect();

    if critical_failures.is_empty() {
        return Ok(());
    }

    let messages: Vec<String> = critical_failures
        .iter()
        .map(|r| format!("{}: {}", r.name, r.message))
        .collect();

    Err(Error::ConfigError(format!(
        "Preflight checks failed — operator cannot start safely:\n{}",
        messages.join("\n")
    )))
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

/// Verify the StellarNode CRD is installed in the cluster.
async fn check_crd_installed(client: &Client) -> CheckResult {
    use crate::crd::StellarNode;

    let api: Api<StellarNode> = Api::all(client.clone());
    match api.list(&ListParams::default().limit(1)).await {
        Ok(_) => CheckResult::pass(
            "CRD Installed",
            CheckSeverity::Critical,
            "StellarNode CRD is present and accessible",
        ),
        Err(e) => CheckResult::fail(
            "CRD Installed",
            CheckSeverity::Critical,
            format!(
                "StellarNode CRD not found — install it with: kubectl apply -f config/crd/stellarnode-crd.yaml ({e})"
            ),
        ),
    }
}

/// Verify the operator has sufficient RBAC permissions by probing key API groups.
async fn check_rbac_permissions(client: &Client, namespace: &str) -> CheckResult {
    use k8s_openapi::api::apps::v1::Deployment;
    use k8s_openapi::api::core::v1::ConfigMap;

    // Probe: list Deployments in the operator namespace
    let deploy_api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    if let Err(e) = deploy_api.list(&ListParams::default().limit(1)).await {
        return CheckResult::fail(
            "RBAC Permissions",
            CheckSeverity::Critical,
            format!(
                "Cannot list Deployments in namespace '{namespace}' — check ClusterRole/RoleBinding ({e})"
            ),
        );
    }

    // Probe: list ConfigMaps in the operator namespace
    let cm_api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    if let Err(e) = cm_api.list(&ListParams::default().limit(1)).await {
        return CheckResult::fail(
            "RBAC Permissions",
            CheckSeverity::Critical,
            format!(
                "Cannot list ConfigMaps in namespace '{namespace}' — check ClusterRole/RoleBinding ({e})"
            ),
        );
    }

    CheckResult::pass(
        "RBAC Permissions",
        CheckSeverity::Critical,
        format!("Sufficient permissions verified in namespace '{namespace}'"),
    )
}

/// Verify the operator namespace exists.
async fn check_namespace_exists(client: &Client, namespace: &str) -> CheckResult {
    use k8s_openapi::api::core::v1::Namespace;

    let ns_api: Api<Namespace> = Api::all(client.clone());
    match ns_api.get(namespace).await {
        Ok(_) => CheckResult::pass(
            "Namespace Exists",
            CheckSeverity::Critical,
            format!("Namespace '{namespace}' exists"),
        ),
        Err(kube::Error::Api(e)) if e.code == 404 => CheckResult::fail(
            "Namespace Exists",
            CheckSeverity::Critical,
            format!(
                "Namespace '{namespace}' does not exist — create it with: kubectl create namespace {namespace}"
            ),
        ),
        Err(e) => CheckResult::fail(
            "Namespace Exists",
            CheckSeverity::Warning,
            format!("Could not verify namespace '{namespace}': {e}"),
        ),
    }
}

/// Verify the leader election Lease resource is accessible.
async fn check_leader_election_lease(client: &Client, namespace: &str) -> CheckResult {
    use k8s_openapi::api::coordination::v1::Lease;

    let lease_api: Api<Lease> = Api::namespaced(client.clone(), namespace);
    // We only need to be able to list/get leases — the lease may not exist yet.
    match lease_api.list(&ListParams::default().limit(1)).await {
        Ok(_) => CheckResult::pass(
            "Leader Election Lease",
            CheckSeverity::Critical,
            format!("Lease API accessible in namespace '{namespace}'"),
        ),
        Err(e) => CheckResult::fail(
            "Leader Election Lease",
            CheckSeverity::Critical,
            format!(
                "Cannot access Lease resources in namespace '{namespace}' — \
                 ensure the operator ServiceAccount has 'coordination.k8s.io' RBAC permissions ({e})"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_label_names_typical() {
        let json = br#"[{"name":"ci"},{"name":"security"}]"#;
        let labels = parse_label_names(json).expect("json should parse");
        assert_eq!(labels, vec!["ci".to_string(), "security".to_string()]);
    }

    #[test]
    fn parse_label_names_empty() {
        let json = br#"[]"#;
        let labels = parse_label_names(json).expect("json should parse");
        assert!(labels.is_empty());
    }

    #[test]
    fn parse_label_names_invalid_json() {
        let err = parse_label_names(b"not-json").expect_err("must fail for invalid json");
        assert!(err.to_string().contains("failed to parse label JSON"));
    }

    #[test]
    fn local_preflight_passes_with_no_tools() {
        // An empty tool list should always pass.
        let result = run_local_preflight_with_tools(&[]);
        assert!(result.is_ok(), "empty tool list should pass");
    }

    #[test]
    fn local_preflight_fails_for_nonexistent_tool() {
        let fake_tools: &[(&str, &str)] = &[
            ("__nonexistent_binary_xyz__", "Install from https://example.com"),
        ];
        let result = run_local_preflight_with_tools(fake_tools);
        assert!(result.is_err(), "missing binary must cause failure");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("__nonexistent_binary_xyz__"),
            "error message must name the missing binary"
        );
        assert!(
            msg.contains("https://example.com"),
            "error message must include the install hint"
        );
    }

    #[test]
    fn check_tool_available_finds_real_binary() {
        // Use `cargo` itself — it must be present since this is compiled by cargo.
        let result = check_tool_available("cargo");
        assert!(result.is_ok(), "cargo must be found in PATH");
        let version = result.unwrap();
        assert!(!version.is_empty(), "version string must not be empty");
    }
}
