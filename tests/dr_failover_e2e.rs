use std::error::Error;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn tool_available(binary: &str) -> bool {
    Command::new(binary)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

const OPERATOR_NAMESPACE: &str = "dr-operator-system";
const PRIMARY_NAMESPACE: &str = "region-primary";
const STANDBY_NAMESPACE: &str = "region-standby";
const OPERATOR_NAME: &str = "stellar-operator";
const PRIMARY_NODE_NAME: &str = "e2e-dr-primary";
const STANDBY_NODE_NAME: &str = "e2e-dr-standby";

#[test]
#[ignore]
fn e2e_dr_failover() -> Result<(), Box<dyn std::error::Error>> {
    // ── Prerequisite check ─────────────────────────────────────────────────────
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
    let _cleanup = DrCleanup::new(operator_yaml.clone());

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

    // ── Create test namespaces ────────────────────────────────────────────────
    for ns in &[PRIMARY_NAMESPACE, STANDBY_NAMESPACE] {
        run_cmd(
            "kubectl",
            &["create", "namespace", ns, "--dry-run=client", "-o", "yaml"],
        )
        .and_then(|output| kubectl_apply(&output))?;
    }

    // ── Apply the StellarNode manifests ───────────────────────────────────────
    let primary_manifest = dr_node_manifest(
        PRIMARY_NODE_NAME,
        PRIMARY_NAMESPACE,
        "Primary",
        STANDBY_NAMESPACE,
    );
    let standby_manifest = dr_node_manifest(
        STANDBY_NODE_NAME,
        STANDBY_NAMESPACE,
        "Standby",
        PRIMARY_NAMESPACE,
    );

    kubectl_apply(&primary_manifest)?;
    kubectl_apply(&standby_manifest)?;

    // ── Wait for both Deployments to be Running ───────────────────────────────
    wait_for(
        "Primary Deployment created",
        Duration::from_secs(90),
        || {
            Ok(run_cmd(
                "kubectl",
                &[
                    "get",
                    "deployment",
                    PRIMARY_NODE_NAME,
                    "-n",
                    PRIMARY_NAMESPACE,
                ],
            )
            .is_ok())
        },
    )?;

    wait_for(
        "Standby Deployment created",
        Duration::from_secs(90),
        || {
            Ok(run_cmd(
                "kubectl",
                &[
                    "get",
                    "deployment",
                    STANDBY_NODE_NAME,
                    "-n",
                    STANDBY_NAMESPACE,
                ],
            )
            .is_ok())
        },
    )?;

    wait_for(
        "Primary StellarNode phase == Running",
        Duration::from_secs(180),
        || {
            let phase = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    PRIMARY_NODE_NAME,
                    "-n",
                    PRIMARY_NAMESPACE,
                    "-o",
                    "jsonpath={.status.conditions[?(@.type=='Ready')].status}",
                ],
            )
            .unwrap_or_default();
            Ok(phase == "True")
        },
    )?;

    wait_for(
        "Standby StellarNode phase == Running",
        Duration::from_secs(180),
        || {
            let phase = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    STANDBY_NODE_NAME,
                    "-n",
                    STANDBY_NAMESPACE,
                    "-o",
                    "jsonpath={.status.conditions[?(@.type=='Ready')].status}",
                ],
            )
            .unwrap_or_default();
            Ok(phase == "True")
        },
    )?;

    // Wait until Primary's Deployment has 1 ready replica
    wait_for(
        "Primary readyReplicas == 1",
        Duration::from_secs(90),
        || {
            let ready = run_cmd(
                "kubectl",
                &[
                    "get",
                    "deployment",
                    PRIMARY_NODE_NAME,
                    "-n",
                    PRIMARY_NAMESPACE,
                    "-o",
                    "jsonpath={.status.readyReplicas}",
                ],
            )
            .unwrap_or_default();
            Ok(ready == "1")
        },
    )?;

    // ── Simulate Primary Failure ──────────────────────────────────────────────
    println!("Simulating Primary Failure by scaling Deployment to 0...");
    run_cmd(
        "kubectl",
        &[
            "scale",
            "deployment",
            PRIMARY_NODE_NAME,
            "-n",
            PRIMARY_NAMESPACE,
            "--replicas=0",
        ],
    )?;

    // ── Verify Standby Failover ───────────────────────────────────────────────
    println!("Waiting for Standby to promote to Primary...");
    wait_for(
        "Standby failoverActive == true & currentRole == Primary",
        Duration::from_secs(180),
        || {
            let failover_active = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    STANDBY_NODE_NAME,
                    "-n",
                    STANDBY_NAMESPACE,
                    "-o",
                    "jsonpath={.status.drStatus.failoverActive}",
                ],
            )
            .unwrap_or_default();
            let role = run_cmd(
                "kubectl",
                &[
                    "get",
                    "stellarnode",
                    STANDBY_NODE_NAME,
                    "-n",
                    STANDBY_NAMESPACE,
                    "-o",
                    "jsonpath={.status.drStatus.currentRole}",
                ],
            )
            .unwrap_or_default();
            Ok(failover_active == "true" && role == "Primary")
        },
    )?;

    println!("Failover confirmed successfully!");

    Ok(())
}

fn dr_node_manifest(node_name: &str, namespace: &str, role: &str, peer_cluster_id: &str) -> String {
    format!(
        r#"apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: {node_name}
  namespace: {namespace}
spec:
  nodeType: SorobanRpc
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  sorobanConfig:
    stellarCoreUrl: "http://stellar-core.default:11626"
  resources:
    requests:
      cpu: "50m"
      memory: "128Mi"
    limits:
      cpu: "100m"
      memory: "256Mi"
  storage:
    storageClass: "standard"
    size: "1Gi"
    retentionPolicy: Delete
  drConfig:
    enabled: true
    role: {role}
    syncStrategy: PeerTracking
    peerClusterId: {peer_cluster_id}
"#,
        node_name = node_name,
        namespace = namespace,
        role = role,
        peer_cluster_id = peer_cluster_id
    )
}

struct DrCleanup {
    operator_manifest: String,
}

impl DrCleanup {
    fn new(operator_manifest: String) -> Self {
        Self { operator_manifest }
    }
}

impl Drop for DrCleanup {
    fn drop(&mut self) {
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "stellarnode",
                PRIMARY_NODE_NAME,
                "-n",
                PRIMARY_NAMESPACE,
                "--ignore-not-found=true",
                "--timeout=60s",
                "--wait=true",
            ],
        );
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "stellarnode",
                STANDBY_NODE_NAME,
                "-n",
                STANDBY_NAMESPACE,
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
                PRIMARY_NAMESPACE,
                "--ignore-not-found=true",
            ],
        );
        let _ = run_cmd_quiet(
            "kubectl",
            &[
                "delete",
                "namespace",
                STANDBY_NAMESPACE,
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
            "command failed: {} {:?}\nstdout:\n{}\nstderr:\n{}",
            program, args, stdout, stderr
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_cmd_with_stdin(program: &str, args: &[&str], input: &str) -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
        cmd.env("KUBECONconfig", kubeconfig);
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
            "command failed: {} {:?}\nstdout:\n{}\nstderr:\n{}",
            program, args, stdout, stderr
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
                "timeout while waiting for {} after {:?} (attempts={})",
                label, timeout, attempts
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
  name: {operator_name}
  namespace: {operator_namespace}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {operator_name}-dr
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
  name: {operator_name}-dr
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {operator_name}-dr
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
          env:
            - name: OPERATOR_NAMESPACE
              value: {operator_namespace}
"#,
        operator_name = OPERATOR_NAME,
        operator_namespace = OPERATOR_NAMESPACE,
        image = image
    )
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
