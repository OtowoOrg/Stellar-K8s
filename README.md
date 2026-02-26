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

## 🤝 Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details on our development process, coding standards, and how to submit pull requests.

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

# Quick pre-commit check
make quick

# Full CI validation
make ci-local

# Build and run
make build
make run
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed development guidelines.

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
