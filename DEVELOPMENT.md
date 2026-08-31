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

> **Regenerating CRDs, Helm charts, or the OLM bundle?** See [docs/development/regeneration-guide.md](docs/development/regeneration-guide.md).

### Removed maintenance scripts

The following one-off scripts were removed as part of repository hygiene
(#1002, #1217). Use the supported replacements instead:

| Removed | Replacement |
|---------|-------------|
| `scripts/cleanup_root.sh` | `scripts/cleanup.sh` (`make cleanup`) |
| `scripts/organize_scripts.sh` | `scripts/cleanup.sh` (`make cleanup`) |
| `scripts/archive/*` | Removed; no archive tree — use `scripts/cleanup.sh` |
| `scripts/lib/batch.sh` | Removed with archive batch scripts |
| `scripts/cleanup_root.sh` | Manual cleanup; no automated replacement |
| `scripts/quickstart-verify.sh` | Golden-path quickstart verification |
| `scripts/dev-utils/*` | `make dev-setup`, `make preflight`, `make health-fast` |
| `benchmarks/test-webhook-local.sh` | `make benchmark-webhook` |
| `benchmarks/run-proximity-benchmark.sh` | `make benchmark` |
| `config/samples/benchmark-compare-example.sh` | `benchmarks/run-regression-test.sh` |
| `src/update_check.rs` | `src/version_check.rs` (used by the operator binary) |
| `src/kubectl_plugin/interactive.rs` | Standard kubectl-stellar subcommands |

#### Repository cleanup

Use the single supported cleanup tool:

```bash
make cleanup              # remove root scratch artifacts; guard obsolete paths
make cleanup DRY_RUN=1    # report only
# or
./scripts/cleanup.sh
./scripts/cleanup.sh --dry-run
```

---

## Prerequisites

You need: **Rust**, **Docker**, **kind**, **kubectl**, **Helm**, **gh**, **pre-commit**, **shellcheck**, and **k6**.

`make dev-setup` installs and configures the Rust toolchain, the Rust dev
tools (`cargo-audit`, `cargo-watch`), and the `pre-commit` git hooks for you
— see [Run Development Setup](#2-run-development-setup) below.

**Docker, kind, kubectl, Helm, gh, shellcheck, and k6 are not yet installed
automatically** (automating OS-level package-manager installs for these is
tracked separately) — install them manually for your OS before running
`make dev-setup`:

| Tool | Install docs |
|---|---|
| Docker | <https://docs.docker.com/engine/install/> |
| kind | <https://kind.sigs.k8s.io/docs/user/quick-start/#installation> |
| kubectl | <https://kubernetes.io/docs/tasks/tools/> |
| Helm 3 | <https://helm.sh/docs/intro/install/> |
| gh (GitHub CLI) | <https://cli.github.com/> |
| shellcheck | <https://github.com/koalaman/shellcheck#installing> |
| k6 | <https://k6.io/docs/get-started/installation/> |

Once those are installed, `make dev-setup` validates the whole environment
as its last step and tells you exactly what — if anything — is still
missing or out of date (see [Verify Setup](#3-verify-setup)).

---

## Initial Setup

### 1. Clone the Repository

```bash
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
```

### 2. Run Development Setup

Install the manually-installed tools listed in [Prerequisites](#prerequisites) above. On Linux or macOS you can automate this step with:

```bash
./scripts/setup-dev-env.sh
```

This detects your OS and runs the matching `scripts/setup-linux.sh` or `scripts/setup-mac.sh`. Then run:

```bash
make dev-setup
```

This command:
- Updates Rust to the latest stable version and installs `clippy` (linter) and `rustfmt` (formatter)
- Installs `cargo-audit` (security scanner) and `cargo-watch` (file watcher for hot reload)
- Installs the `pre-commit` git hooks
- Runs the cross-platform environment validator (`stellar-bootstrap-verify`) as its final step and prints a `[PASS]`/`[FAIL]` report

If that last step reports failures, see [Verify Setup](#3-verify-setup) and [Troubleshooting](#troubleshooting) below.

### 3. Verify Setup

`make dev-setup` already runs the validator as its last step. To re-check at any time — e.g. right after installing a tool it flagged as missing — without repeating the install steps:

```bash
# Cross-platform (Linux/macOS/Windows — no shell dependency, safe without WSL/Git Bash)
make dev-setup-verify

# Bash-only equivalent that also enforces the exact pinned minimum versions
# in scripts/lib/versions.sh (requires bash/WSL/Git Bash)
make preflight

# Then run the repository health check (recommended before opening a PR)
make health

# Or run a fast compile/format check only
make quick
```

`make dev-setup-verify` (`stellar-bootstrap-verify`) checks that `docker`, `kind`, `kubectl`, `helm`, `cargo`, and `gh` are all on your `PATH`, that `rustc` meets the minimum supported version, that you're running inside a git work tree, and whether the Docker daemon is reachable — printing one `[PASS]`/`[FAIL]` line per check plus an install hint for anything missing or outdated. Fix any reported gaps before proceeding.

`make health` runs format, lint, tests, API docs drift, markdown link checks, and shellcheck (when available) in one command and stops at the first failure with a clear summary.

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
```

This runs **1000+ tests** including:
This is the canonical command. It wraps `cargo test` with the project's
feature set (`rest-api`, `metrics`, `admission-webhook`, `k8s-v1-30`,
`reconciler-fuzz`) and `K8S_OPENAPI_ENABLED_VERSION=1.30`, matching CI
exactly. Plain `cargo test --all-features` will **not** produce the same
result.

Additional gates:

```bash
make test-db-migrations      # SQL forward/rollback harness (needs DATABASE_URL)
cargo test --test otel_propagation -- --nocapture
make yaml-schema-validate    # yamllint + CRD JSON schemas + Helm kubeconform
make helm-unittest
make helm-upgrade-test
```

See [docs/database/migrations.md](docs/database/migrations.md),
[docs/yaml-schema-validation.md](docs/yaml-schema-validation.md),
[docs/observability/tracing.md](docs/observability/tracing.md), and
[docs/helm-chart-testing.md](docs/helm-chart-testing.md).

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
make run-local

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

The Makefile provides convenient shortcuts for common tasks. See below for the **canonical command flow** — the recommended order for common development tasks.

```bash
make help          # Show all available targets and canonical flow
```

### Canonical Command Flow

This is the single recommended command sequence for day-to-day work. Prefer
these `make` targets over ad-hoc `cargo` invocations so local results match CI
feature flags. Full checklist and rationale:
[docs/development/repo-health-checklist.md](docs/development/repo-health-checklist.md).

```bash
make preflight     # Validate required tools are installed (run first after setup)
make dev-setup     # One-time environment setup (Rust toolchain, tools, pre-commit hooks)
make quick         # Fast pre-commit check (fmt-check + cargo check)
make health-fast   # Fast compile path: format + lint + compile check (no tests)
make health        # Full contributor health gate (format + lint + tests + docs)
make ci-local      # Full CI pipeline locally (fmt-check + lint + audit + test + build + link-check)
```

`make validate` is kept as a back-compat alias for `make health-fast`.

### Development Commands

```bash
make dev-setup     # One-time setup: install Rust components and tools
make fmt           # Auto-format all code
make fmt-check     # Check if code is formatted (CI uses this)
make lint          # Run clippy linter
make lint-strict   # Run clippy with complexity checks (stricter)
make audit         # Security audit on dependencies
make test          # Run all tests
make build         # Build release binaries
make clean         # Remove build artifacts
```

### Quick Checks

```bash
make preflight     # Validate all required tools are installed (run this first)
make health        # Recommended: format + lint + tests + docs (+ shellcheck)
make quick         # Fast pre-commit check (format + compile)
make health-fast  # Fast compile path: format + lint + compile check (no tests)
make ci-local      # Full CI pipeline locally (fmt-check + lint + audit + test + build + link-check)
```

### Security

```bash
make audit         # Run cargo-audit on dependencies
make security-scan # Run audit + shellcheck
make shellcheck    # Run shellcheck on all shell scripts
make security-all  # Run all security checks
```

### Kubernetes Operations

```bash
make install-crd   # Install CRDs to current cluster
make apply-samples # Apply sample StellarNode resources
make crd-gen       # Generate CRDs from Rust types
make regenerate    # Regenerate all derived artifacts (CRDs, API docs, OLM bundle)
```

### Running the Operator

```bash
make run-local     # Build and run operator from release binary
make run-dev       # Run with hot reload (debug mode)
make watch         # Watch mode: rebuild on changes
```

### Docker

```bash
make docker-build      # Build Docker image (local arch, fast mode using host binaries)
make docker-build-ci   # Build Docker image (CI mode, builds binaries in container)
make docker-multiarch  # Build multi-arch image (amd64 + arm64)
```

### Performance

```bash
make benchmark          # Run k6 performance benchmarks
make benchmark-all      # Run all benchmarks
make benchmark-webhook  # Run webhook benchmarks
```

### Complete Pipeline

```bash
make all           # Run CI checks + build + Docker image
make quickstart    # End-to-end local quickstart (kind cluster)
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

GitHub Actions runs these checks on every PR. Each one maps to a `make`
target so you can reproduce CI locally with the same feature flags and
environment variables:

1. **Security Audit**: `make audit`
2. **Format Check**: `make fmt-check`
3. **Lint**: `make lint`
4. **Tests**: `make test`
5. **Build**: `make build`
6. **Link Check**: `make link-check` (markdown), `make link-check-all` (repo-wide via lychee)
7. **Docker Build**: Multi-arch image build (`make docker-multiarch`)
8. **Security Scan**: Trivy container scan

Run the whole gate locally with `make ci-local`.

See [.github/CI_COMMANDS.md](.github/CI_COMMANDS.md) for the exact `cargo`
invocations each target wraps.

---

## Troubleshooting

### Missing or Outdated Tools

**Problem**: `make dev-setup` (or `make dev-setup-verify`) reports one or more `[FAIL]` lines, e.g.:

```text
=== Stellar-K8s Bootstrap Verification ===
  [FAIL] kind — not found in PATH — Install kind: https://kind.sigs.k8s.io/docs/user/quick-start/#installation
  [FAIL] rustc-version — rustc 1.80 is older than the minimum supported 1.92 — run `rustup update`
  [PASS] docker — Docker version 27.3.1, build ce12230
=== 5/8 checks passed, 2 critical failure(s) ===
```

Each failing line names the check and an install/fix hint:

- **A tool is `not found in PATH`** — install it using the link in the message (also listed in [Prerequisites](#prerequisites) above), then re-run `make dev-setup-verify`.
- **`rustc-version` is below the minimum** — run `rustup update stable` (or re-run `make dev-setup-rust`), then re-verify.
- **`docker-daemon` fails but `docker` itself passed** — Docker Desktop/daemon isn't running; start it. This check is a `Warning`, not `Critical`, so it won't block `make dev-setup` on its own.
- **`git-repository` fails** — you're not inside a git work tree (e.g. you downloaded a zip instead of `git clone`-ing); re-clone the repository.

Re-run `make dev-setup-verify` after each fix; it's safe to run repeatedly and only reports, it never modifies your system.

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
# Auto-fix formatting (canonical)
make fmt
```

### Clippy Warnings

**Problem**: Clippy reports warnings

```bash
# See detailed warnings (canonical — uses project features)
make lint

# Strict mode (adds complexity checks)
make lint-strict

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

- [docs/errors.md](docs/errors.md) - Error code reference (SK8S-001 through SK8S-022)
- [docs/kubectl-plugin.md](docs/kubectl-plugin.md) - kubectl-stellar plugin guide
- [docs/health-checks.md](docs/health-checks.md) - Health check implementation
- [docs/peer-discovery.md](docs/peer-discovery.md) - Peer discovery guide
- [docs/wasm-webhook.md](docs/wasm-webhook.md) - Admission webhook with WASM

### Community

- GitHub Issues: https://github.com/OtowoOrg/Stellar-K8s/issues
- Pull Requests: https://github.com/OtowoOrg/Stellar-K8s/pulls

---

## Quick Reference

Health and validation commands are listed once under
[Canonical Command Flow](#canonical-command-flow) and in the
[Canonical Repository Health Checklist](docs/development/repo-health-checklist.md).
Use `make help` for the full target list.

### Other useful commands
```bash
# Setup
make dev-setup                    # One-time setup
make preflight                    # Validate required tools are installed
make health                       # Common health gate (format, lint, test, docs)
make quick                        # Fast pre-commit check
make health-fast                 # Format + lint + compile check (no tests)
make ci-local                     # Full CI validation

```bash
# Development (canonical — prefer make targets to match CI feature flags)
make build                        # Build release (wraps `cargo build --release --locked`)
make test                         # Run tests (wraps `cargo test` with project features)
make fmt                          # Format code (wraps `cargo fmt --all`)
make lint                         # Lint code (wraps `cargo clippy` with project features)

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

## Repo Health Checklist

To maintain the quality, security, and cleanliness of the repository, all pull requests must satisfy the project's hygiene standards.

Before submitting or merging any changes, please review and verify all items in the [Canonical Repository Health Checklist](docs/development/repo-health-checklist.md).

You can run `make health` locally to execute format, lint, tests, and link checks in one command.

---

## Regenerating Manifests

Several files in this repo are generated from a source of truth. Always regenerate them after changing the source. See the [Regeneration Guide](docs/development/regeneration-guide.md) for detailed instructions.

### Policy on Compiled Binaries & WebAssembly Artifacts

To maintain a clean and lightweight repository, compiled binaries, WebAssembly modules (`*.wasm`), and auto-generated shell completion scripts must **never** be committed to the repository. These paths are explicitly ignored in `.gitignore`. 

If you modify source code that affects these outputs (such as CRDs, CLI definitions, or WebAssembly plugins):
1. **Source Code**: Commit only the source code changes (e.g., Rust files, build scripts, templates).
2. **Local Regeneration**: Build or regenerate the binaries locally during development and testing using the commands below.
3. **CI/CD Validation**: The CI/CD pipelines will automatically rebuild and validate these artifacts from source.

| Generated file | Source of truth | Regeneration command |
|---|---|---|
| `docs/api-reference.md` | CRD types in `src/crd/` | `make generate-api-docs` |
| `config/crd/*.yaml` | CRD structs in `src/crd/` | `make crd-gen` |
| `bundle/manifests/*.yaml` (gitignored — do not commit) | `config/manifests/bases/` + operator metadata | `make bundle` (requires operator-sdk) |
| `charts/stellar-operator/templates/*.yaml` | Hand-written (see [guide](docs/development/regeneration-guide.md)) | `helm template` for validation |
| Shell completions | CLI definitions in `src/cli.rs` | `make completions` |

For detailed instructions on each regeneration step, see the [Regeneration Guide](docs/development/regeneration-guide.md).

After running any of the above, commit the updated generated file alongside the source change in the same PR — except `bundle/manifests/*.yaml`, which is gitignored and must be regenerated locally on demand instead.

---

Happy coding! If you encounter issues not covered here, please open an issue or ask in the community channels.
