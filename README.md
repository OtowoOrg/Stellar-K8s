<p align="center">
  <img src="assets/logo.png" alt="Stellar-K8s Logo" width="200" />
</p>

# Stellar-K8s: Cloud-Native Stellar Infrastructure

![Rust](https://img.shields.io/badge/Built%20with-Rust-orange?style=for-the-badge&logo=rust) ![Kubernetes](https://img.shields.io/badge/Kubernetes-Operator-blue?style=for-the-badge&logo=kubernetes) ![License](https://img.shields.io/badge/License-Apache%202.0-green?style=for-the-badge) ![CI/CD](https://img.shields.io/github/actions/workflow/status/stellar/stellar-k8s/ci.yml?style=for-the-badge&label=Build)

> **Production-grade Stellar infrastructure in one command.**

**Stellar-K8s** is a high-performance Kubernetes Operator written in strict Rust using `kube-rs`. It automates the deployment, management, and scaling of **Stellar Core**, **Horizon**, and **Soroban RPC** nodes, bringing the power of Cloud-Native patterns to the Stellar ecosystem.

Designed for high availability, type safety, and minimal footprint.

---

## ✨ Key Features

- **🦀 Rust-Native Performance**: Built with `kube-rs` and `Tokio` for an ultra-lightweight footprint (~15MB binary) and complete memory safety.
- **🛡️ Enterprise Reliability**: Type-safe error handling prevents runtime failures. Built-in `Finalizers` ensure clean PVC and resource cleanup.
- **🏥 Auto-Sync Health Checks**: Automatically monitors Horizon and Soroban RPC nodes, only marking them Ready when fully synced with the network.
- **GitOps Ready**: Fully compatible with ArgoCD and Flux for declarative infrastructure management.
- **📈 Observable by Default**: Native Prometheus metrics integration for monitoring node health, ledger sync status, and resource usage.
- **⚡ Soroban Ready**: First-class support for Soroban RPC nodes with captive core configuration.

---

## 🏗️ Architecture Overview

Stellar-K8s follows the **Operator Pattern**, extending Kubernetes with a `StellarNode` Custom Resource Definition (CRD).

1.  **CRD Source of Truth**: You define your node requirements (Network, Type, Resources) in a `StellarNode` manifest.
2.  **Reconciliation Loop**: The Rust-based controller watches for changes and drives the cluster state to match your desired specification.
3.  **Stateful Management**: Automatically handles complex lifecycle events for Validators (StatefulSets) and RPC nodes (Deployments), including persistent storage and configuration.
4.  **Modular & Extensible**: The operator binary is structured into dedicated subcommand modules for improved maintainability and clear separation of concerns (CLI, logic, telemetry).

---

## 📋 Prerequisites

- **Kubernetes cluster** (1.28+)
- **kubectl** configured
- **Helm 3.x** (for operator installation)
- **Rust 1.88+** (for local development)
  - CI/CD and Docker builds use Rust 1.93 for consistency
  - Contributors can use any Rust 1.88+ version locally

---

## 🚀 Quick Start

Get a Testnet node running in under 5 minutes.

### Option 1: Docker Compose (No K8s Required)

Perfect for local development and testing without a full Kubernetes cluster:

```bash
# Start the development environment
make compose-up

# View logs
make compose-logs

# Stop the environment
make compose-down
```

See the [Docker Compose Quickstart Guide](docs/docker-compose-quickstart.md) for detailed instructions.

### Option 2: Kubernetes Cluster

### 1. Install the Operator via Helm

```bash
# Add the helm repo (example)
helm repo add stellar-k8s https://stellar.github.io/stellar-k8s
helm repo update

# Install the operator
helm install stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --create-namespace
```

### Install the Operator via OLM

If you are installing on a cluster with the Operator Lifecycle Manager (e.g. OpenShift), refer to the [OLM Deployment Guide](docs/deploy-olm.md).

### 2. Deploy a Testnet Validator

Apply the following manifest to your cluster:

```yaml
# validator.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-validator
  namespace: stellar
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  storage:
    storageClass: "standard"
    size: "100Gi"
    retentionPolicy: Retain
  validatorConfig:
    seedSecretRef: "my-validator-seed" # Pre-created K8s secret
    enableHistoryArchive: true
```

```bash
kubectl apply -f validator.yaml
kubectl get stellarnodes -n stellar
```

---

## 📚 Examples

Ready-to-use manifests for all supported node types are available in the [examples/](examples/) directory:

- [Validator (Mainnet)](examples/validator-mainnet.yaml) - High-performance validator with SCP quorum and history archives.
- [Validator (Testnet)](examples/validator-testnet.yaml) - Standard validator for network testing.
- [Horizon API](examples/horizon.yaml) - Scalable REST API server with Ingress and ingestion.
- [Soroban RPC](examples/soroban-rpc.yaml) - Smart contract execution node with autoscaling.
- [Disaster Recovery Setup](examples/dr-setup.yaml) - Multi-cluster HA configuration with automated drills.

---

### 3. Use the kubectl-stellar Plugin

The project includes a kubectl plugin for convenient interaction with StellarNode resources:

```bash
# Build the plugin
cargo build --release --bin kubectl-stellar
cp target/release/kubectl-stellar ~/.local/bin/kubectl-stellar

# List all StellarNode resources
kubectl stellar list

# Check sync status
kubectl stellar status

# View logs from a node
kubectl stellar logs my-validator -f
```

See [kubectl-plugin.md](docs/kubectl-plugin.md) for complete documentation.

### 4. Shell Completion

Generate shell completion scripts for the stellar-operator CLI to enable tab completion:

```bash
# Generate completions for all shells
make completions

# Or generate for a specific shell
cargo run --bin stellar-completions completions bash > stellar-operator.bash
cargo run --bin stellar-completions completions zsh > _stellar-operator
cargo run --bin stellar-completions completions fish > stellar-operator.fish
```

**Installation:**

- **Bash**: `source completions/stellar-operator.bash` or copy to `/etc/bash_completion.d/`
- **Zsh**: Copy `completions/_stellar-operator` to a directory in your `$fpath`
- **Fish**: Copy `completions/stellar-operator.fish` to `~/.config/fish/completions/`

After installation, you can use tab completion with the `stellar-operator` command:

```bash
stellar-operator <TAB>        # Shows available subcommands
stellar-operator run --<TAB>  # Shows available flags
```

### Architecture Decision Records (ADRs)

Major architectural decisions are documented in our [ADR directory](docs/adr/README.md), including:

- **Choice of Rust** - Rationale for selecting Rust as the programming language
- **kube-rs Finalizers** - Strategy for resource cleanup and lifecycle management
- **CRD Versioning** - Approach to API evolution and backward compatibility

### 4. Custom Validation Policies with WebAssembly

Stellar-K8s supports custom validation policies written in WebAssembly, allowing you to enforce organization-specific requirements without modifying the operator code.

```rust
// Example: Enforce approved image registries
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    let input = read_validation_input()?;

    // Check if image is from approved registry
    if !is_approved_registry(&input.object.spec.version) {
        return deny("Image must be from approved registry");
    }

    allow()
}
```

Features:

- **Sandboxed Execution**: Plugins run in a secure, isolated Wasm environment
- **Dynamic Loading**: Load plugins from ConfigMaps at runtime
- **Multi-Language Support**: Write policies in Rust, Go, C++, or any language that compiles to Wasm
- **Fail-Open Support**: Configure plugins to allow requests if they fail

See [wasm-webhook.md](docs/wasm-webhook.md) for complete documentation and examples.

---

## 📊 Monitoring & Observability

Stellar-K8s comes with built-in Prometheus metrics and a pre-configured Grafana dashboard that provides a comprehensive overview of both the operator's health and the managed Stellar nodes.

### Operator Build Info & Leader Metrics

The operator exposes the following production-readiness metrics:

| Metric                                  | Type    | Description                                                     |
| --------------------------------------- | ------- | --------------------------------------------------------------- |
| `stellar_operator_info`                 | Gauge   | Always `1`; carries `version`, `git_sha`, `rust_version` labels |
| `stellar_operator_leader_status`        | Gauge   | `1` if this instance is the current leader, `0` otherwise       |
| `stellar_operator_uptime_seconds_total` | Counter | Total uptime of the operator process in seconds                 |

### Importing the Grafana Dashboard

1. Open your Grafana instance.
2. Navigate to **Dashboards** -> **Import**.
3. Upload the `monitoring/grafana-dashboard.json` file provided in this repository.
4. Select your Prometheus data source when prompted.
5. The dashboard will now automatically visualize:
   - Node availability, sync status, and peer connectivity
   - Controller reconciliation rates and duration (p50, p95, p99)
   - Error rates and operator resource usage (CPU/Memory)
   - Operator version, leader status, and uptime (new panels)

---

## ⚙️ Runtime Feature Flags

The operator supports runtime feature flags via the `stellar-operator-config` ConfigMap. Changes are picked up **without restart**.

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: stellar-operator-config
  namespace: stellar-system
data:
  enable_cve_scanning: "true"
  enable_read_pool: "false"
  enable_dr: "false"
  enable_peer_discovery: "true"
  enable_archive_health: "true"
  enable_soroban_metrics: "true"
```

| Flag                     | Default | Description                         |
| ------------------------ | ------- | ----------------------------------- |
| `enable_cve_scanning`    | `true`  | Automatic CVE patch reconciliation  |
| `enable_read_pool`       | `false` | Read-replica pool management        |
| `enable_dr`              | `false` | Disaster-recovery drill scheduling  |
| `enable_peer_discovery`  | `true`  | Automatic peer discovery            |
| `enable_archive_health`  | `true`  | History archive health checks       |
| `enable_soroban_metrics` | `true`  | Soroban-specific Prometheus metrics |

When using the Helm chart, set flags via `values.yaml`:

```yaml
featureFlags:
  enableCveScanning: "true"
  enableReadPool: "false"
```

---

## 🤝 Contributing

We welcome contributions! This project uses pre-commit hooks to ensure code quality. Please see our [Contributing Guide](CONTRIBUTING.md) for details on our development process, coding standards, and how to submit pull requests.

### Quick Start for Contributors

```bash
# Setup development environment (includes pre-commit hooks)
make dev-setup

# Run pre-commit hooks manually
make pre-commit
```

---

## Roadmap

### Phase 1: Core Operator & Helm Charts (Current)

- [x] `StellarNode` CRD with Validator support
- [x] Basic Controller logic with `kube-rs`
- [x] Helm Chart for easy deployment
- [x] CI/CD Pipeline with GitHub Actions and Docker builds
- [x] Auto-Sync Health Checks for Horizon and Soroban RPC nodes
- [x] kubectl-stellar plugin for node management

### Phase 2: Soroban & Observability (Month 2)

- [ ] Full Soroban RPC node support with captive core
- [ ] Comprehensive Prometheus metrics export (Ledger age, peer count)
- [ ] Dedicated Grafana Dashboards
- [ ] Automated history archive management

### Phase 3: High Availability & DR (Month 3)

- [ ] Automated failover for high-availability setups
- [ ] Disaster Recovery automation (backup/restore from history)
- [ ] Multi-region federation support

---

## 💾 High-Performance Local Storage (NVMe)

Standard cloud Persistent Volumes (like AWS EBS or GCP Persistent Disks) can sometimes bottleneck Stellar Core's highly demanding database I/O, leading to ledger sync lag. Stellar-K8s supports a specialized `LocalStorage` mode to take advantage of low-latency local NVMe drives directly attached to your Kubernetes nodes.

### Standard PVCs vs Local NVMe (Testnet Workload Benchmark)

| Storage Type         | Peak IOPS | Read Latency | Write Latency | Avg Sync Lag |
| -------------------- | --------- | ------------ | ------------- | ------------ |
| Cloud Standard (EBS) | ~3,000    | 1.5 - 2.5ms  | 2.0 - 5.0ms   | 5 - 15s      |
| Local NVMe           | 100,000+  | < 0.1ms      | < 0.1ms       | **< 1s**     |

### Enabling LocalStorage

Simply set `spec.storage.mode` to `Local`. Stellar-K8s will automatically attempt to use a provisioner like `local-path` (often bundled with K3s/Kind/EKS). You can also explicitly pin to a specific node using `nodeAffinity` or specify a dedicated `storageClass`.

```yaml
spec:
  nodeType: Validator
  storage:
    mode: Local
    # Automatically detects "local-path" or "local-storage" if omitted
    # Or explicitly pin to specific nodes:
    nodeAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        nodeSelectorTerms:
          - matchExpressions:
              - key: kubernetes.io/hostname
                operator: In
                values: ["my-nvme-node-1"]
```

---

## 📊 Soroban-Specific Observability

Stellar-K8s provides comprehensive monitoring for Soroban RPC nodes with specialized metrics for smart contract operations.

### Grafana Dashboard

A dedicated Soroban monitoring dashboard is available at `monitoring/grafana-soroban.json`. This dashboard provides real-time visibility into:

#### Smart Contract Metrics

- **Wasm Execution Time**: Histogram showing p50, p95, and p99 latencies for host function execution
- **Contract Storage Fees**: Distribution of storage fees charged across contract operations
- **Host Function Calls**: Breakdown of which host functions are being invoked most frequently

#### Resource Consumption

- **CPU per Invocation**: CPU instructions consumed by each contract invocation
- **Memory per Invocation**: Wasm VM memory usage and per-invocation memory consumption
- **Process Resources**: Overall CPU and memory usage of the Soroban RPC process

#### Transaction Metrics

- **Success/Failure Rate**: Real-time success and failure rates for Soroban transactions
- **Transaction Ingestion Rate**: Rate of transactions being processed (10m sliding window)
- **Events Ingestion Rate**: Rate of contract events being ingested

#### Performance Indicators

- **RPC Request Latency**: p50, p95, p99 latencies for JSON RPC methods
- **Database Round Trip Time**: Database query performance monitoring
- **Ledger Ingestion Lag**: How far behind the network the RPC node is

#### Runtime Health

- **Active Goroutines**: Number of concurrent goroutines in the Go runtime
- **Memory Allocations**: Rate of memory allocations
- **GC Pause Time**: Garbage collection pause duration

### Importing the Dashboard

1. **Access Grafana**: Navigate to your Grafana instance
2. **Import Dashboard**: Go to Dashboards → Import
3. **Upload JSON**: Upload `monitoring/grafana-soroban.json`
4. **Configure Datasource**: Select your Prometheus datasource
5. **Save**: The dashboard will be available as "Soroban RPC - Smart Contract Monitoring"

### Prometheus Metrics

The operator exports the following Soroban-specific metrics:

```
# Wasm execution metrics
soroban_rpc_wasm_execution_duration_microseconds{namespace, name, network, contract_id}

# Storage fee metrics
soroban_rpc_contract_storage_fee_stroops{namespace, name, network, contract_id}

# Resource consumption
soroban_rpc_wasm_vm_memory_bytes{namespace, name, network, contract_id}
soroban_rpc_contract_invocation_cpu_instructions{namespace, name, network, contract_id}
soroban_rpc_contract_invocation_memory_bytes{namespace, name, network, contract_id}

# Contract invocations
soroban_rpc_contract_invocations_total{namespace, name, network, contract_type}

# Transaction results
soroban_rpc_transaction_result_total{namespace, name, network, result}

# Host function calls
soroban_rpc_host_function_calls_total{namespace, name, network, contract_id}
```

### Example Queries

**Average Wasm execution time (last 5m)**:

```promql
rate(soroban_rpc_wasm_execution_duration_microseconds_sum[5m]) /
rate(soroban_rpc_wasm_execution_duration_microseconds_count[5m])
```

**Transaction success rate**:

```promql
sum(rate(soroban_rpc_transaction_result_total{result="success"}[5m])) /
sum(rate(soroban_rpc_transaction_result_total[5m]))
```

**Top 5 most invoked contracts**:

```promql
topk(5, sum(rate(soroban_rpc_contract_invocations_total[5m])) by (contract_type))
```

### Alerting Rules

Example Prometheus alerting rules for Soroban RPC:

```yaml
groups:
  - name: soroban_rpc
    rules:
      - alert: HighWasmExecutionLatency
        expr: histogram_quantile(0.99, rate(soroban_rpc_wasm_execution_duration_microseconds_bucket[5m])) > 100000
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High Wasm execution latency (p99 > 100ms)"

      - alert: HighTransactionFailureRate
        expr: |
          sum(rate(soroban_rpc_transaction_result_total{result="failed"}[5m])) /
          sum(rate(soroban_rpc_transaction_result_total[5m])) > 0.1
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Transaction failure rate above 10%"

      - alert: HighLedgerIngestionLag
        expr: soroban_rpc_ingest_ledger_lag > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Ledger ingestion lagging behind network"
```

For more details on Soroban metrics, see the [Stellar Soroban RPC documentation](https://developers.stellar.org/docs/data/apis/rpc/admin-guide/monitoring).

### High Availability & Pod Disruption Budgets

Stellar-K8s includes built-in PodDisruptionBudget (PDB) support to protect the operator and validator nodes during Kubernetes maintenance operations like node drains and cluster upgrades.

**Default Configuration:**

```yaml
podDisruptionBudget:
  enabled: true
  minAvailable: 1
```

**For Validator Nodes (Recommended):**

```yaml
podDisruptionBudget:
  enabled: true
  maxUnavailable: 1 # Allows one pod down during maintenance
```

For comprehensive guidance on PDB configuration, emergency maintenance procedures, and troubleshooting, see **[docs/pod-disruption-budget.md](docs/pod-disruption-budget.md)**.

### History Archive Management

Stellar-K8s includes a `prune-archive` utility for safely managing history archive storage costs:

```bash
# Dry-run mode (default - no deletions)
stellar-operator prune-archive \
  --archive-url s3://my-bucket/stellar-history \
  --retention-days 30

# Execute pruning with safety guarantees
stellar-operator prune-archive \
  --archive-url s3://my-bucket/stellar-history \
  --retention-days 30 \
  --force
```

**Safety Features:**

- ✅ Dry-run enabled by default
- ✅ Minimum checkpoint retention (50 checkpoints)
- ✅ Maximum age protection (7 days)
- ✅ Checkpoint validation before deletion
- ✅ Concurrent deletion with error handling

For comprehensive documentation, see **[docs/archive-pruning.md](docs/archive-pruning.md)**.

### Live State Diff

Debug operator reconciliation issues with the `diff` subcommand that shows differences between desired and actual cluster state:

```bash
# Show what differs from desired state
stellar-operator diff --name my-validator --namespace stellar

# JSON output for scripting
stellar-operator diff --name my-validator --namespace stellar --format json

# Show ConfigMap contents (stellar-core.cfg, etc.)
stellar-operator diff --name my-validator --namespace stellar --show-config
```

**Features:**

- ✅ Colored terminal output with change indicators
- ✅ Multiple output formats (terminal, JSON, unified)
- ✅ Compares all operator-managed resources
- ✅ ConfigMap content inspection
- ✅ Change detection for labels, annotations, specs

For comprehensive documentation, see **[docs/diff-utility.md](docs/diff-utility.md)**.

---

## 📖 API Reference

The full `StellarNode` CRD field reference — including all fields, types, defaults, validation constraints, and example manifests — is available at:

**[docs/api-reference.md](docs/api-reference.md)**

The reference is auto-generated from the CRD OpenAPI schema. To regenerate after modifying the CRD types:

```bash
make generate-api-docs
```

---

## Development

### Prerequisites

- Rust (latest stable)
- Docker & Kubernetes cluster
- Make

### Quick Start

```bash
# Setup development environment
make dev-setup

# Standard Development Targets
make build         # Build release binary
make test          # Run all tests
make lint          # Run clippy
make fmt           # Format code
make docker-build  # Build Docker image
make helm-lint     # Run Helm chart linting
make crd-gen       # Generate CRDs
make run-local     # Run operator locally in dev mode
make clean         # Clean build artifacts

# Full CI validation
make ci-local
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed development guidelines.

### Reconciler fuzzing

To ensure the operator never panics under malformed or extreme inputs, the reconciler is fuzzed with random `StellarNodeSpec` mutations and event sequences (proptest). Run the fuzzer locally:

```bash
cargo test -p stellar-k8s --features reconciler-fuzz --test reconciler_fuzz
```

See [docs/fuzzing.md](docs/fuzzing.md) for full instructions (more cases, env vars, optional reconcile test with cluster).

---

## 👨‍💻 Maintainer

**Otowo Samuel**
_DevOps Engineer & Protocol Developer_

Bringing nearly 5 years of DevOps experience and a deep background in blockchain infrastructure tools (core contributor of `starknetnode-kit`). Passionate about building robust, type-safe tooling for the decentralized web.

---

## 📄 License

This project is licensed under the [Apache 2.0 License](LICENSE).

---

## 📝 Changelog

See [CHANGELOG.md](CHANGELOG.md) for a detailed history of changes and releases.
