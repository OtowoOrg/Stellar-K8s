# Development Guide

This guide walks you through setting up a local development environment for Stellar-K8s, building the project, running tests, and contributing code.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Initial Setup](#initial-setup)
- [Building the Project](#building-the-project)
- [Running Tests](#running-tests)
- [Running the Operator Locally](#running-the-operator-locally)
- [Running E2E Tests](#running-e2e-tests)
- [Useful Make Targets](#useful-make-targets)
- [Development Workflow](#development-workflow)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

Before you begin, ensure you have the following tools installed:

### Required Tools

1. **Rust** (1.75+ required, 1.88+ recommended)
   ```bash
   # Install via rustup (recommended)
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   
   # Verify installation
   rustc --version
   cargo --version
   ```

2. **Docker** (for building container images)
   ```bash
   # Install Docker Engine
   # See: https://docs.docker.com/engine/install/
   
   # Verify installation
   docker --version
   docker ps  # Should not error
   ```

3. **kind** (Kubernetes in Docker - for local testing)
   ```bash
   # Linux/macOS
   curl -Lo ./kind https://kind.sigs.k8s.io/dl/v0.20.0/kind-linux-amd64
   chmod +x ./kind
   sudo mv ./kind /usr/local/bin/kind
   
   # Verify installation
   kind version
   ```

4. **kubectl** (Kubernetes CLI)
   ```bash
   # Linux
   curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
   chmod +x kubectl
   sudo mv kubectl /usr/local/bin/
   
   # Verify installation
   kubectl version --client
   ```

5. **Helm** (Kubernetes package manager)
   ```bash
   # Install Helm 3
   curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
   
   # Verify installation
   helm version
   ```

### Optional Tools

- **cargo-watch**: Auto-rebuild on file changes
  ```bash
  cargo install cargo-watch
  ```

- **k6**: For running performance benchmarks
  ```bash
  # See: https://k6.io/docs/get-started/installation/
  ```

---

## Initial Setup

### 1. Clone the Repository

```bash
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
```

### 2. Run Development Setup

This installs required Rust components and tools:

```bash
make dev-setup
```

This command:
- Updates Rust to the latest stable version
- Installs `clippy` (linter) and `rustfmt` (formatter)
- Installs `cargo-audit` (security scanner)
- Installs `cargo-watch` (file watcher for hot reload)

### 3. Verify Setup

Run a quick check to ensure everything is configured correctly:

```bash
make quick
```

This performs:
- Format check (`cargo fmt --all --check`)
- Compile check (`cargo check --workspace`)

---

## Building the Project

### Build All Binaries

The project produces two binaries:

1. **stellar-operator**: The main Kubernetes operator
2. **kubectl-stellar**: A kubectl plugin for managing StellarNode resources

```bash
# Build both binaries in release mode
make build

# Or use cargo directly
cargo build --release --locked
```

Binaries will be located at:
- `target/release/stellar-operator`
- `target/release/kubectl-stellar`

### Build for Development (Debug Mode)

```bash
# Faster compilation, includes debug symbols
cargo build

# Binaries at: target/debug/stellar-operator
```

### Build Docker Image

```bash
# Build local Docker image
make docker-build

# Or specify custom tag
docker build -t stellar-operator:dev .
```

The Dockerfile uses a multi-stage build:
- **Stage 1-2**: Dependency caching with cargo-chef
- **Stage 3**: Build both binaries
- **Stage 4**: Minimal distroless runtime (~15-20MB)

---

## Running Tests

### Unit Tests

Run all unit tests across the workspace:

```bash
make test

# Or use cargo directly
cargo test --workspace --all-features --verbose
```

This runs **62+ tests** including:
- 52 `StellarNodeSpec` validation tests (CRD schema validation)
- 5 kubectl plugin tests (output formatting)
- Controller reconciliation logic tests
- Webhook validation tests
- Backup scheduler tests

### Run Specific Test

```bash
# Run tests matching a pattern
cargo test <test_name>

# Example: Run only CRD tests
cargo test --package stellar-k8s --lib crd::tests

# Run with output visible
cargo test -- --nocapture
```

### Documentation Tests

Run code examples in documentation:

```bash
cargo test --doc --workspace
```

### Watch Mode (Auto-run Tests)

```bash
# Re-run tests on file changes
cargo watch -x test
```

---

## Running the Operator Locally

### Option 1: Against a kind Cluster (Recommended)

This is the most realistic development environment.

#### Step 1: Create a kind Cluster

```bash
# Create a new cluster
kind create cluster --name stellar-dev

# Verify cluster is running
kubectl cluster-info --context kind-stellar-dev
```

#### Step 2: Install CRDs

```bash
make install-crd

# Or manually
kubectl apply -f config/crd/stellarnode-crd.yaml
```

#### Step 3: Build and Load Operator Image

```bash
# Build Docker image
docker build -t stellar-operator:dev .

# Load image into kind cluster
kind load docker-image stellar-operator:dev --name stellar-dev
```

#### Step 4: Deploy the Operator

```bash
# Create operator namespace
kubectl create namespace stellar-system

# Apply operator manifests (from tests/e2e_kind.rs or create your own)
# You can use the Helm chart or create a simple deployment:

kubectl apply -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: stellar-operator
  namespace: stellar-system
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
        image: stellar-operator:dev
        imagePullPolicy: IfNotPresent
        env:
        - name: RUST_LOG
          value: "info"
EOF
```

Note: You'll also need to create RBAC resources (ServiceAccount, ClusterRole, ClusterRoleBinding). See `tests/e2e_kind.rs` for a complete example.

#### Step 5: Apply Sample Resources

```bash
# Apply a test StellarNode
kubectl apply -f config/samples/test-stellarnode.yaml

# Watch operator logs
kubectl logs -f -n stellar-system deployment/stellar-operator
```

### Option 2: Run Locally (Out-of-Cluster)

Run the operator binary directly on your machine, connecting to a Kubernetes cluster:

```bash
# Ensure KUBECONFIG is set
export KUBECONFIG=~/.kube/config

# Build and run
make run

# Or with debug logging
RUST_LOG=debug cargo run --bin stellar-operator
```

### Option 3: Development Mode with Hot Reload

Automatically rebuild and restart on code changes:

```bash
make run-dev

# Or use cargo-watch directly
RUST_LOG=debug cargo watch -x run
```

---

## Running E2E Tests

End-to-end tests validate the full operator lifecycle against a real Kubernetes cluster.

### Prerequisites

- Docker running
- kind installed
- kubectl installed

### Run E2E Tests

```bash
# Run the full E2E test suite
cargo test --test e2e_kind -- --ignored

# Run specific E2E test
cargo test --test e2e_kind e2e_stellarnode_reconciliation -- --ignored --nocapture
```

### E2E Test Environment Variables

Control test behavior with environment variables:

```bash
# Use custom cluster name
export KIND_CLUSTER_NAME=my-test-cluster

# Use existing operator image (skip build)
export E2E_OPERATOR_IMAGE=stellar-operator:latest
export E2E_BUILD_IMAGE=false
export E2E_LOAD_IMAGE=false

# Run tests
cargo test --test e2e_kind -- --ignored
```

### What E2E Tests Validate

1. **Cluster Setup**: Creates/reuses kind cluster
2. **CRD Installation**: Applies StellarNode CRD
3. **Operator Deployment**: Builds, loads, and deploys operator
4. **Resource Creation**: Creates StellarNode resources
5. **Reconciliation**: Verifies Deployment, Service, ConfigMap, PVC creation
6. **Status Updates**: Checks `status.phase` transitions to `Running`
7. **Updates**: Tests version upgrades and replica scaling
8. **Cleanup**: Verifies finalizers properly clean up resources

---

## Useful Make Targets

The Makefile provides convenient shortcuts for common tasks:

```bash
make help          # Show all available targets
```

### Development Commands

```bash
make dev-setup     # One-time setup: install Rust components and tools
make fmt           # Auto-format all code
make fmt-check     # Check if code is formatted (CI uses this)
make lint          # Run clippy linter
make audit         # Run security audit on dependencies
make test          # Run all tests
make build         # Build release binaries
make clean         # Remove build artifacts
```

### Quick Checks

```bash
make quick         # Fast pre-commit check (format + compile)
make ci-local      # Full CI pipeline locally (format + lint + audit + test + build)
```

### Kubernetes Operations

```bash
make install-crd   # Install CRDs to current cluster
make apply-samples # Apply sample StellarNode resources
```

### Running the Operator

```bash
make run           # Build and run operator (release mode)
make run-dev       # Run with hot reload (debug mode)
make watch         # Watch mode: rebuild on changes
```

### Docker

```bash
make docker-build      # Build Docker image (local arch)
make docker-multiarch  # Build multi-arch image (amd64 + arm64)
```

### Performance

```bash
make benchmark     # Run k6 performance benchmarks
```

### Complete Pipeline

```bash
make all           # Run full CI + Docker build
```

---

## Development Workflow

### Recommended Workflow for Contributors

1. **Create a feature branch**
   ```bash
   git checkout -b feature/my-feature
   ```

2. **Make changes and test frequently**
   ```bash
   # Run in watch mode for instant feedback
   cargo watch -x check -x test
   ```

3. **Before committing, run quick checks**
   ```bash
   make quick
   ```

4. **Format and fix lints**
   ```bash
   make fmt
   cargo clippy --fix --workspace --all-targets --all-features
   ```

5. **Run full CI validation**
   ```bash
   make ci-local
   ```

6. **Commit and push**
   ```bash
   git add .
   git commit -m "feat: add my feature"
   git push origin feature/my-feature
   ```

7. **Create Pull Request**
   - Ensure all CI checks pass (GitHub Actions)
   - Address review feedback
   - Squash commits if requested

### CI Pipeline Overview

GitHub Actions runs these checks on every PR:

1. **Security Audit**: `cargo audit --deny unsound`
2. **Format Check**: `cargo fmt --all --check`
3. **Lint**: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
4. **Tests**: `cargo test --workspace --all-features --verbose`
5. **Build**: `cargo build --release --locked`
6. **Docker Build**: Multi-arch image build
7. **Security Scan**: Trivy container scan

See [.github/CI_COMMANDS.md](.github/CI_COMMANDS.md) for exact commands.

---

## Troubleshooting

### Build Failures

**Problem**: Compilation errors or dependency issues

```bash
# Clean build cache and rebuild
cargo clean
make build

# Update dependencies
cargo update

# Check dependency tree
cargo tree
```

### Test Failures

**Problem**: Tests fail locally

```bash
# Run tests with detailed output
cargo test --workspace --verbose -- --nocapture

# Run specific failing test
cargo test <test_name> -- --nocapture

# Check for resource conflicts (e.g., port already in use)
lsof -i :8080
```

### Format Check Fails

**Problem**: `make ci-local` fails on format check

```bash
# Auto-fix formatting
make fmt

# Or manually
cargo fmt --all
```

### Clippy Warnings

**Problem**: Clippy reports warnings

```bash
# See detailed warnings
cargo clippy --workspace --all-targets --all-features

# Auto-fix some issues
cargo clippy --fix --workspace --all-targets --all-features

# Allow specific warnings (use sparingly)
#[allow(clippy::warning_name)]
```

### Security Audit Failures

**Problem**: `cargo audit` reports vulnerabilities

```bash
# View detailed advisory
cargo audit

# Find which crate depends on vulnerable dependency
cargo tree -i <vulnerable-crate>

# Update dependencies
cargo update <crate-name>
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for more details on handling RUSTSEC advisories.

### E2E Test Failures

**Problem**: E2E tests timeout or fail

```bash
# Check if kind cluster is running
kind get clusters

# Check if Docker is running
docker ps

# View kind cluster logs
kind export logs --name stellar-dev

# Manually inspect cluster
export KUBECONFIG="$(kind get kubeconfig --name stellar-dev)"
kubectl get all -A

# Clean up and retry
kind delete cluster --name stellar-dev
cargo test --test e2e_kind -- --ignored
```

### Operator Not Starting in kind

**Problem**: Operator pod crashes or won't start

```bash
# Check pod status
kubectl get pods -n stellar-system

# View logs
kubectl logs -n stellar-system deployment/stellar-operator

# Describe pod for events
kubectl describe pod -n stellar-system <pod-name>

# Common issues:
# - Image not loaded: kind load docker-image stellar-operator:dev --name stellar-dev
# - RBAC issues: Verify ServiceAccount, ClusterRole, ClusterRoleBinding
# - CRD not installed: kubectl apply -f config/crd/stellarnode-crd.yaml
```

### kubectl-stellar Plugin Not Working

**Problem**: Plugin not found or not executable

```bash
# Build plugin
cargo build --release --bin kubectl-stellar

# Install to PATH
cp target/release/kubectl-stellar ~/.local/bin/
# Or
sudo cp target/release/kubectl-stellar /usr/local/bin/

# Make executable
chmod +x ~/.local/bin/kubectl-stellar

# Verify
kubectl stellar --help
```

---

## Additional Resources

- [CONTRIBUTING.md](CONTRIBUTING.md) - Contribution guidelines and coding standards
- [README.md](README.md) - Project overview and quick start
- [.github/CI_COMMANDS.md](.github/CI_COMMANDS.md) - Exact CI commands reference
- [config/README.md](config/README.md) - Configuration files documentation
- [Makefile](Makefile) - All available make targets

### Documentation

- [docs/kubectl-plugin.md](docs/kubectl-plugin.md) - kubectl-stellar plugin guide
- [docs/health-checks.md](docs/health-checks.md) - Health check implementation
- [docs/peer-discovery.md](docs/peer-discovery.md) - Peer discovery guide
- [docs/wasm-webhook.md](docs/wasm-webhook.md) - Admission webhook with WASM

### Community

- GitHub Issues: https://github.com/OtowoOrg/Stellar-K8s/issues
- Pull Requests: https://github.com/OtowoOrg/Stellar-K8s/pulls

---

## Quick Reference

### Essential Commands

```bash
# Setup
make dev-setup                    # One-time setup
make quick                        # Fast pre-commit check
make ci-local                     # Full CI validation

# Development
cargo build                       # Build debug
cargo build --release             # Build release
cargo test                        # Run tests
cargo fmt                         # Format code
cargo clippy                      # Lint code

# Kubernetes
kind create cluster --name stellar-dev
kubectl apply -f config/crd/stellarnode-crd.yaml
kubectl apply -f config/samples/test-stellarnode.yaml
kubectl logs -f -n stellar-system deployment/stellar-operator

# E2E Tests
cargo test --test e2e_kind -- --ignored
```

### Environment Variables

```bash
RUST_LOG=debug                    # Enable debug logging
KUBECONFIG=~/.kube/config         # Kubernetes config path
KIND_CLUSTER_NAME=stellar-dev     # kind cluster name for E2E tests
E2E_OPERATOR_IMAGE=stellar-operator:dev  # Custom operator image for E2E
```

---

Happy coding! If you encounter issues not covered here, please open an issue or ask in the community channels.
