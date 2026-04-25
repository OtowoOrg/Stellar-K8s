use crate::cli::{SimulatorCli, SimulatorCmd, SimulatorUpArgs};
use crate::Error;
use std::env;
use std::process::Command;

pub async fn run_simulator(cli: SimulatorCli) -> Result<(), Error> {
    match cli.command {
        SimulatorCmd::Up(args) => simulator_up(args).await,
    }
}

pub async fn simulator_up(args: SimulatorUpArgs) -> Result<(), Error> {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let validators = repo_root.join("examples/simulator/three-validators.yaml");
    let csi_sample = repo_root.join("config/samples/test-stellarnode.yaml");

    let have_kind = Command::new("kind")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let have_k3s = Command::new("k3s")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if args.use_k3s && have_k3s {
        println!("Using k3s: ensure the cluster is running and kubeconfig is configured.");
    } else if have_kind {
        println!("Creating kind cluster '{}' ...", args.cluster_name);
        let status = Command::new("kind")
            .args([
                "create",
                "cluster",
                "--name",
                &args.cluster_name,
                "--wait",
                "120s",
            ])
            .status()
            .map_err(|e| Error::ConfigError(format!("kind failed to start: {e}")))?;
        if !status.success() {
            println!("Note: kind create failed (cluster may already exist); continuing.");
        }
    } else {
        return Err(Error::ConfigError(
            "Neither kind nor k3s found in PATH. Install kind (https://kind.sigs.k8s.io/) or k3s."
                .to_string(),
        ));
    }

    println!("Applying StellarNode CRD ...");
    let crd_path = repo_root.join("config/crd/stellarnode-crd.yaml");
    let mut kubectl_crd = Command::new("kubectl");
    kubectl_crd.args(["apply", "-f", crd_path.to_str().unwrap()]);
    run_cmd(kubectl_crd, "kubectl apply CRD")?;

    println!("Ensuring namespace {} ...", args.namespace);
    let ns_pipe = format!(
        "kubectl create namespace {} --dry-run=client -o yaml | kubectl apply -f -",
        args.namespace
    );
    let mut sh_ns = Command::new("sh");
    sh_ns.arg("-c").arg(&ns_pipe);
    run_cmd(sh_ns, "kubectl ensure namespace")?;

    println!("Building operator image stellar-operator:sim …");
    let _ = Command::new("docker")
        .args(["build", "-t", "stellar-operator:sim", "."])
        .current_dir(&repo_root)
        .status();

    if have_kind && !args.use_k3s {
        let _ = Command::new("kind")
            .args([
                "load",
                "docker-image",
                "stellar-operator:sim",
                "--name",
                &args.cluster_name,
            ])
            .status();
    }

    println!("Applying simulator operator Deployment (dev-only RBAC) …");
    let op_yaml = temp_operator_yaml(&args.namespace)?;
    let mut kubectl_op = Command::new("kubectl");
    kubectl_op.args(["apply", "-f", &op_yaml]);
    run_cmd(kubectl_op, "kubectl apply operator")?;

    println!("Applying demo workloads …");
    if validators.exists() {
        let mut kubectl_val = Command::new("kubectl");
        kubectl_val.args(["apply", "-f", validators.to_str().unwrap()]);
        run_cmd(kubectl_val, "kubectl apply validators")?;
    } else if csi_sample.exists() {
        println!("Using {}", csi_sample.display());
        let mut kubectl_sample = Command::new("kubectl");
        kubectl_sample.args(["apply", "-f", csi_sample.to_str().unwrap()]);
        run_cmd(kubectl_sample, "kubectl apply sample")?;
    } else {
        println!("No examples/simulator/three-validators.yaml — skipping demo StellarNodes.");
    }

    println!("\n=== stellar simulator up — summary ===");
    println!(
        "  StellarNodes: kubectl get stellarnode -n {}",
        args.namespace
    );
    println!("  Services:     kubectl get svc -n {}", args.namespace);
    let _ = Command::new("kubectl")
        .args(["get", "stellarnode,svc", "-n", &args.namespace])
        .status();
    Ok(())
}

pub fn run_cmd(mut c: Command, ctx: &str) -> Result<(), Error> {
    let st = c
        .status()
        .map_err(|e| Error::ConfigError(format!("{ctx}: {e}")))?;
    if !st.success() {
        return Err(Error::ConfigError(format!("{ctx}: exit {:?}", st.code())));
    }
    Ok(())
}

pub fn temp_operator_yaml(namespace: &str) -> Result<String, Error> {
    use std::io::Write;
    let dir = std::env::temp_dir();
    let path = dir.join("stellar-operator-sim.yaml");
    let mut f = std::fs::File::create(&path).map_err(|e| Error::ConfigError(e.to_string()))?;
    write!(
        f,
        r#"apiVersion: v1
kind: ServiceAccount
metadata:
  name: stellar-operator
  namespace: {namespace}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: stellar-operator-sim
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: cluster-admin
subjects:
  - kind: ServiceAccount
    name: stellar-operator
    namespace: {namespace}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: stellar-operator
  namespace: {namespace}
  labels:
    app: stellar-operator
spec:
  replicas: 1
  selector:
    matchLabels:
      app: stellar-operator
  template:
    metadata:
      labels:
        app: stellar-operator
    spec:
      serviceAccountName: stellar-operator
      containers:
        - name: operator
          image: stellar-operator:sim
          imagePullPolicy: Never
          command: ["stellar-operator", "run", "--namespace", "{namespace}"]
          env:
            - name: RUST_LOG
              value: info
            - name: OPERATOR_NAMESPACE
              value: "{namespace}"
"#
    )
    .map_err(|e| Error::ConfigError(e.to_string()))?;
    Ok(path.to_string_lossy().to_string())
}
