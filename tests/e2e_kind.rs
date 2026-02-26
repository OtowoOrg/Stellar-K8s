use std::error::Error;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

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
const OPERATOR_NAME: &str = "stellar-operator";
const NODE_NAME: &str = "test-soroban";
const E2E_NODE_NAME: &str = "e2e-soroban";
const HORIZON_NODE_NAME: &str = "test-horizon";

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

    let operator_yaml = operator_manifest(&image);
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

    let operator_yaml = operator_manifest(&image);
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

fn operator_manifest(image: &str) -> String {
    format!(
        r#"---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {OPERATOR_NAME}
  namespace: {OPERATOR_NAMESPACE}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {OPERATOR_NAME}
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
kind: ClusterRoleBinding
metadata:
  name: {OPERATOR_NAME}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
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
  name: {node_name}
  namespace: {namespace}
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
  namespace: {namespace}
type: Opaque
stringData:
  DATABASE_URL: "postgres://horizon:password@postgres:5432/horizon?sslmode=disable"
"#,
        node_name = HORIZON_NODE_NAME,
        namespace = HORIZON_TEST_NAMESPACE,
        version = version,
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

    let operator_yaml = operator_manifest(&image);
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
                &format!("{}-config", HORIZON_NODE_NAME),
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
                &format!("{}-data", HORIZON_NODE_NAME),
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
        return Err(format!(
            "unexpected Horizon node image after create: {}",
            current_image
        )
        .into());
    }

    // ── Step 3: Wait for pod to become Ready ─────────────────────────────────
    wait_for("Pod ready", Duration::from_secs(180), || {
        let result = run_cmd(
            "kubectl",
            &[
                "get",
                "pods",
                "-l",
                &format!("app.kubernetes.io/instance={}", HORIZON_NODE_NAME),
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
            &format!("app.kubernetes.io/instance={}", HORIZON_NODE_NAME),
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
            &format!("pod/{}", pod_name),
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
                &format!("{}-data", HORIZON_NODE_NAME),
                "-n",
                HORIZON_TEST_NAMESPACE,
            ],
        );
        let config_map = run_cmd(
            "kubectl",
            &[
                "get",
                "configmap",
                &format!("{}-config", HORIZON_NODE_NAME),
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
                &format!("app.kubernetes.io/instance={}", HORIZON_NODE_NAME),
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
