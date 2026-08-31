# Developer Setup Prerequisites

This guide covers all prerequisites, environment setup, and troubleshooting for developing Stellar-K8s.

## Table of Contents

- [System Requirements](#system-requirements)
- [Prerequisites](#prerequisites)
- [Installation by OS](#installation-by-os)
- [Verification](#verification)
- [Troubleshooting](#troubleshooting)

## System Requirements

### Minimum Hardware

- **CPU:** 4 vCPU (8+ recommended for kind clusters)
- **RAM:** 8GB (16GB+ recommended)
- **Disk:** 20GB free space (30GB+ recommended)
- **Network:** Stable internet connection

### Supported Operating Systems

- **macOS:** 11+ (Intel or Apple Silicon)
- **Linux:** Ubuntu 20.04+, Debian 11+, Fedora 35+
- **Windows:** Windows 10/11 with WSL2

## Prerequisites

The following tools are **required**:

| Tool | Min Version | Purpose |
|------|-------------|---------|
| Rust | 1.92 | Language toolchain |
| Cargo | 1.92 | Package manager |
| Docker | 20.0.0 | Container runtime |
| kubectl | 1.30.0 | Kubernetes CLI |
| kind | 0.24.0 | Local K8s clusters |
| Helm | 3.16.0 | Package manager for K8s |

### Optional but Recommended

| Tool | Purpose |
|------|---------|
| Git | Version control |
| pre-commit | Automated code checks |
| shellcheck | Shell script linting |
| k6 | Load testing |
| kube-score | Kubernetes manifest scoring |
| kubeconform | Kubernetes manifest validation |

## Installation by OS

### macOS

```bash
# Clone the repository
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s

# Run the setup script (idempotent)
bash scripts/setup-mac.sh
```

**What it installs:**
- Rust via Homebrew
- Docker Desktop (if missing)
- kubectl, kind, Helm via Homebrew
- Additional development tools
- Pre-commit hooks
- Shell completion

**Verification:**
```bash
make preflight  # Verify all required tools
```

### Linux (Ubuntu/Debian)

```bash
# Clone the repository
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s

# Run the setup script (idempotent)
bash scripts/setup-linux.sh
```

**What it installs:**
- Rust via rustup
- Docker (via package manager)
- kubectl, kind, Helm via curl downloads (or package manager)
- Build dependencies (C compiler, OpenSSL, etc.)
- Additional development tools
- Pre-commit hooks

**Note:** May require `sudo` for package installation.

### Linux (Fedora)

```bash
# Clone the repository
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s

# Run the setup script (idempotent)
bash scripts/setup-linux.sh
```

**Note:** Fedora variant of the Linux script uses `dnf` instead of `apt`.

### Windows (WSL2)

1. **Install WSL2:**
   ```powershell
   wsl --install
   wsl --set-default-version 2
   ```

2. **Inside WSL2 Ubuntu:**
   ```bash
   git clone https://github.com/OtowoOrg/Stellar-K8s.git
   cd Stellar-K8s
   bash scripts/setup-linux.sh
   ```

3. **Docker Desktop for Windows:**
   - Install [Docker Desktop for Windows](https://docs.docker.com/desktop/install/windows-install/)
   - Enable WSL2 integration in Docker Desktop settings
   - In WSL2 terminal: `docker run hello-world` (verify connectivity)

4. **Configure kubectl for Docker Desktop:**
   ```bash
   kubectl config use-context docker-desktop
   ```

## Verification

### Quick Check (1 minute)

```bash
# Verify all required tools are installed and at minimum versions
make preflight
```

### Detailed Health Check (2 minutes)

```bash
# Full environment health report with component status
make health-check
```

**Output example:**
```
╔════════════════════════════════════════════════════════════════╗
║           ENVIRONMENT HEALTH CHECK REPORT                      ║
╚════════════════════════════════════════════════════════════════╝

Required Tools (Strict):
  [✓] Rust: 1.92.0
  [✓] kubectl: 1.30.1
  [✓] kind: 0.24.0
  [✓] Helm: 3.16.0
  [✓] Docker: 24.0.0

Rust Components:
  [✓] rustfmt: installed
  [✓] clippy: installed

Optional Tools (Recommended):
  [✓] pre-commit: 3.5.0
  [!] shellcheck: not installed
  [✓] k6: 0.47.0

System Resources:
  [✓] Disk space: 45GB available

Git Configuration:
  [✓] Git config: ready
```

### Build Verification

```bash
# Test compilation
make quick

# Output: ✓ Quick checks passed
```

### Full CI Locally

```bash
# Run the complete CI pipeline locally
make ci-local
```

## Troubleshooting

### Common Issues

#### 1. Rust Installation Failed

**Symptom:** `cargo: command not found`

**Solution:**
```bash
# Reinstall Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add to PATH (if needed)
source $HOME/.cargo/env
```

#### 2. Docker Daemon Not Running

**Symptom:** `Cannot connect to Docker daemon`

**Solution:**
```bash
# macOS:
open /Applications/Docker.app

# Linux:
sudo systemctl start docker
sudo systemctl enable docker

# Verify:
docker ps
```

#### 3. kubectl Context Issues

**Symptom:** `The connection to the server was refused`

**Solution:**
```bash
# List available contexts
kubectl config get-contexts

# Switch to appropriate context
kubectl config use-context docker-desktop  # or kind-stellar-dev

# Verify connectivity
kubectl cluster-info
```

#### 4. kind Cluster Creation Fails

**Symptom:** `Error creating cluster`

**Solution:**
```bash
# Check Docker is running
docker ps

# Delete old cluster if exists
kind delete cluster --name stellar-dev

# Create with verbose output
kind create cluster --name stellar-dev --wait 120s -v 4

# Check cluster nodes
kubectl get nodes
```

#### 5. Out of Memory / Disk Space

**Symptom:** Build fails or Docker operations slow

**Solution:**
```bash
# Check available disk
df -h

# Check Docker disk usage
docker system df

# Clean up Docker images/containers
docker system prune -a --volumes

# For macOS Docker Desktop, increase resource limits:
# Preferences → Resources → Increase memory/disk
```

#### 6. Pre-commit Hook Failures

**Symptom:** `pre-commit hook failed` during git commit

**Solution:**
```bash
# Reinstall pre-commit hooks
make dev-setup-hooks

# Or manually:
pre-commit install
pre-commit install --hook-type pre-push

# Run pre-commit checks manually
pre-commit run --all-files
```

### Detailed Diagnostics

#### Health Check with JSON Output

```bash
# Machine-readable format
make health-check --json | jq '.components'
```

#### Auto-fix (where possible)

```bash
# Attempt to automatically install missing components
make health-check --fix
```

#### Verify Git Configuration

```bash
# Check git user configuration
git config --global --list | grep user

# Configure if needed
git config --global user.name "Your Name"
git config --global user.email "your.email@example.com"
```

## Development Workflow

### After Initial Setup

```bash
# 1. Verify preflight checks pass
make preflight

# 2. Run full health check
make health-check

# 3. Run quick pre-commit checks
make quick

# 4. Run full CI suite locally (optional, takes ~10 minutes)
make ci-local
```

### Daily Development

```bash
# Format code before committing
make fmt

# Quick lint check
make quick

# Run tests
make test

# Build locally
make build
```

### Pre-Pull Request

```bash
# Run full health check
make health

# Or specifically:
make fmt-check lint test
```

## Environment Variables

### Optional Configuration

```bash
# Rust logging
export RUST_LOG=debug

# Enable strict mode for clippy
export CLIPPY_STRICT=1

# Disable interactive prompts in CI
export CI=true
```

### Kubernetes Configuration

```bash
# Set default namespace
kubectl config set-context --current --namespace=stellar-system

# Set resource quota
export KUBE_QUOTA=10Gi
```

## Getting Help

1. **Check existing issues:** https://github.com/OtowoOrg/Stellar-K8s/issues
2. **Run troubleshooting guide:** `make health-check`
3. **Review logs:** `make health-check --debug`
4. **Ask in discussions:** https://github.com/OtowoOrg/Stellar-K8s/discussions

## See Also

- [DEVELOPMENT.md](../DEVELOPMENT.md) — Development workflows
- [CONTRIBUTING.md](../../CONTRIBUTING.md) — Contribution guidelines
- [docs/ci-failure-diagnostics.md](../ci-failure-diagnostics.md) — CI debugging
