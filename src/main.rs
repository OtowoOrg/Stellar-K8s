use std::process::{self};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use clap::{Parser, Subcommand};
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use kube::ResourceExt;
use stellar_k8s::controller::archive_prune::{prune_archive, PruneArchiveArgs};
use stellar_k8s::controller::diff::{diff, DiffArgs};
use stellar_k8s::infra;
use stellar_k8s::{controller, crd::StellarNode, preflight, Error};
use tracing::{debug, info, info_span, warn, Instrument, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Stellar-K8s: Cloud-Native Kubernetes Operator for Stellar Infrastructure",
    long_about = "stellar-operator manages StellarNode custom resources on Kubernetes.\n\n\
        It reconciles the desired state of Stellar validator, Horizon, and Soroban RPC nodes,\n\
        handles leader election, optional mTLS, peer discovery, and a latency-aware scheduler.\n\n\
        EXAMPLES:\n  \
        stellar-operator run --namespace stellar-system\n  \
        stellar-operator run --namespace stellar-system --enable-mtls\n  \
        stellar-operator run --namespace stellar-system --scheduler\n  \
        stellar-operator run --namespace stellar-system --dry-run\n  \
        stellar-operator run --dump-config\n  \
        stellar-operator webhook --bind 0.0.0.0:8443 --cert-path /tls/tls.crt --key-path /tls/tls.key\n  \
        stellar-operator info --namespace stellar-system\n  \
        stellar-operator check-crd\n  \
        stellar-operator version"
)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the operator reconciliation loop
    Run(RunArgs),
    /// Run the admission webhook server
    Webhook(WebhookArgs),
    /// Show version and build information
    Version,
    /// Show cluster information (node count) for a namespace
    Info(InfoArgs),
    /// Verify StellarNode CRD installation and expected version
    CheckCrd,
    /// Prune old history archive checkpoints
    PruneArchive(PruneArchiveArgs),
    /// Show difference between desired and live cluster state
    Diff(DiffArgs),
    /// Generate a troubleshooting runbook for a StellarNode
    GenerateRunbook(GenerateRunbookArgs),
    /// Local simulator (kind/k3s + operator + demo validators)
    Simulator(SimulatorCli),
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum LogFormat {
    Json,
    Pretty,
}

#[derive(Parser, Debug)]
#[command(
    about = "Run the operator reconciliation loop",
    long_about = "Starts the main operator process that watches StellarNode resources and reconciles\n\
        their desired state. Supports leader election, optional mTLS for the REST API,\n\
        dry-run mode, and a latency-aware scheduler mode.\n\n\
        EXAMPLES:\n  \
        stellar-operator run\n  \
        stellar-operator run --namespace stellar-system\n  \
        stellar-operator run --namespace stellar-system --enable-mtls\n  \
        stellar-operator run --namespace stellar-system --dry-run\n  \
        stellar-operator run --namespace stellar-system --scheduler --scheduler-name my-scheduler\n  \
        stellar-operator run --dump-config\n\n\
        NOTE: --scheduler and --dry-run are mutually exclusive."
)]
struct RunArgs {
    /// Enable mutual TLS for the REST API.
    ///
    /// When set, the operator provisions a CA and server certificate in the target namespace,
    /// and the REST API requires client certificates signed by that CA.
    /// Env: ENABLE_MTLS
    #[arg(long, env = "ENABLE_MTLS")]
    enable_mtls: bool,

    /// Kubernetes namespace to watch and manage StellarNode resources in.
    ///
    /// Must match the namespace where StellarNode CRs are deployed.
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,

    /// Restrict the operator to only watch and manage StellarNode resources in a specific namespace.
    ///
    /// When unset (default), the operator watches all namespaces and requires cluster-wide RBAC.
    /// When set, the operator only reconciles StellarNodes in this namespace and can run with
    /// namespace-scoped RBAC (Role/RoleBinding).
    /// Env: WATCH_NAMESPACE
    ///
    /// Example: --watch-namespace stellar-prod
    #[arg(long, env = "WATCH_NAMESPACE")]
    watch_namespace: Option<String>,

    /// Simulate reconciliation without applying any changes to the cluster.
    ///
    /// All reconciliation logic runs normally, but no Kubernetes API write calls are made.
    /// Useful for validating operator behaviour before a production rollout.
    /// Mutually exclusive with --scheduler.
    /// Env: DRY_RUN
    ///
    /// Example: --dry-run
    #[arg(long, env = "DRY_RUN")]
    dry_run: bool,

    /// Run the latency-aware scheduler instead of the standard operator reconciler.
    ///
    /// The scheduler assigns pending pods to nodes based on measured network latency
    /// between Stellar validators. Only one mode (scheduler or operator) runs per process.
    /// Mutually exclusive with --dry-run.
    /// Env: RUN_SCHEDULER
    ///
    /// Example: --scheduler --scheduler-name stellar-scheduler
    #[arg(long, env = "RUN_SCHEDULER")]
    scheduler: bool,

    /// Name registered with the Kubernetes scheduler framework when --scheduler is active.
    ///
    /// This name must match the `schedulerName` field in pod specs that should be
    /// handled by this scheduler instance.
    /// Env: SCHEDULER_NAME
    ///
    /// Example: --scheduler-name stellar-latency-scheduler
    #[arg(long, env = "SCHEDULER_NAME", default_value = "stellar-scheduler")]
    scheduler_name: String,

    /// Print the resolved runtime configuration and exit without starting the operator.
    #[arg(long)]
    pub dump_config: bool,

    /// Run preflight checks and exit without starting the operator
    #[arg(long, env = "PREFLIGHT_ONLY")]
    pub preflight_only: bool,
}

impl RunArgs {
    /// Validate mutually exclusive flags and other constraints.
    /// Returns an error string suitable for display if validation fails.
    fn validate(&self) -> Result<(), String> {
        if self.scheduler && self.dry_run {
            return Err(
                "--scheduler and --dry-run are mutually exclusive: the scheduler mode does not \
                 perform reconciliation writes, so dry-run has no effect and the combination is \
                 likely a misconfiguration."
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Parser, Debug)]
struct InfoArgs {
    /// Kubernetes namespace to query for StellarNode resources.
    ///
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,
}

#[derive(Parser, Debug)]
#[command(
    about = "Generate a troubleshooting runbook for a StellarNode",
    long_about = "Generates a context-aware troubleshooting runbook tailored to the specific\n\
        configuration of a deployed StellarNode. The runbook includes:\n\n\
        - Exact kubectl commands to fetch logs from specific containers\n\
        - KMS key status checks if KMS is configured\n\
        - S3/GCS CLI commands to verify archive buckets if archiving is enabled\n\
        - Network information and expected peer connections based on quorum set\n\
        - Resource and storage troubleshooting steps\n\n\
        EXAMPLES:\n  \
        stellar-operator generate-runbook my-validator -n stellar\n  \
        stellar-operator generate-runbook my-validator -n stellar -o runbook.md\n  \
        stellar-operator generate-runbook my-validator -n stellar | less"
)]
struct GenerateRunbookArgs {
    /// Name of the StellarNode resource
    node_name: String,

    /// Kubernetes namespace containing the StellarNode
    ///
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(short, long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,

    /// Output file path (optional, defaults to stdout)
    ///
    /// Example: --output runbook.md
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(clap::Subcommand, Debug)]
enum SimulatorCmd {
    /// Create cluster, install operator manifests, print health hints
    Up(SimulatorUpArgs),
}

#[derive(Parser, Debug)]
struct SimulatorCli {
    #[command(subcommand)]
    command: SimulatorCmd,
}

#[derive(Parser, Debug)]
#[command(
    about = "Spin up a local simulator cluster with demo validators",
    long_about = "Creates a local kind or k3s cluster, applies the StellarNode CRD and operator\n\
        manifests, and deploys demo validator StellarNode resources for local development.\n\n\
        EXAMPLES:\n  \
        stellar-operator simulator up\n  \
        stellar-operator simulator up --cluster-name my-cluster --namespace stellar-dev\n  \
        stellar-operator simulator up --use-k3s"
)]
struct SimulatorUpArgs {
    /// Name of the kind cluster to create.
    ///
    /// Ignored when --use-k3s is set (k3s manages its own cluster name).
    ///
    /// Example: --cluster-name stellar-dev
    #[arg(long, default_value = "stellar-sim")]
    cluster_name: String,

    /// Kubernetes namespace for the operator and demo StellarNode resources.
    ///
    /// Example: --namespace stellar-dev
    #[arg(long, default_value = "stellar-system")]
    namespace: String,

    /// Use k3s instead of kind when both are available in PATH.
    ///
    /// k3s must already be running; the simulator will use the current kubeconfig context.
    ///
    /// Example: --use-k3s
    #[arg(long, default_value_t = false)]
    use_k3s: bool,
}

#[derive(Parser, Debug)]
#[command(
    about = "Run the admission webhook server",
    long_about = "Starts the HTTPS admission webhook server that validates and mutates StellarNode\n\
        resources on admission. Requires a valid TLS certificate and key for production use.\n\n\
        EXAMPLES:\n  \
        stellar-operator webhook --bind 0.0.0.0:8443 --cert-path /tls/tls.crt --key-path /tls/tls.key\n  \
        stellar-operator webhook --bind 127.0.0.1:8443 --log-level debug\n\n\
        NOTE: Running without --cert-path / --key-path is only suitable for local development."
)]
struct WebhookArgs {
    /// Address and port the webhook HTTPS server will listen on.
    ///
    /// Use 0.0.0.0 to listen on all interfaces, or a specific IP to restrict access.
    /// Env: WEBHOOK_BIND
    ///
    /// Example: --bind 0.0.0.0:8443
    #[arg(long, env = "WEBHOOK_BIND", default_value = "0.0.0.0:8443")]
    bind: String,

    /// Path to the PEM-encoded TLS certificate file served by the webhook.
    ///
    /// Must be signed by the CA configured in the ValidatingWebhookConfiguration.
    /// Env: WEBHOOK_CERT_PATH
    ///
    /// Example: --cert-path /etc/webhook/tls/tls.crt
    #[arg(long, env = "WEBHOOK_CERT_PATH")]
    cert_path: Option<String>,

    /// Path to the PEM-encoded TLS private key file for the webhook certificate.
    ///
    /// Must correspond to the certificate provided via --cert-path.
    /// Env: WEBHOOK_KEY_PATH
    ///
    /// Example: --key-path /etc/webhook/tls/tls.key
    #[arg(long, env = "WEBHOOK_KEY_PATH")]
    key_path: Option<String>,

    /// Minimum log level emitted by the webhook server.
    ///
    /// Accepted values: trace, debug, info, warn, error.
    /// Env: LOG_LEVEL
    ///
    /// Example: --log-level debug
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    log_level: String,

    /// Log output format (json or pretty)
    #[arg(long, env = "LOG_FORMAT", value_enum, default_value = "json")]
    log_format: LogFormat,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::parse();

    match args.command {
        Commands::Version => {
            println!("Stellar-K8s Operator v{}", env!("CARGO_PKG_VERSION"));
            println!("Build Date: {}", env!("BUILD_DATE"));
            println!("Git SHA: {}", env!("GIT_SHA"));
            println!("Rust Version: {}", env!("RUST_VERSION"));
            return Ok(());
        }
        Commands::Info(info_args) => {
            return run_info(info_args).await;
        }
        Commands::CheckCrd => {
            return run_check_crd().await;
        }
        Commands::Run(run_args) => {
            if let Err(e) = run_args.validate() {
                eprintln!("error: {e}");
                process::exit(2);
            }
            return run_operator(run_args).await;
        }
        Commands::Webhook(webhook_args) => {
            return run_webhook(webhook_args).await;
        }
        Commands::Simulator(cli) => {
            return run_simulator(cli).await;
        }
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            let mut cmd = Args::command();
            let name = cmd.get_name().to_string();
            generate(shell, &mut cmd, name, &mut std::io::stdout());
            return Ok(());
        }
        Commands::PruneArchive(prune_args) => {
            return prune_archive(prune_args).await;
        }
        Commands::Diff(diff_args) => {
            return diff(diff_args).await;
        }
        Commands::GenerateRunbook(runbook_args) => {
            return run_generate_runbook(runbook_args).await;
        }
    }
}

async fn run_simulator(cli: SimulatorCli) -> Result<(), Error> {
    match cli.command {
        SimulatorCmd::Up(args) => simulator_up(args).await,
    }
}

async fn run_check_crd() -> Result<(), Error> {
    const EXPECTED_VERSION: &str = "v1alpha1";
    const CRD_NAME: &str = "stellarnodes.stellar.org";

    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;
    let crds: Api<CustomResourceDefinition> = Api::all(client);

    let crd = match crds.get(CRD_NAME).await {
        Ok(crd) => crd,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            return Err(Error::ConfigError(format!(
                "StellarNode CRD '{CRD_NAME}' is not installed. Install with: kubectl apply -f config/crd/stellarnode-crd.yaml"
            )));
        }
        Err(e) => return Err(Error::KubeError(e)),
    };

    let versions = crd.spec.versions;
    let installed_versions = versions
        .iter()
        .filter(|v| v.served)
        .map(|v| v.name.clone())
        .collect::<Vec<_>>();

    let expected_present = versions
        .iter()
        .any(|v| v.name == EXPECTED_VERSION && v.served);

    if !expected_present {
        return Err(Error::ConfigError(format!(
            "CRD '{}' is installed but expected served version '{}' is missing. Served versions: {}",
            CRD_NAME,
            EXPECTED_VERSION,
            if installed_versions.is_empty() {
                "<none>".to_string()
            } else {
                installed_versions.join(", ")
            }
        )));
    }

    println!("CRD check passed");
    println!("CRD: {CRD_NAME}");
    println!("Expected version: {EXPECTED_VERSION}");
    println!("Served versions: {}", installed_versions.join(", "));
    Ok(())
}

async fn simulator_up(args: SimulatorUpArgs) -> Result<(), Error> {
    use std::process::Command;

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

fn run_cmd(mut c: std::process::Command, ctx: &str) -> Result<(), Error> {
    let st = c
        .status()
        .map_err(|e| Error::ConfigError(format!("{ctx}: {e}")))?;
    if !st.success() {
        return Err(Error::ConfigError(format!("{ctx}: exit {:?}", st.code())));
    }
    Ok(())
}

fn temp_operator_yaml(namespace: &str) -> Result<String, Error> {
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

async fn run_info(args: InfoArgs) -> Result<(), Error> {
    use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
    use k8s_openapi::api::core::v1::Service;

    // Initialize Kubernetes client
    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    let api: kube::Api<StellarNode> = kube::Api::namespaced(client.clone(), &args.namespace);
    let nodes = api
        .list(&Default::default())
        .await
        .map_err(Error::KubeError)?;

    println!("Managed Stellar Nodes: {}", nodes.items.len());
    println!();

    // Display detailed information for each node
    for node in &nodes.items {
        let name = node.name_any();
        let node_type = format!("{:?}", node.spec.node_type);
        let network = format!("{:?}", node.spec.network);
        let replicas = node.spec.replicas;

        println!("StellarNode: {name}");
        println!("  Type: {node_type}");
        println!("  Network: {network}");
        println!("  Replicas: {replicas}");

        // Find owned Deployments
        let deployment_api: kube::Api<Deployment> =
            kube::Api::namespaced(client.clone(), &args.namespace);
        let label_selector =
            format!("app.kubernetes.io/instance={name},app.kubernetes.io/name=stellar-node");
        let deployments = deployment_api
            .list(&kube::api::ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        if !deployments.items.is_empty() {
            println!("  Deployments:");
            for deployment in &deployments.items {
                let dep_name = deployment.metadata.name.as_deref().unwrap_or("unknown");
                let ready = deployment
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                let desired = deployment
                    .spec
                    .as_ref()
                    .and_then(|s| s.replicas)
                    .unwrap_or(0);
                println!("    - {dep_name} ({ready}/{desired} ready)");
            }
        }

        // Find owned StatefulSets
        let statefulset_api: kube::Api<StatefulSet> =
            kube::Api::namespaced(client.clone(), &args.namespace);
        let statefulsets = statefulset_api
            .list(&kube::api::ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        if !statefulsets.items.is_empty() {
            println!("  StatefulSets:");
            for sts in &statefulsets.items {
                let sts_name = sts.metadata.name.as_deref().unwrap_or("unknown");
                let ready = sts
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                let desired = sts.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
                println!("    - {sts_name} ({ready}/{desired} ready)");
            }
        }

        // Find owned Services
        let service_api: kube::Api<Service> =
            kube::Api::namespaced(client.clone(), &args.namespace);
        let services = service_api
            .list(&kube::api::ListParams::default().labels(&label_selector))
            .await
            .map_err(Error::KubeError)?;

        if !services.items.is_empty() {
            println!("  Services:");
            for service in &services.items {
                let svc_name = service.metadata.name.as_deref().unwrap_or("unknown");
                let svc_type = service
                    .spec
                    .as_ref()
                    .and_then(|s| s.type_.as_deref())
                    .unwrap_or("ClusterIP");
                let cluster_ip = service
                    .spec
                    .as_ref()
                    .and_then(|s| s.cluster_ip.as_deref())
                    .unwrap_or("None");
                println!("    - {svc_name} (type: {svc_type}, IP: {cluster_ip})");
            }
        }

        match infra::resolve_stellar_node_infra(&client, node).await {
            Ok(summary) if !summary.is_empty() => {
                println!("  Infra Details:");
                println!(
                    "    Hardware Generation: {}",
                    summary.hardware_generation_label()
                );

                for assignment in &summary.assignments {
                    let kube_node = assignment.kubernetes_node.as_deref().unwrap_or("pending");
                    println!(
                        "    - Pod {} on {} ({})",
                        assignment.pod_name, kube_node, assignment.hardware_generation
                    );

                    if assignment.feature_labels.is_empty() {
                        println!("      feature.node.kubernetes.io/* labels: none found");
                    } else {
                        println!("      feature.node.kubernetes.io/* labels:");
                        for (key, value) in &assignment.feature_labels {
                            println!("        - {key}={value}");
                        }
                    }
                }
            }
            Ok(_) => {
                println!("  Infra Details:");
                println!("    Hardware Generation: unknown");
                println!("    Pods are not scheduled yet, so node feature labels are unavailable.");
            }
            Err(err) => {
                println!("  Infra Details:");
                println!("    Hardware Generation: unknown");
                println!("    Failed to inspect Kubernetes node labels: {err}");
            }
        }

        println!();
    }

    Ok(())
}

async fn run_generate_runbook(args: GenerateRunbookArgs) -> Result<(), Error> {
    use kube::Client;
    use kube::api::Api;
    use stellar_k8s::runbook::generate_runbook;

    // Create Kubernetes client
    let client = Client::try_default()
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to create Kubernetes client: {e}")))?;

    // Get the StellarNode resource
    let api: Api<stellar_k8s::crd::StellarNode> = Api::namespaced(client, &args.namespace);
    let node = api
        .get(&args.node_name)
        .await
        .map_err(|e| Error::NotFound {
            kind: "StellarNode".to_string(),
            name: args.node_name.clone(),
            namespace: args.namespace.clone(),
        })?;

    // Generate the runbook
    let runbook = generate_runbook(&node)?;

    // Output to file or stdout
    if let Some(output_path) = args.output {
        std::fs::write(&output_path, &runbook)
            .map_err(|e| Error::IoError(e))?;
        println!("Runbook generated successfully: {}", output_path);
    } else {
        println!("{}", runbook);
    }

    Ok(())
}

#[cfg(feature = "admission-webhook")]
async fn run_webhook(args: WebhookArgs) -> Result<(), Error> {
    use stellar_k8s::webhook::{runtime::WasmRuntime, server::WebhookServer};

    // Initialize tracing
    let env_filter = EnvFilter::builder()
        .with_default_directive(args.log_level.parse().unwrap_or(Level::INFO.into()))
        .from_env_lossy();

    let fmt_layer = fmt::layer().json().with_target(true);

    let namespace = std::env::var("OPERATOR_NAMESPACE").unwrap_or_else(|_| "default".to_string());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    let root_span =
        info_span!("operator", node_name = "-", namespace = %namespace, reconcile_id = "-");
    let _root_enter = root_span.enter();

    info!(
        "Starting Webhook Server v{} on {}",
        env!("CARGO_PKG_VERSION"),
        args.bind
    );

    // Parse bind address
    let addr: std::net::SocketAddr = args
        .bind
        .parse()
        .map_err(|e| Error::ConfigError(format!("Invalid bind address: {e}")))?;

    // Initialize Wasm runtime
    let runtime = WasmRuntime::new()
        .map_err(|e| Error::ConfigError(format!("Failed to initialize Wasm runtime: {e}")))?;

    // Create webhook server
    let mut server = WebhookServer::new(runtime);

    // Configure TLS if provided
    if let (Some(cert_path), Some(key_path)) = (args.cert_path, args.key_path) {
        info!("Configuring TLS with cert: {cert_path}, key: {key_path}");
        server = server.with_tls(cert_path, key_path);
    } else {
        warn!("Running webhook server without TLS (not recommended for production)");
    }

    // Start the server
    info!("Webhook server listening on {addr}");
    server
        .start(addr)
        .await
        .map_err(|e| Error::ConfigError(format!("Webhook server error: {e}")))?;

    Ok(())
}

#[cfg(not(feature = "admission-webhook"))]
async fn run_webhook(_args: WebhookArgs) -> Result<(), Error> {
    Err(Error::ConfigError(
        "Webhook feature not enabled. Rebuild with --features admission-webhook".to_string(),
    ))
}

async fn run_operator(args: RunArgs) -> Result<(), Error> {
    // Handle --dump-config: print resolved configuration and exit.
    if args.dump_config {
        let operator_config = stellar_k8s::controller::OperatorConfig::load();
        let resolved = serde_json::json!({
            "cli": {
                "namespace": args.namespace,
                "watch_namespace": args.watch_namespace,
                "enable_mtls": args.enable_mtls,
                "dry_run": args.dry_run,
                "scheduler": args.scheduler,
                "scheduler_name": args.scheduler_name,
            },
            "operator_config": operator_config,
        });
        println!(
            "{}",
            serde_yaml::to_string(&resolved)
                .unwrap_or_else(|_| serde_json::to_string_pretty(&resolved).unwrap())
        );
        return Ok(());
    }

    // Initialize tracing with OpenTelemetry
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();

    let (env_filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);

    let fmt_layer = fmt::layer().json().with_target(true);

    // Register the subscriber with both stdout logging and OpenTelemetry tracing
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    // Only enable OTEL if an endpoint is provided or via a flag
    let otel_enabled = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();

    if otel_enabled {
        let otel_layer = stellar_k8s::telemetry::init_telemetry(&registry);
        registry.with(otel_layer).init();
    } else {
        registry.init();
    }

    let root_span = info_span!(
        "operator",
        node_name = "-",
        namespace = %args.namespace,
        reconcile_id = "-"
    );
    let _root_enter = root_span.enter();

    if otel_enabled {
        info!("OpenTelemetry tracing initialized");
    } else {
        info!("OpenTelemetry tracing disabled (OTEL_EXPORTER_OTLP_ENDPOINT not set)");
    }

    info!(
        "Starting Stellar-K8s Operator v{}",
        env!("CARGO_PKG_VERSION")
    );

    // Initialise operator build-info metric (Issue #301)
    #[cfg(feature = "metrics")]
    {
        stellar_k8s::controller::metrics::init_operator_info();
    }

    // Initialize Kubernetes client
    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    info!("Connected to Kubernetes cluster");

    // Run preflight self-checks
    let preflight_results = preflight::run_preflight_checks(&client, &args.namespace).await;
    preflight::print_diagnostic_summary(&preflight_results);

    if args.preflight_only {
        info!("--preflight-only flag set; exiting after diagnostics.");
        return preflight::evaluate_results(&preflight_results);
    }

    preflight::evaluate_results(&preflight_results)?;

    // If --scheduler flag is set, run the latency-aware scheduler instead
    if args.scheduler {
        info!(
            "Running in scheduler mode with name: {}",
            args.scheduler_name
        );
        let scheduler = stellar_k8s::scheduler::core::Scheduler::new(client, args.scheduler_name);
        return scheduler
            .run()
            .await
            .map_err(|e| Error::ConfigError(e.to_string()));
    }

    let client_clone = client.clone();
    let namespace = args.namespace.clone();

    let mtls_config = if args.enable_mtls {
        info!("Initializing mTLS for Operator...");

        controller::mtls::ensure_ca(&client_clone, &namespace).await?;
        controller::mtls::ensure_server_cert(
            &client_clone,
            &namespace,
            vec![
                "stellar-operator".to_string(),
                format!("stellar-operator.{}", namespace),
            ],
        )
        .await?;

        let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
            kube::Api::namespaced(client_clone.clone(), &namespace);
        let secret = secrets
            .get(controller::mtls::SERVER_CERT_SECRET_NAME)
            .await
            .map_err(Error::KubeError)?;
        Some(controller::mtls::load_mtls_config_from_secret(&secret)?)
    } else {
        None
    };
    // Leader election configuration
    let leader_namespace =
        std::env::var("POD_NAMESPACE").unwrap_or_else(|_| args.namespace.clone());
    let holder_identity = std::env::var("HOSTNAME").unwrap_or_else(|_| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown-host".to_string())
    });

    info!("Leader election using holder ID: {}", holder_identity);

    let is_leader = Arc::new(AtomicBool::new(false));

    {
        let lease_client = client.clone();
        let lease_ns = leader_namespace.clone();
        let identity = holder_identity.clone();
        let is_leader_bg = Arc::clone(&is_leader);

        tokio::spawn(
            async move {
                run_leader_election(lease_client, &lease_ns, &identity, is_leader_bg).await;
            }
            .instrument(root_span.clone()),
        );
    }

    // Update leader-status and uptime metrics every 10 s (Issue #301)
    #[cfg(feature = "metrics")]
    {
        let is_leader_metrics = Arc::clone(&is_leader);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let leader = is_leader_metrics.load(std::sync::atomic::Ordering::Relaxed);
                stellar_k8s::controller::metrics::set_leader_status(leader);
                stellar_k8s::controller::metrics::inc_uptime_seconds(10);
            }
        });
    }

    // Create shared controller state
    let operator_config = stellar_k8s::controller::OperatorConfig::load();
    let state = Arc::new(controller::ControllerState {
        client: client.clone(),
        enable_mtls: args.enable_mtls,
        operator_namespace: args.namespace.clone(),
        watch_namespace: args.watch_namespace.clone(),
        mtls_config: mtls_config.clone(),
        dry_run: args.dry_run,
        is_leader: Arc::clone(&is_leader),
        event_reporter: kube::runtime::events::Reporter {
            controller: "stellar-operator".to_string(),
            instance: None,
        },
        operator_config: Arc::new(operator_config),
        reconcile_id_counter: std::sync::atomic::AtomicU64::new(0),
        last_reconcile_success: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        log_reload_handle: reload_handle,
        log_level_expires_at: Arc::new(tokio::sync::Mutex::new(None)),
    });

    // Start the peer discovery manager
    let peer_discovery_client = client.clone();
    let peer_discovery_config = controller::PeerDiscoveryConfig::default();
    tokio::spawn(
        async move {
            let manager =
                controller::PeerDiscoveryManager::new(peer_discovery_client, peer_discovery_config);
            if let Err(e) = manager.run().await {
                tracing::error!("Peer discovery manager error: {:?}", e);
            }
        }
        .instrument(root_span.clone()),
    );

    // Start the feature-flag watcher (watches stellar-operator-config ConfigMap)
    let feature_flags = controller::feature_flags::new_shared();
    {
        let ff_client = client.clone();
        let ff_namespace = args.namespace.clone();
        let ff_flags = feature_flags.clone();
        tokio::spawn(async move {
            controller::watch_feature_flags(ff_client, ff_namespace, ff_flags).await;
        });
    }

    // Start the REST API server and optional mTLS certificate rotation
    #[cfg(feature = "rest-api")]
    {
        let api_state = state.clone();
        let rustls_config = mtls_config
            .as_ref()
            .and_then(|cfg| {
                stellar_k8s::rest_api::build_tls_server_config(
                    &cfg.cert_pem,
                    &cfg.key_pem,
                    &cfg.ca_pem,
                )
                .ok()
            })
            .map(axum_server::tls_rustls::RustlsConfig::from_config);
        let server_tls = rustls_config.clone();

        tokio::spawn(
            async move {
                if let Err(e) = stellar_k8s::rest_api::run_server(api_state, server_tls).await {
                    tracing::error!("REST API server error: {:?}", e);
                }
            }
            .instrument(root_span.clone()),
        );

        // Certificate rotation: when mTLS is enabled, periodically check and rotate
        // server cert if within threshold, then graceful reload of TLS config
        if let (true, Some(rustls_config)) = (args.enable_mtls, rustls_config) {
            let rotation_client = client.clone();
            let rotation_namespace = args.namespace.clone();
            let rotation_dns = vec![
                "stellar-operator".to_string(),
                format!("stellar-operator.{}", args.namespace),
            ];
            let rotation_threshold_days = std::env::var("CERT_ROTATION_THRESHOLD_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(controller::mtls::DEFAULT_CERT_ROTATION_THRESHOLD_DAYS);
            let is_leader_rot = Arc::clone(&is_leader);

            tokio::spawn(
                async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // check hourly
                    interval.tick().await; // first tick completes immediately
                    loop {
                        interval.tick().await;
                        if !is_leader_rot.load(Ordering::Relaxed) {
                            continue;
                        }
                        match controller::mtls::maybe_rotate_server_cert(
                            &rotation_client,
                            &rotation_namespace,
                            rotation_dns.clone(),
                            rotation_threshold_days,
                        )
                        .await
                        {
                            Ok(true) => {
                                // Rotation performed: fetch new secret and reload TLS
                                let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
                                    kube::Api::namespaced(
                                        rotation_client.clone(),
                                        &rotation_namespace,
                                    );
                                if let Ok(secret) =
                                    secrets.get(controller::mtls::SERVER_CERT_SECRET_NAME).await
                                {
                                    if let (Some(cert), Some(key), Some(ca)) = (
                                        secret.data.as_ref().and_then(|d| d.get("tls.crt")),
                                        secret.data.as_ref().and_then(|d| d.get("tls.key")),
                                        secret.data.as_ref().and_then(|d| d.get("ca.crt")),
                                    ) {
                                        match stellar_k8s::rest_api::build_tls_server_config(
                                            &cert.0, &key.0, &ca.0,
                                        ) {
                                            Ok(new_config) => {
                                                rustls_config.reload_from_config(new_config);
                                                info!(
                                                "TLS server config reloaded with new certificate"
                                            );
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                "Failed to build TLS config after rotation: {:?}",
                                                e
                                            );
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(false) => {}
                            Err(e) => {
                                tracing::error!("Certificate rotation check failed: {:?}", e);
                            }
                        }
                    }
                }
                .instrument(root_span.clone()),
            );
        }
    }

    // Run the main controller loop.
    // The kube-rs controller already listens for OS signals via .shutdown_on_signal(),
    // so this select! adds explicit lease release before the process exits.
    let shutdown_state = state.clone();
    let shutdown_client = client.clone();
    let shutdown_namespace = args.namespace.clone();
    let shutdown_is_leader = Arc::clone(&is_leader);
    let shutdown_identity = holder_identity.clone();

    let result = tokio::select! {
        res = controller::run_controller(state) => {
            res
        }
        _ = wait_for_shutdown_signal() => {
            info!("Shutdown signal received – draining reconciliations and releasing leader lease...");
            // Mark as non-leader so the renewal loop stops promoting us.
            shutdown_is_leader.store(false, std::sync::atomic::Ordering::Relaxed);
            drop(shutdown_state); // release controller state references
            // Release the Kubernetes Lease so a peer can take over immediately.
            release_leader_lease(&shutdown_client, &shutdown_namespace, &shutdown_identity).await;
            // The controller future's .shutdown_on_signal() will have already
            // stopped processing new work by this point; return cleanly.
            Ok(())
        }
    };

    // Flush any remaining traces
    stellar_k8s::telemetry::shutdown_telemetry();

    result
}

/// Wait for SIGTERM or SIGINT (Ctrl-C).
async fn wait_for_shutdown_signal() {
    use tokio::signal;
    #[cfg(unix)]
    {
        use signal::unix::{signal as unix_signal, SignalKind};
        let mut sigterm =
            unix_signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
        let mut sigint =
            unix_signal(SignalKind::interrupt()).expect("Failed to register SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => { info!("Received SIGTERM"); }
            _ = sigint.recv()  => { info!("Received SIGINT");  }
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl-C");
        info!("Received Ctrl-C");
    }
}

/// Clear holderIdentity on the leader Lease so peers can promote immediately.
/// Errors are logged but not propagated – we are already shutting down.
async fn release_leader_lease(client: &kube::Client, namespace: &str, identity: &str) {
    use k8s_openapi::api::coordination::v1::Lease;
    use kube::api::{Api, Patch, PatchParams};

    let leases: Api<Lease> = Api::namespaced(client.clone(), namespace);
    let existing = match leases.get(LEASE_NAME).await {
        Ok(l) => l,
        Err(e) => {
            warn!("Could not fetch lease for release: {:?}", e);
            return;
        }
    };
    let currently_held_by = existing
        .spec
        .as_ref()
        .and_then(|s| s.holder_identity.as_deref())
        .unwrap_or("");
    if currently_held_by != identity {
        debug!("Lease is held by {currently_held_by:?}, skipping release");
        return;
    }
    let patch = serde_json::json!({ "spec": { "holderIdentity": null } });
    match leases
        .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
        .await
    {
        Ok(_) => info!("Released leader lease {LEASE_NAME}"),
        Err(e) => warn!("Failed to release leader lease: {:?}", e),
    }
}

const LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;
const RENEW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

async fn run_leader_election(
    client: kube::Client,
    namespace: &str,
    identity: &str,
    is_leader: Arc<AtomicBool>,
) {
    let leases: Api<Lease> = Api::namespaced(client, namespace);

    loop {
        match try_acquire_or_renew(&leases, identity).await {
            Ok(true) => {
                if !is_leader.load(Ordering::Relaxed) {
                    info!("Acquired leadership for lease {}", LEASE_NAME);
                }
                is_leader.store(true, Ordering::Relaxed);
                tokio::time::sleep(RENEW_INTERVAL).await;
            }
            Ok(false) => {
                if is_leader.load(Ordering::Relaxed) {
                    warn!("Lost leadership for lease {}", LEASE_NAME);
                }
                is_leader.store(false, Ordering::Relaxed);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(e) => {
                warn!("Leader election error: {:?}", e);
                is_leader.store(false, Ordering::Relaxed);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
        }
    }
}

async fn try_acquire_or_renew(leases: &Api<Lease>, identity: &str) -> Result<bool, kube::Error> {
    let now = Utc::now();

    match leases.get(LEASE_NAME).await {
        Ok(existing) => {
            let spec = existing.spec.as_ref();
            let current_holder = spec.and_then(|s| s.holder_identity.as_deref());

            if current_holder == Some(identity) {
                let patch = serde_json::json!({
                    "spec": {
                        "renewTime": MicroTime(now),
                        "leaseDurationSeconds": LEASE_DURATION_SECS,
                    }
                });
                leases
                    .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
                return Ok(true);
            }

            let expired = spec
                .and_then(|s| s.renew_time.as_ref())
                .map(|renew| {
                    let duration = spec
                        .and_then(|s| s.lease_duration_seconds)
                        .unwrap_or(LEASE_DURATION_SECS);
                    let expiry = renew.0 + chrono::Duration::seconds(duration as i64);
                    now > expiry
                })
                .unwrap_or(true);

            if expired {
                info!(
                    "Lease held by {:?} has expired, taking over",
                    current_holder
                );
                let patch = serde_json::json!({
                    "spec": {
                        "holderIdentity": identity,
                        "acquireTime": MicroTime(now),
                        "renewTime": MicroTime(now),
                        "leaseDurationSeconds": LEASE_DURATION_SECS,
                    }
                });
                leases
                    .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            let lease = Lease {
                metadata: ObjectMeta {
                    name: Some(LEASE_NAME.to_string()),
                    namespace: Some(
                        leases
                            .resource_url()
                            .split('/')
                            .nth(5)
                            .unwrap_or("default")
                            .to_string(),
                    ),
                    ..Default::default()
                },
                spec: Some(k8s_openapi::api::coordination::v1::LeaseSpec {
                    holder_identity: Some(identity.to_string()),
                    acquire_time: Some(MicroTime(now)),
                    renew_time: Some(MicroTime(now)),
                    lease_duration_seconds: Some(LEASE_DURATION_SECS),
                    ..Default::default()
                }),
            };
            leases.create(&PostParams::default(), &lease).await?;
            info!("Created lease {} with holder {}", LEASE_NAME, identity);
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::Parser;

    // Helper: parse RunArgs from a slice of &str (simulates `stellar-operator run <args>`)
    fn parse_run(args: &[&str]) -> Result<RunArgs, clap::Error> {
        // Prepend a fake binary name so clap sees argv[0]
        let mut full: Vec<&str> = vec!["stellar-operator", "run"];
        full.extend_from_slice(args);
        // Parse via the top-level Args so subcommand routing works
        let parsed = Args::try_parse_from(full)?;
        match parsed.command {
            Commands::Run(r) => Ok(r),
            _ => panic!("expected Run subcommand"),
        }
    }

    // ── defaults ────────────────────────────────────────────────────────────

    #[test]
    fn run_defaults() {
        let args = parse_run(&[]).expect("default parse should succeed");
        assert_eq!(args.namespace, "default");
        assert!(!args.enable_mtls);
        assert!(!args.dry_run);
        assert!(!args.scheduler);
        assert_eq!(args.scheduler_name, "stellar-scheduler");
        assert!(!args.dump_config);
    }

    // ── individual flags ─────────────────────────────────────────────────────

    #[test]
    fn run_namespace_flag() {
        let args = parse_run(&["--namespace", "stellar-system"]).unwrap();
        assert_eq!(args.namespace, "stellar-system");
    }

    #[test]
    fn run_watch_namespace_flag() {
        let args = parse_run(&["--watch-namespace", "stellar-prod"]).unwrap();
        assert_eq!(args.watch_namespace, Some("stellar-prod".to_string()));
    }

    #[test]
    fn run_dry_run_flag() {
        let args = parse_run(&["--dry-run"]).unwrap();
        assert!(args.dry_run);
    }

    #[test]
    fn run_scheduler_flag() {
        let args = parse_run(&["--scheduler"]).unwrap();
        assert!(args.scheduler);
    }

    #[test]
    fn run_scheduler_name_flag() {
        let args = parse_run(&["--scheduler", "--scheduler-name", "my-sched"]).unwrap();
        assert_eq!(args.scheduler_name, "my-sched");
    }

    #[test]
    fn run_enable_mtls_flag() {
        let args = parse_run(&["--enable-mtls"]).unwrap();
        assert!(args.enable_mtls);
    }

    #[test]
    fn run_dump_config_flag() {
        let args = parse_run(&["--dump-config"]).unwrap();
        assert!(args.dump_config);
    }

    // ── mutual exclusion validation ──────────────────────────────────────────

    #[test]
    fn scheduler_and_dry_run_are_mutually_exclusive() {
        let args = parse_run(&["--scheduler", "--dry-run"]).unwrap();
        let result = args.validate();
        assert!(
            result.is_err(),
            "--scheduler and --dry-run should fail validation"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("mutually exclusive"),
            "error message should mention 'mutually exclusive', got: {msg}"
        );
    }

    #[test]
    fn scheduler_alone_is_valid() {
        let args = parse_run(&["--scheduler"]).unwrap();
        assert!(args.validate().is_ok());
    }

    #[test]
    fn dry_run_alone_is_valid() {
        let args = parse_run(&["--dry-run"]).unwrap();
        assert!(args.validate().is_ok());
    }

    #[test]
    fn no_flags_is_valid() {
        let args = parse_run(&[]).unwrap();
        assert!(args.validate().is_ok());
    }

    // ── dump-config does not conflict with other flags ───────────────────────

    #[test]
    fn dump_config_with_namespace_is_valid() {
        let args = parse_run(&["--dump-config", "--namespace", "prod"]).unwrap();
        assert!(args.validate().is_ok());
        assert_eq!(args.namespace, "prod");
        assert!(args.dump_config);
    }

    // ── webhook args ─────────────────────────────────────────────────────────

    fn parse_webhook(args: &[&str]) -> Result<WebhookArgs, clap::Error> {
        let mut full: Vec<&str> = vec!["stellar-operator", "webhook"];
        full.extend_from_slice(args);
        let parsed = Args::try_parse_from(full)?;
        match parsed.command {
            Commands::Webhook(w) => Ok(w),
            _ => panic!("expected Webhook subcommand"),
        }
    }

    #[test]
    fn webhook_defaults() {
        let args = parse_webhook(&[]).unwrap();
        assert_eq!(args.bind, "0.0.0.0:8443");
        assert_eq!(args.log_level, "info");
        assert!(args.cert_path.is_none());
        assert!(args.key_path.is_none());
    }

    #[test]
    fn webhook_custom_bind() {
        let args = parse_webhook(&["--bind", "127.0.0.1:9443"]).unwrap();
        assert_eq!(args.bind, "127.0.0.1:9443");
    }

    #[test]
    fn webhook_tls_paths() {
        let args =
            parse_webhook(&["--cert-path", "/tls/tls.crt", "--key-path", "/tls/tls.key"]).unwrap();
        assert_eq!(args.cert_path.as_deref(), Some("/tls/tls.crt"));
        assert_eq!(args.key_path.as_deref(), Some("/tls/tls.key"));
    }

    #[test]
    fn webhook_log_level() {
        let args = parse_webhook(&["--log-level", "debug"]).unwrap();
        assert_eq!(args.log_level, "debug");
    }

    // ── unknown flags are rejected ────────────────────────────────────────────

    #[test]
    fn unknown_flag_is_rejected() {
        let result = parse_run(&["--nonexistent-flag"]);
        assert!(result.is_err(), "unknown flags should be rejected by clap");
    }

    #[test]
    fn check_crd_subcommand_parses() {
        let parsed = Args::try_parse_from(["stellar-operator", "check-crd"])
            .expect("check-crd subcommand should parse");
        assert!(matches!(parsed.command, Commands::CheckCrd));
    }

    // ── simulator args ────────────────────────────────────────────────────────

    fn parse_simulator_up(args: &[&str]) -> Result<SimulatorUpArgs, clap::Error> {
        let mut full: Vec<&str> = vec!["stellar-operator", "simulator", "up"];
        full.extend_from_slice(args);
        let parsed = Args::try_parse_from(full)?;
        match parsed.command {
            Commands::Simulator(s) => match s.command {
                SimulatorCmd::Up(u) => Ok(u),
            },
            _ => panic!("expected Simulator subcommand"),
        }
    }

    #[test]
    fn simulator_up_defaults() {
        let args = parse_simulator_up(&[]).unwrap();
        assert_eq!(args.cluster_name, "stellar-sim");
        assert_eq!(args.namespace, "stellar-system");
        assert!(!args.use_k3s);
    }

    #[test]
    fn simulator_up_custom_cluster() {
        let args = parse_simulator_up(&["--cluster-name", "my-cluster"]).unwrap();
        assert_eq!(args.cluster_name, "my-cluster");
    }
}
