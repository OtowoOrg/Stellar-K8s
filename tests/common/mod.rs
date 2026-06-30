/// tests/common/mod.rs
///
/// Shared test fixtures, RAII cleanup guards, and helpers for integration and
/// E2E test suites.  Every test that creates cluster resources must hold a
/// guard returned by one of the functions below so that cleanup is guaranteed
/// even when the test panics or returns early with `?`.
///
/// # Design goals (issue #906)
/// - Deterministic creation *and* removal of fixtures.
/// - Cleanup runs in `Drop`, so it fires even on test failure.
/// - No cross-test coupling: each test gets its own namespace or unique
///   resource name and tears it down independently.
use std::process::{Command, Stdio};

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
// Low-level helpers
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
