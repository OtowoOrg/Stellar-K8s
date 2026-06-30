/// tests/quickstart_smoke_test.rs
///
/// E2E smoke tests that validate the quickstart deployment path
/// (`make quickstart`).  These tests create a kind cluster, install CRDs,
/// deploy the operator via Helm, apply a sample StellarNode, and verify
/// that the operator reconciles the resource.
///
/// # Usage
///
/// ```bash
/// # Run all quickstart smoke tests (requires kind, kubectl, helm, docker)
/// cargo test --test quickstart_smoke_test -- --ignored --nocapture
///
/// # Run a specific test
/// cargo test --test quickstart_smoke_test quickstart_operator_boots -- --ignored --nocapture
/// ```
///
/// # Environment variables
///
/// | Variable | Default | Description |
/// |---|---|---|
/// | `KIND_CLUSTER_NAME` | `qs-smoke-test` | Name of the kind cluster |
/// | `E2E_OPERATOR_IMAGE` | `stellar-operator:smoke-test` | Operator image tag |
/// | `SKIP_CLUSTER_SETUP` | `false` | Skip cluster creation (use existing) |
/// | `SKIP_TEARDOWN` | `false` | Keep cluster running after test |
mod common;

use crate::common::{apply_manifest, run_kubectl_output, skip_if_tools_missing, NamespaceGuard};
use std::process::Command;
use std::time::{Duration, Instant};

/// Name for the kind cluster used by smoke tests.
fn cluster_name() -> String {
    std::env::var("KIND_CLUSTER_NAME").unwrap_or_else(|_| "qs-smoke-test".to_string())
}

/// Operator image tag used in the Helm chart.
fn operator_image() -> String {
    std::env::var("E2E_OPERATOR_IMAGE")
        .unwrap_or_else(|_| "stellar-operator:smoke-test".to_string())
}

fn skip_cluster_setup() -> bool {
    std::env::var("SKIP_CLUSTER_SETUP").as_deref() == Ok("true")
}

fn skip_teardown() -> bool {
    std::env::var("SKIP_TEARDOWN").as_deref() == Ok("true")
}

const OPERATOR_NAMESPACE: &str = "stellar-system";
const NODE_NAMESPACE: &str = "stellar";
const TIMEOUT_SECS: u64 = 180;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn kind_binary() -> &'static str {
    "kind"
}

fn create_kind_cluster(name: &str) {
    let mut cmd = Command::new(kind_binary());
    cmd.args(["create", "cluster", "--name", name, "--wait", "120s"]);
    let status = cmd.status().expect("failed to run kind create cluster");
    assert!(status.success(), "kind create cluster failed");
}

fn delete_kind_cluster(name: &str) {
    let _ = Command::new(kind_binary())
        .args(["delete", "cluster", "--name", name])
        .status();
}

fn build_and_load_image(image: &str) {
    let status = Command::new("docker")
        .args(["build", "-t", image, "."])
        .status()
        .expect("failed to run docker build");
    assert!(status.success(), "docker build failed");

    let status = Command::new(kind_binary())
        .args(["load", "docker-image", image, "--name", &cluster_name()])
        .status()
        .expect("failed to load image into kind");
    assert!(status.success(), "kind load docker-image failed");
}

fn install_crds() {
    let status = Command::new("kubectl")
        .args(["apply", "-f", "config/crd/stellarnode-crd.yaml"])
        .status()
        .expect("failed to apply CRDs");
    assert!(status.success(), "CRD install failed");

    // Wait for CRD to be established
    let status = Command::new("kubectl")
        .args([
            "wait",
            "--for=condition=established",
            "--timeout=30s",
            "crd/stellarnodes.stellar.org",
        ])
        .status();
    if let Ok(s) = status {
        let _ = s;
    }
}

fn deploy_operator_helm(image: &str) {
    let status = Command::new("helm")
        .args([
            "upgrade",
            "--install",
            "stellar-operator",
            "charts/stellar-operator",
            "--namespace",
            OPERATOR_NAMESPACE,
            "--create-namespace",
            "--set",
            &format!(
                "image.tag={}",
                image.trim_start_matches("stellar-operator:")
            ),
            "--set",
            "image.pullPolicy=Never",
            "--wait",
            "--timeout",
            "120s",
        ])
        .status()
        .expect("failed to run helm install");
    assert!(status.success(), "Helm install failed");
}

fn wait_for_operator_ready(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut ready = false;
    while Instant::now() < deadline {
        let output = Command::new("kubectl")
            .args([
                "get",
                "deployment",
                "stellar-operator",
                "-n",
                OPERATOR_NAMESPACE,
                "-o",
                "jsonpath={.status.readyReplicas}",
            ])
            .output();

        if let Ok(out) = output {
            if let Ok(stdout) = String::from_utf8(out.stdout) {
                if stdout.trim() == "1" {
                    ready = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    assert!(ready, "Operator did not become ready within {timeout:?}");
}

fn apply_sample_stellarnode() {
    apply_manifest(include_str!("../config/samples/test-stellarnode.yaml"))
        .expect("failed to apply sample StellarNode");
}

fn wait_for_stellarnode_ready(name: &str, namespace: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut ready = false;
    while Instant::now() < deadline {
        let output = Command::new("kubectl")
            .args([
                "get",
                "stellarnode",
                name,
                "-n",
                namespace,
                "-o",
                "jsonpath={.status.phase}",
            ])
            .output();

        if let Ok(out) = output {
            if let Ok(stdout) = String::from_utf8(out.stdout) {
                let phase = stdout.trim();
                if !phase.is_empty() && phase != "Pending" {
                    ready = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    assert!(
        ready,
        "StellarNode {name} did not progress past Pending within {timeout:?}"
    );
}

fn operator_logs_contain(pattern: &str) -> bool {
    let output = Command::new("kubectl")
        .args([
            "logs",
            "--selector=app.kubernetes.io/name=stellar-operator",
            "-n",
            OPERATOR_NAMESPACE,
            "--tail=50",
        ])
        .output();

    match output {
        Ok(out) => {
            let logs = String::from_utf8_lossy(&out.stdout);
            logs.contains(pattern)
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Validates that the operator builds, deploys, and connects to the cluster.
#[test]
#[ignore]
fn quickstart_operator_boots() {
    if skip_if_tools_missing(&["kind", "kubectl", "helm", "docker"]) {
        return;
    }

    let cluster = cluster_name();
    let image = operator_image();

    // Setup
    if !skip_cluster_setup() {
        create_kind_cluster(&cluster);
    }

    let _ns_guard = if !skip_cluster_setup() {
        NamespaceGuard::create(OPERATOR_NAMESPACE)
    } else {
        // Ensure namespace exists
        if let Ok(yaml) = run_kubectl_output(&[
            "create",
            "namespace",
            OPERATOR_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ]) {
            let _ = apply_manifest(&yaml);
        }
        NamespaceGuard::create(OPERATOR_NAMESPACE)
    };

    install_crds();

    if !skip_cluster_setup() {
        build_and_load_image(&image);
    }

    deploy_operator_helm(&image);

    let timeout = Duration::from_secs(TIMEOUT_SECS);
    wait_for_operator_ready(timeout);

    // Verify operator is connected to the cluster by checking logs
    let connected = operator_logs_contain("Connected to Kubernetes cluster")
        || operator_logs_contain("starting leader election")
        || operator_logs_contain("Controller started")
        || operator_logs_contain("Started watching");
    assert!(
        connected,
        "Operator logs did not indicate cluster connection"
    );

    // Teardown
    if !skip_teardown() {
        delete_kind_cluster(&cluster);
    }
}

/// Validates the full quickstart flow: operator + StellarNode reconciliation.
#[test]
#[ignore]
fn quickstart_full_flow() {
    if skip_if_tools_missing(&["kind", "kubectl", "helm", "docker"]) {
        return;
    }

    let cluster = cluster_name();
    let image = operator_image();

    // Setup cluster
    if !skip_cluster_setup() {
        create_kind_cluster(&cluster);
    }

    let _ns_guard = if !skip_cluster_setup() {
        NamespaceGuard::create(OPERATOR_NAMESPACE)
    } else {
        if let Ok(yaml) = run_kubectl_output(&[
            "create",
            "namespace",
            OPERATOR_NAMESPACE,
            "--dry-run=client",
            "-o",
            "yaml",
        ]) {
            let _ = apply_manifest(&yaml);
        }
        NamespaceGuard::create(OPERATOR_NAMESPACE)
    };

    install_crds();

    if !skip_cluster_setup() {
        build_and_load_image(&image);
    }

    deploy_operator_helm(&image);

    let timeout = Duration::from_secs(TIMEOUT_SECS);
    wait_for_operator_ready(timeout);

    // Create namespace and apply StellarNode
    let _node_ns = NamespaceGuard::create(NODE_NAMESPACE);
    apply_sample_stellarnode();

    // Wait for the operator to reconcile the StellarNode
    wait_for_stellarnode_ready("test-stellarnode", "stellar", timeout);

    // Verify operator logs show reconciliation
    let reconciled = operator_logs_contain("reconciling")
        || operator_logs_contain("Reconciling")
        || operator_logs_contain("test-stellarnode");
    assert!(
        reconciled,
        "Operator logs did not show reconciliation of StellarNode"
    );

    // Teardown
    if !skip_teardown() {
        delete_kind_cluster(&cluster);
    }
}

/// Validates that the quickstart path works with the Helm chart defaults
/// and the operator starts successfully with minimal configuration.
#[test]
#[ignore]
fn quickstart_minimal_config() {
    if skip_if_tools_missing(&["kind", "kubectl", "helm", "docker"]) {
        return;
    }

    let cluster = format!("{}-minimal", cluster_name());
    let image = operator_image();

    if !skip_cluster_setup() {
        create_kind_cluster(&cluster);
    }

    install_crds();

    if !skip_cluster_setup() {
        build_and_load_image(&image);
    }

    // Deploy with minimal Helm values
    let status = Command::new("helm")
        .args([
            "upgrade",
            "--install",
            "stellar-operator",
            "charts/stellar-operator",
            "--namespace",
            OPERATOR_NAMESPACE,
            "--create-namespace",
            "--set",
            &format!(
                "image.tag={}",
                image.trim_start_matches("stellar-operator:")
            ),
            "--set",
            "image.pullPolicy=Never",
            "--set",
            "resources.requests.cpu=100m",
            "--set",
            "resources.requests.memory=128Mi",
            "--set",
            "resources.limits.cpu=200m",
            "--set",
            "resources.limits.memory=256Mi",
            "--wait",
            "--timeout",
            "120s",
        ])
        .status()
        .expect("failed to run helm install");
    assert!(status.success(), "Helm install with minimal config failed");

    let timeout = Duration::from_secs(TIMEOUT_SECS);
    wait_for_operator_ready(timeout);

    // Confirm operator boots with minimal config
    let running = operator_logs_contain("stellar-operator") || operator_logs_contain("Operator");
    assert!(running, "Operator did not start with minimal config");

    if !skip_teardown() {
        delete_kind_cluster(&cluster);
    }
}
