//! Operator startup self-test and diagnostics
//!
//! Runs a suite of preflight checks before the operator begins reconciling.
//! If critical checks fail, the operator exits with a descriptive error.

use kube::{
    api::{Api, ListParams},
    client::Client,
};
use tracing::{error, info, warn};

use crate::error::{Error, Result};

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
