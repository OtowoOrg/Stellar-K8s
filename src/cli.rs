//! Command-line argument definitions for the Stellar-K8s operator.
//!
//! This module uses `clap` to define the CLI structure, including all
//! subcommands, arguments, and environment variable mappings.

use clap::{Parser, Subcommand};
use stellar_k8s::controller::archive_prune::PruneArchiveArgs;
use stellar_k8s::controller::diff::DiffArgs;
use stellar_k8s::incident;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Stellar-K8s: Cloud-Native Kubernetes Operator for Stellar Infrastructure",
    long_about = "\
\x1b[1;36m\
  ███████╗████████╗███████╗██╗     ██╗      █████╗ ██████╗       ██╗  ██╗ █████╗ ███████╗\n\
  ██╔════╝╚══██╔══╝██╔════╝██║     ██║     ██╔══██╗██╔══██╗      ██║ ██╔╝██╔══██╗██╔════╝\n\
  ███████╗   ██║   █████╗  ██║     ██║     ███████║██████╔╝█████╗█████╔╝ ╚█████╔╝███████╗\n\
  ╚════██║   ██║   ██╔══╝  ██║     ██║     ██╔══██║██╔══██╗╚════╝██╔═██╗ ██╔══██╗╚════██║\n\
  ███████║   ██║   ███████╗███████╗███████╗██║  ██║██║  ██║      ██║  ██╗╚█████╔╝███████║\n\
  ╚══════╝   ╚═╝   ╚══════╝╚══════╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝      ╚═╝  ╚═╝ ╚════╝ ╚══════╝\n\
\x1b[0m\
\x1b[1;35m  Cloud-Native Stellar Infrastructure on Kubernetes\x1b[0m\n\
\x1b[90m  Built with Rust 🦀 · Powered by kube-rs · Apache 2.0\x1b[0m\n\n\
stellar-operator manages StellarNode custom resources on Kubernetes.\n\n\
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
stellar-operator doctor\n  \
stellar-operator check-crd\n  \
stellar-operator version"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Commands,

    /// Skip the background version check against GitHub releases.
    #[arg(long, global = true, env = "STELLAR_OFFLINE")]
    pub offline: bool,

    /// Print version and exit
    #[arg(short = 'v', long, global = true)]
    pub version: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the operator reconciliation loop
    Run(RunArgs),
    /// Run the admission webhook server
    Webhook(WebhookArgs),
    /// Run the StellarBenchmark controller (can be co-located with the main operator)
    Benchmark(BenchmarkArgs),
    /// Show version and build information
    Version,
    /// Show cluster information (node count) for a namespace
    Info(InfoArgs),
    /// Verify StellarNode CRD installation and expected version
    CheckCrd,
    /// Verify local CLI tooling, Kubernetes context, and operator permissions
    Doctor(DoctorArgs),
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
    /// Install shell completion scripts to user's home directory
    InstallCompletion {
        /// Shell to install completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Incident Response Toolkit
    Incident {
        #[command(subcommand)]
        command: incident::IncidentCommands,
    },
    /// Compare performance metrics between two clusters
    BenchmarkCompare(stellar_k8s::benchmark_compare::BenchmarkCompareArgs),
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum LogFormat {
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
pub struct RunArgs {
    /// GitHub repository in owner/repo format used for label readiness preflight.
    /// If omitted, GitHub preflight is skipped.
    #[arg(long, env = "GITHUB_REPOSITORY")]
    pub github_repo: Option<String>,

    /// Enable mutual TLS for the REST API.
    ///
    /// When set, the operator provisions a CA and server certificate in the target namespace,
    /// and the REST API requires client certificates signed by that CA.
    /// Env: ENABLE_MTLS
    #[arg(long, env = "ENABLE_MTLS")]
    pub enable_mtls: bool,

    /// Kubernetes namespace to watch and manage StellarNode resources in.
    ///
    /// Must match the namespace where StellarNode CRs are deployed.
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    pub namespace: String,

    /// Restrict the operator to only watch and manage StellarNode resources in a specific namespace.
    ///
    /// When unset (default), the operator watches all namespaces and requires cluster-wide RBAC.
    /// When set, the operator only reconciles StellarNodes in this namespace and can run with
    /// namespace-scoped RBAC (Role/RoleBinding).
    /// Env: WATCH_NAMESPACE
    ///
    /// Example: --watch-namespace stellar-prod
    #[arg(long, env = "WATCH_NAMESPACE")]
    pub watch_namespace: Option<String>,

    /// Simulate reconciliation without applying any changes to the cluster.
    ///
    /// All reconciliation logic runs normally, but no Kubernetes API write calls are made.
    /// Useful for validating operator behaviour before a production rollout.
    /// Mutually exclusive with --scheduler.
    /// Env: DRY_RUN
    ///
    /// Example: --dry-run
    #[arg(long, env = "DRY_RUN")]
    pub dry_run: bool,

    /// Run the latency-aware scheduler instead of the standard operator reconciler.
    ///
    /// The scheduler assigns pending pods to nodes based on measured network latency
    /// between Stellar validators. Only one mode (scheduler or operator) runs per process.
    /// Mutually exclusive with --dry-run.
    /// Env: RUN_SCHEDULER
    ///
    /// Example: --scheduler --scheduler-name stellar-scheduler
    #[arg(long, env = "RUN_SCHEDULER")]
    pub scheduler: bool,

    /// Name registered with the Kubernetes scheduler framework when --scheduler is active.
    ///
    /// This name must match the `schedulerName` field in pod specs that should be
    /// handled by this scheduler instance.
    /// Env: SCHEDULER_NAME
    ///
    /// Example: --scheduler-name stellar-latency-scheduler
    #[arg(long, env = "SCHEDULER_NAME", default_value = "stellar-scheduler")]
    pub scheduler_name: String,

    /// Requeue interval in seconds for retriable reconciliation errors.
    #[arg(long, env = "RETRY_BUDGET_RETRIABLE_SECS", default_value_t = 15)]
    pub retry_budget_retriable_secs: u64,

    /// Requeue interval in seconds for non-retriable reconciliation errors.
    #[arg(long, env = "RETRY_BUDGET_NONRETRIABLE_SECS", default_value_t = 60)]
    pub retry_budget_nonretriable_secs: u64,

    /// Maximum HTTP retry attempts for SCP and quorum queries.
    #[arg(long, env = "RETRY_BUDGET_MAX_ATTEMPTS", default_value_t = 3)]
    pub retry_budget_max_attempts: u32,

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
    pub fn validate(&self) -> Result<(), String> {
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
pub struct InfoArgs {
    /// Kubernetes namespace to query for StellarNode resources.
    ///
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    pub namespace: String,
}

#[derive(Parser, Debug)]
pub struct DoctorArgs {
    /// Kubernetes namespace used by the operator for permission checks.
    ///
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    pub namespace: String,
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
pub struct GenerateRunbookArgs {
    /// Name of the StellarNode resource
    pub node_name: String,

    /// Kubernetes namespace containing the StellarNode
    ///
    /// Env: OPERATOR_NAMESPACE
    ///
    /// Example: --namespace stellar-system
    #[arg(short, long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    pub namespace: String,

    /// Output file path (optional, defaults to stdout)
    ///
    /// Example: --output runbook.md
    #[arg(short, long)]
    pub output: Option<String>,
}

#[derive(clap::Subcommand, Debug)]
pub enum SimulatorCmd {
    /// Create cluster, install operator manifests, print health hints
    Up(SimulatorUpArgs),
}

#[derive(Parser, Debug)]
pub struct SimulatorCli {
    #[command(subcommand)]
    pub command: SimulatorCmd,
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
pub struct SimulatorUpArgs {
    /// Name of the kind cluster to create.
    ///
    /// Ignored when --use-k3s is set (k3s manages its own cluster name).
    ///
    /// Example: --cluster-name stellar-dev
    #[arg(long, default_value = "stellar-sim")]
    pub cluster_name: String,

    /// Kubernetes namespace for the operator and demo StellarNode resources.
    ///
    /// Example: --namespace stellar-dev
    #[arg(long, default_value = "stellar-system")]
    pub namespace: String,

    /// Use k3s instead of kind when both are available in PATH.
    ///
    /// k3s must already be running; the simulator will use the current kubeconfig context.
    ///
    /// Example: --use-k3s
    #[arg(long, default_value_t = false)]
    pub use_k3s: bool,
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
pub struct WebhookArgs {
    /// Address and port the webhook HTTPS server will listen on.
    ///
    /// Use 0.0.0.0 to listen on all interfaces, or a specific IP to restrict access.
    /// Env: WEBHOOK_BIND
    ///
    /// Example: --bind 0.0.0.0:8443
    #[arg(long, env = "WEBHOOK_BIND", default_value = "0.0.0.0:8443")]
    pub bind: String,

    /// Path to the PEM-encoded TLS certificate file served by the webhook.
    ///
    /// Must be signed by the CA configured in the ValidatingWebhookConfiguration.
    /// Env: WEBHOOK_CERT_PATH
    ///
    /// Example: --cert-path /etc/webhook/tls/tls.crt
    #[arg(long, env = "WEBHOOK_CERT_PATH")]
    pub cert_path: Option<String>,

    /// Path to the PEM-encoded TLS private key file for the webhook certificate.
    ///
    /// Must correspond to the certificate provided via --cert-path.
    /// Env: WEBHOOK_KEY_PATH
    ///
    /// Example: --key-path /etc/webhook/tls/tls.key
    #[arg(long, env = "WEBHOOK_KEY_PATH")]
    pub key_path: Option<String>,

    /// Minimum log level emitted by the webhook server.
    ///
    /// Accepted values: trace, debug, info, warn, error.
    /// Env: LOG_LEVEL
    ///
    /// Example: --log-level debug
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Log output format (json or pretty)
    #[arg(long, env = "LOG_FORMAT", value_enum, default_value = "json")]
    pub log_format: LogFormat,
}

/// Arguments for the `benchmark` subcommand.
#[derive(Parser, Debug)]
#[command(
    about = "Run the StellarBenchmark controller",
    long_about = "Starts the StellarBenchmark controller that watches StellarBenchmark resources\n\
        and reconciles them by spinning up ephemeral load-generator pods, collecting metrics,\n\
        and writing results to BenchmarkReport resources or ConfigMaps.\n\n\
        This controller can run standalone or be co-located with the main operator process.\n\n\
        EXAMPLES:\n  \
        stellar-operator benchmark\n  \
        stellar-operator benchmark --namespace stellar-system\n  \
        stellar-operator benchmark --log-level debug"
)]
pub struct BenchmarkArgs {
    /// Kubernetes namespace to watch for StellarBenchmark resources.
    ///
    /// When unset, the controller watches all namespaces.
    /// Env: OPERATOR_NAMESPACE
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    pub namespace: String,

    /// Minimum log level.
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,
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

    #[test]
    fn dump_config_with_namespace_is_valid() {
        let args = parse_run(&["--dump-config", "--namespace", "prod"]).unwrap();
        assert!(args.validate().is_ok());
        assert_eq!(args.namespace, "prod");
        assert!(args.dump_config);
    }

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

    #[test]
    fn doctor_subcommand_parses() {
        let parsed = Args::try_parse_from(["stellar-operator", "doctor"])
            .expect("doctor subcommand should parse");
        assert!(matches!(parsed.command, Commands::Doctor(_)));
    }

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

    #[test]
    fn offline_flag_is_false_by_default() {
        let parsed = Args::try_parse_from(["stellar-operator", "version"]).unwrap();
        assert!(!parsed.offline);
    }

    #[test]
    fn offline_flag_can_be_set() {
        let parsed = Args::try_parse_from(["stellar-operator", "--offline", "version"]).unwrap();
        assert!(parsed.offline);
    }

    #[test]
    fn offline_flag_is_global_on_subcommand() {
        let parsed = Args::try_parse_from(["stellar-operator", "check-crd", "--offline"]).unwrap();
        assert!(parsed.offline);
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests_ext;