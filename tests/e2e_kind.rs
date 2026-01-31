use std::error::Error;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const OPERATOR_NAMESPACE: &str = "stellar-system";
const TEST_NAMESPACE: &str = "stellar-e2e";
const OPERATOR_NAME: &str = "stellar-operator";
const NODE_NAME: &str = "test-soroban";

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
                &format!("{}-config", NODE_NAME),
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
                &format!("{}-data", NODE_NAME),
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
        return Err(format!("unexpected node image after create: {}", current_image).into());
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
                &format!("{}-data", NODE_NAME),
                "-n",
                TEST_NAMESPACE,
            ],
        );
        let config_map = run_cmd(
            "kubectl",
            &[
                "get",
                "configmap",
                &format!("{}-config", NODE_NAME),
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
  name: {operator_name}
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
  name: {operator_name}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
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
          env:
            - name: OPERATOR_NAMESPACE
              value: {operator_namespace}
"#,
        operator_name = OPERATOR_NAME,
        operator_namespace = OPERATOR_NAMESPACE,
        image = image
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
  name: {node_name}
  namespace: {namespace}
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
"#,
        node_name = NODE_NAME,
        namespace = TEST_NAMESPACE,
        version = version,
        replicas = replicas,
        suspended = suspended
    )
}
