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
/// tests/common/mod.rs
///
/// Shared test fixtures, RAII cleanup guards, and helpers for integration and
/// E2E test suites.  Every test that creates cluster resources must hold a
/// guard returned by one of the functions below so that cleanup is guaranteed
/// even when the test panics or returns early with `?`.
///
/// # Design goals (issue #906, extended in issue #1140)
/// - Deterministic creation *and* removal of fixtures.
/// - Cleanup runs in `Drop`, so it fires even on test failure.
/// - No cross-test coupling: each test gets its own namespace or unique
///   resource name and tears it down independently.
/// - Fixture data lives in `fixtures.rs`; guards live here.
/// - All cluster-required tests are gated behind `#[ignore]` so they never
///   run in unit-test mode and are never silently skipped.
use std::process::{Command, Stdio};

/// Re-export the fixtures module so integration tests can write
/// `use common::fixtures::init_container;` (and other live helpers).
pub mod fixtures;

// ---------------------------------------------------------------------------
// Namespace guard
// ---------------------------------------------------------------------------

/// RAII guard that deletes a Kubernetes namespace when dropped.
///
/// Use this to ensure test namespaces are removed even if the test fails.
///
/// ```no_run
/// let _ns = NamespaceGuard::create("my-test-ns");
/// // ... run test ...
/// // namespace is deleted here even on panic or early return
/// ```
pub struct NamespaceGuard {
    pub name: String,
}

impl NamespaceGuard {
    /// Idempotently creates the namespace and returns a guard that will delete
    /// it on drop.  Uses `--dry-run=client | kubectl apply` so the call is
    /// safe to repeat across parallel test runs on the same cluster.
    pub fn create(name: &str) -> Self {
        if let Ok(yaml) = run_kubectl_output(&[
            "create",
            "namespace",
            name,
            "--dry-run=client",
            "-o",
            "yaml",
        ]) {
            let _ = apply_manifest(&yaml);
        }

        Self {
            name: name.to_string(),
        }
    }
}

impl Drop for NamespaceGuard {
    fn drop(&mut self) {
        let _ = run_kubectl_quiet(&[
            "delete",
            "namespace",
            &self.name,
            "--ignore-not-found=true",
            "--wait=false",
        ]);
    }
}

// ---------------------------------------------------------------------------
// StellarNode guard
// ---------------------------------------------------------------------------

/// RAII guard that deletes a `StellarNode` resource when dropped.
///
/// Useful for tests that create individual resources without owning the whole
/// namespace lifecycle.
pub struct StellarNodeGuard {
    pub name: String,
    pub namespace: String,
}

impl StellarNodeGuard {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
        }
    }
}

impl Drop for StellarNodeGuard {
    fn drop(&mut self) {
        let _ = run_kubectl_quiet(&[
            "delete",
            "stellarnode",
            &self.name,
            "-n",
            &self.namespace,
            "--ignore-not-found=true",
            "--timeout=60s",
            "--wait=true",
        ]);
    }
}

// ---------------------------------------------------------------------------
// Operator manifest guard
// ---------------------------------------------------------------------------

/// RAII guard that deletes all resources defined in a YAML manifest when
/// dropped.  Suitable for cleaning up operator deployments, RBAC, and service
/// accounts created inline during a test.
pub struct ManifestGuard {
    pub manifest: String,
}

impl ManifestGuard {
    pub fn new(manifest: impl Into<String>) -> Self {
        Self {
            manifest: manifest.into(),
        }
    }
}

impl Drop for ManifestGuard {
    fn drop(&mut self) {
        let _ = run_kubectl_with_stdin_quiet(
            &["delete", "-f", "-", "--ignore-not-found=true"],
            &self.manifest,
        );
    }
}

// ---------------------------------------------------------------------------
// Composite cleanup guard
// ---------------------------------------------------------------------------

/// Composite guard that removes a set of `StellarNode`s, an operator manifest,
/// and a list of namespaces — in that order — when dropped.
///
/// This mirrors the lifecycle expected by the E2E test suite and is a drop-in
/// replacement for ad-hoc inline `Drop` impls scattered across test files.
pub struct E2eTestGuard {
    /// Names of `StellarNode` resources to delete, with their namespaces.
    stellar_nodes: Vec<(String, String)>,
    /// Raw YAML that was `kubectl apply`-ed to deploy the operator.
    operator_manifest: Option<String>,
    /// Namespaces to delete last (after resources are gone).
    namespaces: Vec<String>,
}

impl E2eTestGuard {
    pub fn new() -> Self {
        Self {
            stellar_nodes: Vec::new(),
            operator_manifest: None,
            namespaces: Vec::new(),
        }
    }

    /// Register a `StellarNode` for cleanup.
    pub fn track_node(mut self, name: impl Into<String>, namespace: impl Into<String>) -> Self {
        self.stellar_nodes.push((name.into(), namespace.into()));
        self
    }

    /// Register the operator manifest for cleanup.
    pub fn track_operator_manifest(mut self, manifest: impl Into<String>) -> Self {
        self.operator_manifest = Some(manifest.into());
        self
    }

    /// Register a namespace for cleanup.
    pub fn track_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespaces.push(namespace.into());
        self
    }
}

impl Default for E2eTestGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for E2eTestGuard {
    fn drop(&mut self) {
        // 1. Delete StellarNode resources first so finalizers can run cleanly.
        for (name, ns) in &self.stellar_nodes {
            let _ = run_kubectl_quiet(&[
                "delete",
                "stellarnode",
                name,
                "-n",
                ns,
                "--ignore-not-found=true",
                "--timeout=60s",
                "--wait=true",
            ]);
        }

        // 2. Delete the operator manifest (Deployment, RBAC, ServiceAccount).
        if let Some(manifest) = &self.operator_manifest {
            let _ = run_kubectl_with_stdin_quiet(
                &["delete", "-f", "-", "--ignore-not-found=true"],
                manifest,
            );
        }

        // 3. Delete namespaces last.
        for ns in &self.namespaces {
            let _ = run_kubectl_quiet(&["delete", "namespace", ns, "--ignore-not-found=true"]);
        }
    }
}

// ---------------------------------------------------------------------------
// General-purpose command helpers (consolidated from E2E test files)
// ---------------------------------------------------------------------------

/// Run an arbitrary command and return its trimmed stdout as a `String`.
///
/// Returns `Err` with diagnostic output when the command exits non-zero.
/// Propagates the `KUBECONFIG` environment variable if set.
pub fn run_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "command failed: {program} {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run an arbitrary command, suppressing stdout and stderr.
///
/// Returns `Ok(())` regardless of exit status so cleanup paths stay infallible.
pub fn run_cmd_quiet(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).output();
    Ok(())
}

/// Pipe `input` into `program <args>` via stdin, capturing output.
///
/// Returns `Ok(())` on success; returns `Err` with stdout/stderr on failure.
pub fn run_cmd_with_stdin(program: &str, args: &[&str], input: &str) -> Result<(), String> {
    use std::io::Write;

    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("stdin write failed: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("stdin flush failed: {e}"))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait failed: {e}"))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "command failed: {program} {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(())
}

/// Pipe `input` into `program <args>` via stdin, suppressing all output.
///
/// Returns `Ok(())` regardless of exit status.
pub fn run_cmd_with_stdin_quiet(program: &str, args: &[&str], input: &str) -> Result<(), String> {
    use std::io::Write;

    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    }
    let _ = child.wait();
    Ok(())
}

/// Apply a YAML manifest via `kubectl apply -f -`.
pub fn kubectl_apply(manifest: &str) -> Result<(), String> {
    run_cmd_with_stdin("kubectl", &["apply", "-f", "-"], manifest)
}

/// Poll `condition` every 3 seconds until it returns `Ok(true)` or `timeout`
/// elapses.
///
/// Returns `Err` with a diagnostic message when the timeout is exceeded.
pub fn wait_for<F>(
    label: &str,
    timeout: std::time::Duration,
    mut condition: F,
) -> Result<(), String>
where
    F: FnMut() -> Result<bool, String>,
{
    use std::thread::sleep;
    use std::time::Instant;

    let start = Instant::now();
    let mut attempts: u32 = 0;
    loop {
        if condition()? {
            return Ok(());
        }
        attempts += 1;
        if start.elapsed() > timeout {
            return Err(format!(
                "timeout while waiting for {label} after {timeout:?} (attempts={attempts})"
            ));
        }
        sleep(std::time::Duration::from_secs(3));
    }
}

/// Create a KinD cluster with the given `name` if it does not already exist.
pub fn ensure_kind_cluster(name: &str) -> Result<(), String> {
    let clusters = run_cmd("kind", &["get", "clusters"])?;
    if clusters.lines().any(|line| line.trim() == name) {
        return Ok(());
    }
    run_cmd("kind", &["create", "cluster", "--name", name])?;
    Ok(())
}

/// Parse a boolean-ish environment variable.  Recognises `"1"`, `"true"`,
/// `"yes"`, `"on"` (case-insensitive) as true; everything else falls back to
/// `default`.
pub fn env_true(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

/// Generate the operator deployment manifest (ServiceAccount + RBAC +
/// Deployment) for E2E tests.
///
/// When `watch_namespace` is `Some(ns)`, a namespace-scoped `Role` /
/// `RoleBinding` pair is created and `--watch-namespace` is passed to the
/// operator.  When `None`, cluster-wide `ClusterRole` / `ClusterRoleBinding`
/// resources are used.
pub fn operator_manifest(image: &str, watch_namespace: Option<&str>) -> String {
    let operator_name = "stellar-operator";
    let operator_namespace = "stellar-system";

    let rbac_kind = if watch_namespace.is_some() {
        "Role"
    } else {
        "ClusterRole"
    };
    let rbac_binding_kind = if watch_namespace.is_some() {
        "RoleBinding"
    } else {
        "ClusterRoleBinding"
    };
    let rbac_namespace = if let Some(ns) = watch_namespace {
        format!("\n  namespace: {ns}")
    } else {
        String::new()
    };

    let watch_arg = if let Some(ns) = watch_namespace {
        format!("\n            - --watch-namespace={ns}")
    } else {
        String::new()
    };

    format!(
        r#"---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {operator_name}
  namespace: {operator_namespace}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: {rbac_kind}
metadata:
  name: {operator_name}{rbac_namespace}
rules:
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes/status"]
    verbs: ["get", "update", "patch"]
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes/finalizers"]
    verbs: ["update"]
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["get", "list", "watch"]
  - apiGroups: [""]
    resources: ["services"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [""]
    resources: ["configmaps"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [""]
    resources: ["persistentvolumeclaims"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "list", "watch"]
  - apiGroups: ["apps"]
    resources: ["deployments"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: ["apps"]
    resources: ["statefulsets"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: ["policy"]
    resources: ["poddisruptionbudgets"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [""]
    resources: ["events"]
    verbs: ["create", "patch"]
  - apiGroups: ["coordination.k8s.io"]
    resources: ["leases"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: {rbac_binding_kind}
metadata:
  name: {operator_name}{rbac_namespace}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: {rbac_kind}
  name: {operator_name}
subjects:
  - kind: ServiceAccount
    name: {operator_name}
    namespace: {operator_namespace}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {operator_name}
  namespace: {operator_namespace}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: {operator_name}
  template:
    metadata:
      labels:
        app: {operator_name}
    spec:
      serviceAccountName: {operator_name}
      containers:
        - name: operator
          image: {image}
          imagePullPolicy: IfNotPresent
          args:
            - run
            - --namespace={operator_namespace} {watch_arg}
          env:
            - name: OPERATOR_NAMESPACE
              value: {operator_namespace}
"#
    )
}

// ---------------------------------------------------------------------------
// Low-level kubectl helpers
// ---------------------------------------------------------------------------

/// Run `kubectl <args>` without printing output.  Returns `Ok(())` if the
/// command exits zero; the error message is discarded so that cleanup in
/// `Drop` impls never panics.
pub fn run_kubectl_quiet(args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new("kubectl");
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    match cmd.stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("kubectl exited with status {s}")),
        Err(e) => Err(format!("failed to spawn kubectl: {e}")),
    }
}

/// Run `kubectl <args>` and return stdout as a trimmed `String`.
/// Stderr is discarded.  Returns `Err` if the command fails or stdout is
/// not valid UTF-8.
pub fn run_kubectl_output(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("kubectl");
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let output = cmd
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("failed to spawn kubectl: {e}"))?;
    if !output.status.success() {
        return Err(format!("kubectl exited with status {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("kubectl stdout is not UTF-8: {e}"))
}

/// Pipe `input` into `kubectl <args>` via stdin.  Returns `Ok(())` on
/// success; errors are swallowed so cleanup paths stay infallible.
pub fn run_kubectl_with_stdin_quiet(args: &[&str], input: &str) -> Result<(), String> {
    use std::io::Write;

    let mut cmd = Command::new("kubectl");
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn kubectl: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        let _ = stdin.flush();
    }
    let _ = child.wait();
    Ok(())
}

/// Apply a YAML manifest supplied as a string via `kubectl apply -f -`.
pub fn apply_manifest(yaml: &str) -> Result<(), String> {
    use std::io::Write;

    let mut cmd = Command::new("kubectl");
    cmd.args(["apply", "-f", "-"]);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn kubectl: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(yaml.as_bytes())
            .map_err(|e| format!("stdin write failed: {e}"))?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("kubectl wait failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kubectl apply exited with status {status}"))
    }
}

/// Returns `true` when `binary` is reachable in `PATH`.
pub fn tool_available(binary: &str) -> bool {
    Command::new(binary)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Skip the current test when any required tool is missing, printing a clear
/// message so CI logs are easy to understand.
///
/// Returns `true` when the test should be skipped (caller should return early).
pub fn skip_if_tools_missing(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools
        .iter()
        .copied()
        .filter(|t| !tool_available(t))
        .collect();
    if missing.is_empty() {
        return false;
    }
    eprintln!(
        "Skipping test: required tools not found in PATH: {}",
        missing.join(", ")
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_guard_struct_creation() {
        let guard = NamespaceGuard {
            name: "test-ns-guard".to_string(),
        };
        assert_eq!(guard.name, "test-ns-guard");
    }

    #[test]
    fn test_stellar_node_guard_creation() {
        let guard = StellarNodeGuard::new("node-1", "stellar-test");
        assert_eq!(guard.name, "node-1");
        assert_eq!(guard.namespace, "stellar-test");
    }

    #[test]
    fn test_manifest_guard_creation() {
        let guard =
            ManifestGuard::new("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: test-cm");
        assert!(guard.manifest.contains("test-cm"));
    }

    #[test]
    fn test_e2e_test_guard_builder() {
        let guard = E2eTestGuard::new()
            .track_node("node-a", "default")
            .track_operator_manifest("kind: Deployment")
            .track_namespace("test-namespace");

        assert_eq!(guard.stellar_nodes.len(), 1);
        assert_eq!(
            guard.stellar_nodes[0],
            ("node-a".to_string(), "default".to_string())
        );
        assert_eq!(guard.operator_manifest.as_deref(), Some("kind: Deployment"));
        assert_eq!(guard.namespaces, vec!["test-namespace".to_string()]);
    }

    #[test]
    fn test_skip_if_tools_missing_empty() {
        let skip = skip_if_tools_missing(&[]);
        assert!(!skip, "Empty tools list should not trigger skip");
    }
}
