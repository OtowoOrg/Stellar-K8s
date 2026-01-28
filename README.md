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

## �️ Development

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

## �👨‍💻 Maintainer

**Otowo Samuel**  
_DevOps Engineer & Protocol Developer_

Bringing nearly 5 years of DevOps experience and a deep background in blockchain infrastructure tools (core contributor of `starknetnode-kit`). Passionate about building robust, type-safe tooling for the decentralized web.

---

## 📄 License

This project is licensed under the [Apache 2.0 License](LICENSE).
