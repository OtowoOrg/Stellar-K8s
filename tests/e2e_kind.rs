use std::collections::HashMap;
use std::error::Error;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};
use tracing::info;

/// Returns true if the given binary is accessible in PATH.
fn tool_available(binary: &str) -> bool {
    Command::new(binary)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

const OPERATOR_NAMESPACE: &str = "stellar-system";
const TEST_NAMESPACE: &str = "stellar-e2e";
const HORIZON_TEST_NAMESPACE: &str = "stellar-e2e-horizon";
const UPGRADE_TEST_NAMESPACE: &str = "stellar-e2e-upgrade";
const OPERATOR_NAME: &str = "stellar-operator";
const NODE_NAME: &str = "test-soroban";
const E2E_NODE_NAME: &str = "e2e-soroban";
const HORIZON_NODE_NAME: &str = "test-horizon";
const UPGRADE_NODE_NAME: &str = "upgrade-soroban";

// ---------------------------------------------------------------------------
// Issue #156: E2E reconciliation test
//
// Tests actual StellarNode reconciliation on a real Kind cluster.
// Run with: cargo test --test e2e_kind -- --ignored
// ---------------------------------------------------------------------------

/// End-to-end test that exercises the full StellarNode reconciliation lifecycle:
///
/// 1. Start (or reuse) a Kind cluster.
/// 2. Install CRDs from `config/crd/`.
/// 3. Apply a sample StellarNode manifest.
/// 4. Wait for the operator to create a Deployment and Service.
/// 5. Assert that `status.phase` transitions to `Running`.
/// 6. Delete the resource and verify all child resources are cleaned up.
#[test]
#[ignore]
fn e2e_stellarnode_reconciliation() -> Result<(), Box<dyn std::error::Error>> {
    // ── Prerequisite check ─────────────────────────────────────────────────────
    // Skip gracefully when the required cluster tools are not installed.
    for tool in &["kind", "kubectl", "docker"] {
        if !tool_available(tool) {
            eprintln!("Skipping e2e test: `{tool}` not found in PATH.");
            return Ok(());
        }
    }

    let cluster_name = std::env::var("KIND_CLUSTER_NAME").unwrap_or_else(|_| "stellar-e2e".into());
    ensure_kind_cluster(&cluster_name)?;

    // ── Install the CRD ──────────────────────────────────────────────────────
    run_cmd(
        "kubectl",
        &["apply", "-f", "config/crd/stellarnode-crd.yaml"],
    )?;

    // ── Deploy the operator ──────────────────────────────────────────────────
    let image =
        std::env::var("E2E_OPERATOR_IMAGE").unwrap_or_else(|_| "stellar-operator:e2e".into());
    let build_image = env_true("E2E_BUILD_IMAGE", true);
    let load_image = env_true("E2E_LOAD_IMAGE", true);

    if build_image {
        run_cmd("docker", &["build", "-t", &image, "."])?;
    }
    if load_image {
        run_cmd(
            "kind",
            &["load", "docker-image", &image, "--name", &cluster_name],
        )?;
    }

    let operator_yaml = operator_manifest(&image, None);
    let _cleanup = E2eCleanup::new(operator_yaml.clone(), E2E_NODE_NAME);

    // Create operator namespace
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            OPERATOR_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    kubectl_apply(&operator_yaml)?;
    run_cmd(
        "kubectl",
        &[
            "rollout",
            "status",
            "deployment/stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--timeout=180s",
        ],
    )?;

    // ── Create test namespace ─────────────────────────────────────────────────
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            TEST_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    // ── Apply the StellarNode manifest ────────────────────────────────────────
    kubectl_apply(&e2e_soroban_manifest("v21.0.0"))?;

    // ── Step 1: StellarNode resource created ──────────────────────────────────
    wait_for("StellarNode exists", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "stellarnode", E2E_NODE_NAME, "-n", TEST_NAMESPACE],
        )
        .is_ok())
    })?;

    // ── Step 2: Deployment created by operator ────────────────────────────────
    wait_for("Deployment created", Duration::from_secs(90), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "deployment", E2E_NODE_NAME, "-n", TEST_NAMESPACE],
        )
        .is_ok())
    })?;

    // ── Step 3: Service created by operator ───────────────────────────────────
    wait_for("Service created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "service", E2E_NODE_NAME, "-n", TEST_NAMESPACE],
        )
        .is_ok())
    })?;

    // ── Step 4: status.phase transitions to Running ───────────────────────────
    wait_for(
        "StellarNode phase == Running",
        Duration::from_secs(120),
        || {
            let phase = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    E2E_NODE_NAME,
                    "-n",
                    TEST_NAMESPACE,
                    "-o",
                    "jsonpath={.status.phase}",
                ],
            )
            .unwrap_or_default();
            Ok(phase == "Running")
        },
    )?;

    // ── Step 5: Delete and verify cleanup ─────────────────────────────────────
    run_cmd(
        "kubectl",
        &[
            "delete",
            "stellarnode",
            E2E_NODE_NAME,
            "-n",
            TEST_NAMESPACE,
            "--timeout=180s",
            "--wait=true",
        ],
    )?;

    wait_for(
        "Child resources cleaned up",
        Duration::from_secs(90),
        || {
            let deployment = run_cmd(
                "kubectl",
                &["get", "deployment", E2E_NODE_NAME, "-n", TEST_NAMESPACE],
            );
            let service = run_cmd(
                "kubectl",
                &["get", "service", E2E_NODE_NAME, "-n", TEST_NAMESPACE],
            );
            Ok(deployment.is_err() && service.is_err())
        },
    )?;

    Ok(())
}

/// Manifest for the e2e reconciliation test node.
fn e2e_soroban_manifest(version: &str) -> String {
    format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {E2E_NODE_NAME}
  namespace: {TEST_NAMESPACE}
spec:
  nodeType: SorobanRpc
  network: Testnet
  version: "{version}"
  replicas: 1
  sorobanConfig:
    stellarCoreUrl: "http://stellar-core.default:11626"
  resources:
    requests:
      cpu: "100m"
      memory: "128Mi"
    limits:
      cpu: "250m"
      memory: "256Mi"
  storage:
    storageClass: "standard"
    size: "1Gi"
    retentionPolicy: Delete
"#,
    )
}

/// RAII cleanup guard for the e2e reconciliation test.
struct E2eCleanup {
    operator_manifest: String,
    node_name: &'static str,
}

impl E2eCleanup {
    fn new(operator_manifest: String, node_name: &'static str) -> Self {
        Self {
            operator_manifest,
            node_name,
        }
    }
}

impl Drop for E2eCleanup {
    fn drop(&mut self) {
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "stellarnode",
                self.node_name,
                "-n",
                TEST_NAMESPACE,
                "--ignore-not-found=true",
                "--timeout=60s",
                "--wait=true",
            ],
        );
        let _ =
            run_cmd_with_stdin_quiet("kubectl", &["delete", "-f", "-"], &self.operator_manifest);
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                TEST_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                OPERATOR_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
    }
}

#[test]
fn e2e_kind_install_crud_upgrade_delete() -> Result<(), Box<dyn Error>> {
    if std::env::var("E2E_KIND").is_err() {
        eprintln!("E2E_KIND is not set; skipping KinD E2E test.");
        return Ok(());
    }

    let cluster_name = std::env::var("KIND_CLUSTER_NAME").unwrap_or_else(|_| "stellar-e2e".into());
    ensure_kind_cluster(&cluster_name)?;

    let image =
        std::env::var("E2E_OPERATOR_IMAGE").unwrap_or_else(|_| "stellar-operator:e2e".into());
    let build_image = env_true("E2E_BUILD_IMAGE", true);
    let load_image = env_true("E2E_LOAD_IMAGE", true);

    if build_image {
        run_cmd("docker", &["build", "-t", &image, "."])?;
    }
    if load_image {
        run_cmd(
            "kind",
            &["load", "docker-image", &image, "--name", &cluster_name],
        )?;
    }

    let operator_yaml = operator_manifest(&image, None);
    let _cleanup = Cleanup::new(operator_yaml.clone());

    run_cmd(
        "kubectl",
        &["apply", "-f", "config/crd/stellarnode-crd.yaml"],
    )?;
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            OPERATOR_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    kubectl_apply(&operator_yaml)?;
    run_cmd(
        "kubectl",
        &[
            "rollout",
            "status",
            "deployment/stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--timeout=180s",
        ],
    )?;

    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            TEST_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    kubectl_apply(&soroban_node_manifest("v21.0.0", 1, false))?;
    wait_for("StellarNode exists", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "stellarnode", NODE_NAME, "-n", TEST_NAMESPACE],
        )
        .is_ok())
    })?;

    wait_for("Deployment created", Duration::from_secs(90), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "deployment", NODE_NAME, "-n", TEST_NAMESPACE],
        )
        .is_ok())
    })?;

    wait_for("Service created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "service", NODE_NAME, "-n", TEST_NAMESPACE],
        )
        .is_ok())
    })?;

    wait_for("ConfigMap created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "configmap",
                &format!("{NODE_NAME}-config"),
                "-n",
                TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    wait_for("PVC created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "pvc",
                &format!("{NODE_NAME}-data"),
                "-n",
                TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    let current_image = run_cmd(
        "kubectl",
        &[
            "get",
            "deployment",
            NODE_NAME,
            "-n",
            TEST_NAMESPACE,
            "-o",
            "jsonpath={.spec.template.spec.containers[0].image}",
        ],
    )?;
    if current_image != "stellar/soroban-rpc:v21.0.0" {
        return Err(format!("unexpected node image after create: {current_image}").into());
    }

    run_cmd(
        "kubectl",
        &[
            "patch",
            "stellarnode",
            NODE_NAME,
            "-n",
            TEST_NAMESPACE,
            "--type",
            "merge",
            "-p",
            "{\"spec\":{\"version\":\"v22.0.0\",\"replicas\":2}}",
        ],
    )?;

    wait_for("Deployment updated", Duration::from_secs(90), || {
        let image = run_cmd(
            "kubectl",
            &[
                "get",
                "deployment",
                NODE_NAME,
                "-n",
                TEST_NAMESPACE,
                "-o",
                "jsonpath={.spec.template.spec.containers[0].image}",
            ],
        )?;
        Ok(image == "stellar/soroban-rpc:v22.0.0")
    })?;

    wait_for("Deployment scaled", Duration::from_secs(60), || {
        let replicas = run_cmd(
            "kubectl",
            &[
                "get",
                "deployment",
                NODE_NAME,
                "-n",
                TEST_NAMESPACE,
                "-o",
                "jsonpath={.spec.replicas}",
            ],
        )?;
        Ok(replicas == "2")
    })?;

    run_cmd(
        "kubectl",
        &[
            "delete",
            "stellarnode",
            NODE_NAME,
            "-n",
            TEST_NAMESPACE,
            "--timeout=180s",
            "--wait=true",
        ],
    )?;

    wait_for("Workload cleanup", Duration::from_secs(90), || {
        let deployment = run_cmd(
            "kubectl",
            &["get", "deployment", NODE_NAME, "-n", TEST_NAMESPACE],
        );
        let service = run_cmd(
            "kubectl",
            &["get", "service", NODE_NAME, "-n", TEST_NAMESPACE],
        );
        let pvc = run_cmd(
            "kubectl",
            &[
                "get",
                "pvc",
                &format!("{NODE_NAME}-data"),
                "-n",
                TEST_NAMESPACE,
            ],
        );
        let config_map = run_cmd(
            "kubectl",
            &[
                "get",
                "configmap",
                &format!("{NODE_NAME}-config"),
                "-n",
                TEST_NAMESPACE,
            ],
        );
        Ok(deployment.is_err() && service.is_err() && pvc.is_err() && config_map.is_err())
    })?;

    Ok(())
}

fn ensure_kind_cluster(name: &str) -> Result<(), Box<dyn Error>> {
    let clusters = run_cmd("kind", &["get", "clusters"])?;
    if clusters.lines().any(|line| line.trim() == name) {
        return Ok(());
    }
    run_cmd("kind", &["create", "cluster", "--name", name])?;
    Ok(())
}

fn kubectl_apply(manifest: &str) -> Result<(), Box<dyn Error>> {
    run_cmd_with_stdin("kubectl", &["apply", "-f", "-"], manifest)?;
    Ok(())
}

fn run_cmd(program: &str, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let output = cmd.output()?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "command failed: {program} {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_cmd_with_stdin(program: &str, args: &[&str], input: &str) -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())?;
        stdin.flush()?;
        drop(stdin);
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "command failed: {program} {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )
        .into());
    }
    Ok(())
}

fn wait_for<F>(label: &str, timeout: Duration, mut condition: F) -> Result<(), Box<dyn Error>>
where
    F: FnMut() -> Result<bool, Box<dyn Error>>,
{
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
            )
            .into());
        }
        sleep(Duration::from_secs(3));
    }
}

fn env_true(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

fn operator_manifest(image: &str, watch_namespace: Option<&str>) -> String {
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
        "".to_string()
    };

    let watch_arg = if let Some(ns) = watch_namespace {
        format!("\n            - --watch-namespace={ns}")
    } else {
        "".to_string()
    };

    format!(
        r#"---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {OPERATOR_NAME}
  namespace: {OPERATOR_NAMESPACE}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: {rbac_kind}
metadata:
  name: {OPERATOR_NAME}{rbac_namespace}
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
  name: {OPERATOR_NAME}{rbac_namespace}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: {rbac_kind}
  name: {OPERATOR_NAME}
subjects:
  - kind: ServiceAccount
    name: {OPERATOR_NAME}
    namespace: {OPERATOR_NAMESPACE}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {OPERATOR_NAME}
  namespace: {OPERATOR_NAMESPACE}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: {OPERATOR_NAME}
  template:
    metadata:
      labels:
        app: {OPERATOR_NAME}
    spec:
      serviceAccountName: {OPERATOR_NAME}
      containers:
        - name: operator
          image: {image}
          imagePullPolicy: IfNotPresent
          args:
            - run
            - --namespace={OPERATOR_NAMESPACE} {watch_arg}
          env:
            - name: OPERATOR_NAMESPACE
              value: {OPERATOR_NAMESPACE}
"#
    )
}

struct Cleanup {
    operator_manifest: String,
}

impl Cleanup {
    fn new(operator_manifest: String) -> Self {
        Self { operator_manifest }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ =
            run_cmd_with_stdin_quiet("kubectl", &["delete", "-f", "-"], &self.operator_manifest);
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                TEST_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                OPERATOR_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
    }
}

fn run_cmd_quiet(program: &str, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let _ = cmd.output();
    Ok(())
}

fn run_cmd_with_stdin_quiet(
    program: &str,
    args: &[&str],
    input: &str,
) -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONFIG", kubeconfig);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(input.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    }
    let _ = child.wait_with_output();
    Ok(())
}

/// Parses `kubectl` JSONPath output of the form `name=count\n...` into a map.
///
/// Lines that are empty or cannot be parsed are silently skipped.
pub fn parse_restart_counts(output: &str) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, count_str)) = line.split_once('=') {
            if let Ok(count) = count_str.trim().parse::<u32>() {
                map.insert(name.trim().to_string(), count);
            }
        }
    }
    map
}

/// Queries pod restart counts for all pods belonging to `deployment` in `namespace`.
///
/// Uses the label selector `app.kubernetes.io/instance={deployment}` and a JSONPath
/// template that emits `{pod-name}={restartCount}` lines.
fn record_pod_restart_counts(
    namespace: &str,
    deployment: &str,
) -> Result<HashMap<String, u32>, Box<dyn Error>> {
    let label = format!("app.kubernetes.io/instance={deployment}");
    let jsonpath =
        "{range .items[*]}{.metadata.name}={.status.containerStatuses[0].restartCount}\\n{end}";
    let output = run_cmd(
        "kubectl",
        &[
            "get",
            "pods",
            "-l",
            &label,
            "-n",
            namespace,
            "-o",
            &format!("jsonpath={jsonpath}"),
        ],
    )?;
    Ok(parse_restart_counts(&output))
}

/// Extracts the lease holder identity from raw `kubectl` JSONPath output.
///
/// Trims leading/trailing whitespace. Returns an empty string for empty input.
pub fn parse_lease_holder(output: &str) -> String {
    output.trim().to_string()
}

/// Queries the `stellar-operator` lease holder in `namespace`.
///
/// Returns the trimmed holder identity string, or an empty string if the lease
/// does not exist (i.e. `kubectl` returns an error).
fn get_lease_holder(namespace: &str) -> Result<String, Box<dyn Error>> {
    match run_cmd(
        "kubectl",
        &[
            "get",
            "lease",
            "stellar-operator",
            "-n",
            namespace,
            "-o",
            "jsonpath={.spec.holderIdentity}",
        ],
    ) {
        Ok(output) => Ok(parse_lease_holder(&output)),
        Err(_) => Ok("".to_string()),
    }
}

/// Parses raw `kubectl` JSONPath output into a list of pod names.
///
/// Splits on newlines and filters out empty strings. This is the pure
/// parsing counterpart to `get_pod_names_for_deployment`, needed for
/// property testing (task 2.6).
pub fn parse_pod_names(output: &str) -> Vec<String> {
    output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Queries pod names for all pods belonging to `deployment` in `namespace`.
///
/// Uses the label selector `app.kubernetes.io/instance={deployment}` and a
/// JSONPath template that emits one pod name per line.
fn get_pod_names_for_deployment(
    namespace: &str,
    deployment: &str,
) -> Result<Vec<String>, Box<dyn Error>> {
    let label = format!("app.kubernetes.io/instance={deployment}");
    let jsonpath = "{range .items[*]}{.metadata.name}\\n{end}";
    let output = run_cmd(
        "kubectl",
        &[
            "get",
            "pods",
            "-l",
            &label,
            "-n",
            namespace,
            "-o",
            &format!("jsonpath={jsonpath}"),
        ],
    )?;
    Ok(parse_pod_names(&output))
}

fn soroban_node_manifest(version: &str, replicas: i32, suspended: bool) -> String {
    format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {NODE_NAME}
  namespace: {TEST_NAMESPACE}
spec:
  nodeType: SorobanRpc
  network: Testnet
  version: "{version}"
  replicas: {replicas}
  suspended: {suspended}
  sorobanConfig:
    stellarCoreUrl: "http://stellar-core.default:11626"
  resources:
    requests:
      cpu: "100m"
      memory: "128Mi"
    limits:
      cpu: "250m"
      memory: "256Mi"
  storage:
    storageClass: "standard"
    size: "1Gi"
    retentionPolicy: Delete
"#
    )
}

/// Manifest for the operator upgrade simulation test node.
///
/// Mirrors `soroban_node_manifest` but uses `UPGRADE_TEST_NAMESPACE` and
/// `UPGRADE_NODE_NAME` so the upgrade test has its own isolated resource.
fn upgrade_soroban_manifest(version: &str) -> String {
    format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {UPGRADE_NODE_NAME}
  namespace: {UPGRADE_TEST_NAMESPACE}
spec:
  nodeType: SorobanRpc
  network: Testnet
  version: "{version}"
  replicas: 1
  suspended: false
  sorobanConfig:
    stellarCoreUrl: "http://stellar-core.default:11626"
  resources:
    requests:
      cpu: "100m"
      memory: "128Mi"
    limits:
      cpu: "250m"
      memory: "256Mi"
  storage:
    storageClass: "standard"
    size: "1Gi"
    retentionPolicy: Delete
"#,
    )
}

// ---------------------------------------------------------------------------
// Horizon node lifecycle E2E test
//
// Validates the most common production use case: deploying a Horizon API node
// with health checks. Run with:
//   cargo test --test e2e_kind -- --ignored
// ---------------------------------------------------------------------------

fn horizon_node_manifest(version: &str) -> String {
    format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {HORIZON_NODE_NAME}
  namespace: {HORIZON_TEST_NAMESPACE}
spec:
  nodeType: Horizon
  network: Testnet
  version: "{version}"
  replicas: 1
  horizonConfig:
    databaseSecretRef: "horizon-db-credentials"
    enableIngest: true
    stellarCoreUrl: "http://stellar-core.default:11626"
    ingestWorkers: 1
  resources:
    requests:
      cpu: "100m"
      memory: "128Mi"
    limits:
      cpu: "250m"
      memory: "256Mi"
  storage:
    storageClass: "standard"
    size: "1Gi"
    retentionPolicy: Delete
---
apiVersion: v1
kind: Secret
metadata:
  name: horizon-db-credentials
  namespace: {HORIZON_TEST_NAMESPACE}
type: Opaque
stringData:
  DATABASE_URL: "postgres://horizon:password@postgres:5432/horizon?sslmode=disable"
"#,
    )
}

/// RAII cleanup guard for the Horizon lifecycle test.
struct HorizonCleanup {
    operator_manifest: String,
}

impl HorizonCleanup {
    fn new(operator_manifest: String) -> Self {
        Self { operator_manifest }
    }
}

impl Drop for HorizonCleanup {
    fn drop(&mut self) {
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "stellarnode",
                HORIZON_NODE_NAME,
                "-n",
                HORIZON_TEST_NAMESPACE,
                "--ignore-not-found=true",
                "--timeout=60s",
                "--wait=true",
            ],
        );
        let _ =
            run_cmd_with_stdin_quiet("kubectl", &["delete", "-f", "-"], &self.operator_manifest);
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                HORIZON_TEST_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                OPERATOR_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
    }
}

/// RAII cleanup guard for the operator upgrade simulation test.
struct UpgradeCleanup {
    operator_manifest: String,
}

impl UpgradeCleanup {
    fn new(operator_manifest: String) -> Self {
        Self { operator_manifest }
    }
}

impl Drop for UpgradeCleanup {
    fn drop(&mut self) {
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "stellarnode",
                UPGRADE_NODE_NAME,
                "-n",
                UPGRADE_TEST_NAMESPACE,
                "--ignore-not-found=true",
                "--timeout=60s",
                "--wait=true",
            ],
        );
        let _ =
            run_cmd_with_stdin_quiet("kubectl", &["delete", "-f", "-"], &self.operator_manifest);
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                UPGRADE_TEST_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                OPERATOR_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
    }
}

/// Full Horizon node lifecycle E2E test.
///
/// 1. Apply the Horizon manifest (mirrors examples/horizon-with-health-check.yaml).
/// 2. Wait for the operator to reconcile and the pod to become Ready.
/// 3. Port-forward to the Horizon pod and curl `http://localhost:8000/` — must return HTTP 200.
/// 4. Verify the StellarNode status shows `phase: Running`.
/// 5. Delete the resource and verify pods + services are cleaned up within 60 seconds.
#[test]
#[ignore]
fn e2e_kind_horizon_lifecycle() -> Result<(), Box<dyn Error>> {
    if std::env::var("E2E_KIND").is_err() {
        eprintln!("E2E_KIND is not set; skipping KinD E2E Horizon lifecycle test.");
        return Ok(());
    }

    let cluster_name = std::env::var("KIND_CLUSTER_NAME").unwrap_or_else(|_| "stellar-e2e".into());
    ensure_kind_cluster(&cluster_name)?;

    let image =
        std::env::var("E2E_OPERATOR_IMAGE").unwrap_or_else(|_| "stellar-operator:e2e".into());
    let build_image = env_true("E2E_BUILD_IMAGE", true);
    let load_image = env_true("E2E_LOAD_IMAGE", true);

    if build_image {
        run_cmd("docker", &["build", "-t", &image, "."])?;
    }
    if load_image {
        run_cmd(
            "kind",
            &["load", "docker-image", &image, "--name", &cluster_name],
        )?;
    }

    let operator_yaml = operator_manifest(&image, None);
    let _cleanup = HorizonCleanup::new(operator_yaml.clone());

    // ── Install CRD ──────────────────────────────────────────────────────────
    run_cmd(
        "kubectl",
        &["apply", "-f", "config/crd/stellarnode-crd.yaml"],
    )?;

    // ── Create operator namespace ────────────────────────────────────────────
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            OPERATOR_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    // ── Deploy operator ──────────────────────────────────────────────────────
    kubectl_apply(&operator_yaml)?;
    run_cmd(
        "kubectl",
        &[
            "rollout",
            "status",
            "deployment/stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--timeout=180s",
        ],
    )?;

    // ── Create test namespace ────────────────────────────────────────────────
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            HORIZON_TEST_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    // ── Step 1: Apply the Horizon manifest ───────────────────────────────────
    kubectl_apply(&horizon_node_manifest("v21.0.0"))?;

    wait_for("StellarNode exists", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "stellarnode",
                HORIZON_NODE_NAME,
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    // ── Step 2: Wait for operator to reconcile — Deployment, Service, ConfigMap, PVC
    wait_for("Deployment created", Duration::from_secs(90), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "deployment",
                HORIZON_NODE_NAME,
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    wait_for("Service created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "service",
                HORIZON_NODE_NAME,
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    wait_for("ConfigMap created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "configmap",
                &format!("{HORIZON_NODE_NAME}-config"),
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    wait_for("PVC created", Duration::from_secs(60), || {
        Ok(run_cmd(
            "kubectl",
            &[
                "get",
                "pvc",
                &format!("{HORIZON_NODE_NAME}-data"),
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        )
        .is_ok())
    })?;

    // Verify the container image is correct
    let current_image = run_cmd(
        "kubectl",
        &[
            "get",
            "deployment",
            HORIZON_NODE_NAME,
            "-n",
            HORIZON_TEST_NAMESPACE,
            "-o",
            "jsonpath={.spec.template.spec.containers[0].image}",
        ],
    )?;
    if current_image != "stellar/horizon:v21.0.0" {
        return Err(format!("unexpected Horizon node image after create: {current_image}").into());
    }

    // ── Step 3: Wait for pod to become Ready ─────────────────────────────────
    wait_for("Pod ready", Duration::from_secs(180), || {
        let result = run_cmd(
            "kubectl",
            &[
                "get",
                "pods",
                "-l",
                &format!("app.kubernetes.io/instance={HORIZON_NODE_NAME}"),
                "-n",
                HORIZON_TEST_NAMESPACE,
                "-o",
                "jsonpath={.items[0].status.conditions[?(@.type=='Ready')].status}",
            ],
        );
        match result {
            Ok(status) => Ok(status == "True"),
            Err(_) => Ok(false),
        }
    })?;

    // ── Step 4: Port-forward and curl the Horizon endpoint ───────────────────
    // Must return HTTP 200.
    let pod_name = run_cmd(
        "kubectl",
        &[
            "get",
            "pods",
            "-l",
            &format!("app.kubernetes.io/instance={HORIZON_NODE_NAME}"),
            "-n",
            HORIZON_TEST_NAMESPACE,
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ],
    )?;

    // Start port-forward as a background process
    let mut port_forward = Command::new("kubectl")
        .args([
            "port-forward",
            &format!("pod/{pod_name}"),
            "18000:8000",
            "-n",
            HORIZON_TEST_NAMESPACE,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // Allow port-forward to establish
    sleep(Duration::from_secs(3));

    let curl_result = wait_for("Horizon HTTP 200", Duration::from_secs(30), || {
        let result = run_cmd(
            "curl",
            &[
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "http://localhost:18000/",
            ],
        );
        match result {
            Ok(code) => Ok(code.trim() == "200"),
            Err(_) => Ok(false),
        }
    });

    // Always kill port-forward regardless of result
    let _ = port_forward.kill();
    let _ = port_forward.wait();

    curl_result?;

    // ── Step 5: Verify StellarNode status phase is Running ───────────────────
    wait_for(
        "StellarNode phase Running",
        Duration::from_secs(120),
        || {
            let phase = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    HORIZON_NODE_NAME,
                    "-n",
                    HORIZON_TEST_NAMESPACE,
                    "-o",
                    "jsonpath={.status.phase}",
                ],
            );
            match phase {
                Ok(p) => {
                    let p = p.trim().to_string();
                    Ok(p == "Running" || p == "Ready")
                }
                Err(_) => Ok(false),
            }
        },
    )?;

    // ── Step 6: Delete and verify cleanup within 60 seconds ──────────────────
    run_cmd(
        "kubectl",
        &[
            "delete",
            "stellarnode",
            HORIZON_NODE_NAME,
            "-n",
            HORIZON_TEST_NAMESPACE,
            "--timeout=180s",
            "--wait=true",
        ],
    )?;

    wait_for("Workload cleanup", Duration::from_secs(60), || {
        let deployment = run_cmd(
            "kubectl",
            &[
                "get",
                "deployment",
                HORIZON_NODE_NAME,
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        );
        let service = run_cmd(
            "kubectl",
            &[
                "get",
                "service",
                HORIZON_NODE_NAME,
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        );
        let pvc = run_cmd(
            "kubectl",
            &[
                "get",
                "pvc",
                &format!("{HORIZON_NODE_NAME}-data"),
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        );
        let config_map = run_cmd(
            "kubectl",
            &[
                "get",
                "configmap",
                &format!("{HORIZON_NODE_NAME}-config"),
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        );
        let pods = run_cmd(
            "kubectl",
            &[
                "get",
                "pods",
                "-l",
                &format!("app.kubernetes.io/instance={HORIZON_NODE_NAME}"),
                "-n",
                HORIZON_TEST_NAMESPACE,
                "-o",
                "jsonpath={.items}",
            ],
        );
        let pods_gone = match pods {
            Ok(output) => output.trim() == "[]" || output.trim().is_empty(),
            Err(_) => true,
        };
        Ok(deployment.is_err()
            && service.is_err()
            && pvc.is_err()
            && config_map.is_err()
            && pods_gone)
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Issue #322: Namespace-scoped mode test
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn e2e_namespace_scoped_reconciliation() -> Result<(), Box<dyn Error>> {
    const SCOPED_NAMESPACE: &str = "stellar-scoped";
    const IGNORED_NAMESPACE: &str = "stellar-ignored";
    const SCOPED_NODE: &str = "scoped-node";
    const IGNORED_NODE: &str = "ignored-node";

    if std::env::var("E2E_KIND").is_err() {
        eprintln!("E2E_KIND is not set; skipping namespace-scoped E2E test.");
        return Ok(());
    }

    let cluster_name = std::env::var("KIND_CLUSTER_NAME").unwrap_or_else(|_| "stellar-e2e".into());
    ensure_kind_cluster(&cluster_name)?;

    let image =
        std::env::var("E2E_OPERATOR_IMAGE").unwrap_or_else(|_| "stellar-operator:e2e".into());

    // Deploy operator watching ONLY SCOPED_NAMESPACE
    let operator_yaml = operator_manifest(&image, Some(SCOPED_NAMESPACE));

    // Manual cleanup for this test
    let _ = run_cmd_quiet(
        "kubectl",
        &[
            "delete",
            "namespace",
            SCOPED_NAMESPACE,
            IGNORED_NAMESPACE,
            OPERATOR_NAMESPACE,
            "--ignore-not-found=true",
        ],
    );

    run_cmd(
        "kubectl",
        &["apply", "-f", "config/crd/stellarnode-crd.yaml"],
    )?;
    run_cmd("kubectl", &["create", "namespace", OPERATOR_NAMESPACE])?;
    run_cmd("kubectl", &["create", "namespace", SCOPED_NAMESPACE])?;
    run_cmd("kubectl", &["create", "namespace", IGNORED_NAMESPACE])?;

    kubectl_apply(&operator_yaml)?;
    run_cmd(
        "kubectl",
        &[
            "rollout",
            "status",
            "deployment/stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--timeout=180s",
        ],
    )?;

    // 1. Create node in SCOPED namespace -> Should work
    let scoped_manifest = format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {SCOPED_NODE}
  namespace: {SCOPED_NAMESPACE}
spec:
  nodeType: SorobanRpc
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  sorobanConfig:
    stellarCoreUrl: "http://stellar-core.default:11626"
"#
    );
    kubectl_apply(&scoped_manifest)?;

    wait_for("Scoped node deployment", Duration::from_secs(90), || {
        Ok(run_cmd(
            "kubectl",
            &["get", "deployment", SCOPED_NODE, "-n", SCOPED_NAMESPACE],
        )
        .is_ok())
    })?;
    info!("✓ Scoped node reconciliation verified");

    // 2. Create node in IGNORED namespace -> Should NOT work
    let ignored_manifest = format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {IGNORED_NODE}
  namespace: {IGNORED_NAMESPACE}
spec:
  nodeType: SorobanRpc
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  sorobanConfig:
    stellarCoreUrl: "http://stellar-core.default:11626"
"#
    );
    kubectl_apply(&ignored_manifest)?;

    // Wait a bit and verify NO deployment exists in the ignored namespace
    sleep(Duration::from_secs(20));
    let deployment = run_cmd(
        "kubectl",
        &["get", "deployment", IGNORED_NODE, "-n", IGNORED_NAMESPACE],
    );
    if deployment.is_ok() {
        return Err("Operator reconciled a node in an ignored namespace!".into());
    }
    info!("✓ Ignored node isolation verified");

    // Cleanup
    let _ = run_cmd_with_stdin_quiet("kubectl", &["delete", "-f", "-"], &operator_yaml);
    let _ = run_cmd_quiet(
        "kubectl",
        &[
            "delete",
            "namespace",
            SCOPED_NAMESPACE,
            IGNORED_NAMESPACE,
            OPERATOR_NAMESPACE,
            "--ignore-not-found=true",
        ],
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Operator upgrade simulation E2E test
//
// Verifies that upgrading the stellar-operator from an old version to a new
// version does not disrupt managed StellarNode resources.
// Run with: E2E_KIND=1 cargo test --test e2e_kind -- --ignored
// ---------------------------------------------------------------------------

/// End-to-end test that simulates an operator upgrade from an old image to a
/// new image and verifies that managed StellarNode resources are unaffected.
///
/// Phases (implemented incrementally across tasks):
///   1. Scaffolding — cluster setup, CRD install, namespace creation, cleanup guard
///   2. Old operator steady state
///   3. Upgrade execution
///   4. Leader election handover verification
///   5. Managed pod stability verification
///   6. Status field correctness after upgrade
#[test]
#[ignore]
fn e2e_operator_upgrade_simulation() -> Result<(), Box<dyn Error>> {
    // ── Phase 1: Scaffolding ─────────────────────────────────────────────────

    // Req 1.2 — skip when E2E_KIND is not set
    if std::env::var("E2E_KIND").is_err() {
        eprintln!("E2E_KIND is not set; skipping operator upgrade simulation test.");
        return Ok(());
    }

    // Req 1.3 — cluster name
    let cluster_name = std::env::var("KIND_CLUSTER_NAME").unwrap_or_else(|_| "stellar-e2e".into());

    // Req 1.4 — operator images
    let old_image =
        std::env::var("E2E_OLD_OPERATOR_IMAGE").unwrap_or_else(|_| "stellar-operator:old".into());

    // Req 1.3 — create or reuse Kind cluster
    ensure_kind_cluster(&cluster_name)?;

    // Req 1.5 — install CRD
    run_cmd(
        "kubectl",
        &["apply", "-f", "config/crd/stellarnode-crd.yaml"],
    )?;

    // Create operator namespace (dry-run + apply pattern)
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            OPERATOR_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    // Create upgrade test namespace (dry-run + apply pattern)
    run_cmd(
        "kubectl",
        &[
            "create",
            "namespace",
            UPGRADE_TEST_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ],
    )
    .and_then(|output| kubectl_apply(&output))?;

    // Build old operator manifest and register RAII cleanup guard (Req 1.6)
    let old_operator_yaml = operator_manifest(&old_image, None);
    let _cleanup = UpgradeCleanup::new(old_operator_yaml.clone());

    // ── Phase 2: Old Operator Steady State ──────────────────────────────────

    // Req 2.1 — deploy old operator and wait for rollout
    kubectl_apply(&old_operator_yaml)?;
    run_cmd(
        "kubectl",
        &[
            "rollout",
            "status",
            "deployment/stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--timeout=180s",
        ],
    )?;

    // Apply the StellarNode manifest
    kubectl_apply(&upgrade_soroban_manifest("v21.0.0"))?;

    // Req 2.2 — wait for StellarNode to reach Running phase
    wait_for(
        "StellarNode Running (old operator)",
        Duration::from_secs(120),
        || {
            let phase = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    UPGRADE_NODE_NAME,
                    "-n",
                    UPGRADE_TEST_NAMESPACE,
                    "-o",
                    "jsonpath={.status.phase}",
                ],
            )
            .unwrap_or_default();
            Ok(phase == "Running")
        },
    )?;

    // Req 2.3 — record baseline restart counts
    let baseline_restarts = record_pod_restart_counts(UPGRADE_TEST_NAMESPACE, UPGRADE_NODE_NAME)?;

    // Req 2.4 — record old lease holder
    let _old_lease_holder = get_lease_holder(OPERATOR_NAMESPACE)?;

    // ── Phase 3: Upgrade Execution ───────────────────────────────────────────

    // Req 3.4 — read new image (moved from placeholder above)
    let new_image =
        std::env::var("E2E_NEW_OPERATOR_IMAGE").unwrap_or_else(|_| "stellar-operator:new".into());

    // Req 3.1 — apply new operator manifest
    let new_operator_yaml = operator_manifest(&new_image, None);
    kubectl_apply(&new_operator_yaml)?;

    // Req 3.2 — wait for new operator rollout
    run_cmd(
        "kubectl",
        &[
            "rollout",
            "status",
            "deployment/stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--timeout=180s",
        ],
    )
    .map_err(|_| -> Box<dyn Error> { "New operator rollout timed out after 180s".into() })?;

    // Req 3.3 — verify StellarNode still exists (not deleted/recreated)
    run_cmd(
        "kubectl",
        &[
            "get",
            "stellarnode",
            UPGRADE_NODE_NAME,
            "-n",
            UPGRADE_TEST_NAMESPACE,
        ],
    )?;

    // ── Phase 4: Leader Election Handover ────────────────────────────────────

    // Req 4.1 — get new operator pod names
    let new_pod_names = get_pod_names_for_deployment(OPERATOR_NAMESPACE, OPERATOR_NAME)?;

    // Req 4.2 / 4.3 — poll until lease holder is one of the new pods
    let mut last_holder = String::new();
    wait_for(
        "Lease transferred to new operator",
        Duration::from_secs(60),
        || {
            let holder = get_lease_holder(OPERATOR_NAMESPACE)?;
            last_holder = holder.clone();
            Ok(!holder.is_empty() && new_pod_names.contains(&holder))
        },
    )
    .map_err(|_| -> Box<dyn Error> {
        format!("Lease did not transfer within 60s; last holder: {last_holder}").into()
    })?;

    // ── Phase 5: Managed Pod Stability ───────────────────────────────────────

    // Req 5.1 — settling period after lease acquisition
    sleep(Duration::from_secs(30));

    // Req 5.2 — record post-upgrade restart counts
    let post_restarts = record_pod_restart_counts(UPGRADE_TEST_NAMESPACE, UPGRADE_NODE_NAME)?;

    // Req 5.3 — fail if any pod restarted relative to baseline
    for (pod, &before) in &baseline_restarts {
        let after = post_restarts.get(pod).copied().unwrap_or(0);
        if after > before {
            return Err(format!(
                "Pod {pod} restarted {} times during upgrade (before={before}, after={after})",
                after - before
            )
            .into());
        }
    }

    // Req 5.4 — fail if pod count changed
    if post_restarts.len() != baseline_restarts.len() {
        return Err(format!(
            "Pod count changed during upgrade: before={}, after={}",
            baseline_restarts.len(),
            post_restarts.len()
        )
        .into());
    }

    // ── Phase 6: Status Field Correctness ────────────────────────────────────

    // Helper closure to fetch full status JSON for error context (Req 6.4)
    let full_status = || {
        run_cmd(
            "kubectl",
            &[
                "get",
                "stellarnode",
                UPGRADE_NODE_NAME,
                "-n",
                UPGRADE_TEST_NAMESPACE,
                "-o",
                "jsonpath={.status}",
            ],
        )
        .unwrap_or_else(|_| "<unavailable>".to_string())
    };

    // Req 6.1 — status.phase == "Running"
    let phase = run_cmd(
        "kubectl",
        &[
            "get",
            "stellarnode",
            UPGRADE_NODE_NAME,
            "-n",
            UPGRADE_TEST_NAMESPACE,
            "-o",
            "jsonpath={.status.phase}",
        ],
    )?;
    if phase != "Running" {
        return Err(format!(
            "Expected status.phase=Running after upgrade, got: {phase}\nFull status: {}",
            full_status()
        )
        .into());
    }

    // Req 6.2 — status.observedGeneration == metadata.generation
    let observed_gen = run_cmd(
        "kubectl",
        &[
            "get",
            "stellarnode",
            UPGRADE_NODE_NAME,
            "-n",
            UPGRADE_TEST_NAMESPACE,
            "-o",
            "jsonpath={.status.observedGeneration}",
        ],
    )?;
    let generation = run_cmd(
        "kubectl",
        &[
            "get",
            "stellarnode",
            UPGRADE_NODE_NAME,
            "-n",
            UPGRADE_TEST_NAMESPACE,
            "-o",
            "jsonpath={.metadata.generation}",
        ],
    )?;
    if observed_gen != generation {
        return Err(format!(
            "observedGeneration ({observed_gen}) != generation ({generation}) after upgrade\nFull status: {}",
            full_status()
        )
        .into());
    }

    // Req 6.3 — status.conditions contains type=Ready, status=True
    let ready_status = run_cmd(
        "kubectl",
        &[
            "get",
            "stellarnode",
            UPGRADE_NODE_NAME,
            "-n",
            UPGRADE_TEST_NAMESPACE,
            "-o",
            r#"jsonpath={.status.conditions[?(@.type=="Ready")].status}"#,
        ],
    )?;
    if ready_status != "True" {
        return Err(format!(
            "Expected Ready condition status=True after upgrade, got: {ready_status}\nFull status: {}",
            full_status()
        )
        .into());
    }

    Ok(())
}
