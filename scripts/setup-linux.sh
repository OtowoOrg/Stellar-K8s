#!/bin/bash
# Repeatable Linux dev environment bootstrap for Stellar-K8s.
# Tested on Ubuntu 22.04+ and Debian 12+. Safe to run multiple times.
set -euo pipefail

# ── Pinned versions ───────────────────────────────────────────────────────────
RUST_TOOLCHAIN="1.92"          # keep in sync with ci.yml toolchain
KIND_VERSION="0.24.0"
KUBECTL_VERSION="1.30.0"
HELM_VERSION="3.16.0"
K6_VERSION="0.54.0"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
ok()   { echo -e "${GREEN}✓ $*${NC}"; }
warn() { echo -e "${YELLOW}⚠ $*${NC}"; }
fail() { echo -e "${RED}✗ $*${NC}"; exit 1; }
step() { echo -e "\n${YELLOW}→ $*${NC}"; }

echo -e "${GREEN}=== Stellar-K8s Linux Dev Setup ===${NC}"
echo    "    Rust ${RUST_TOOLCHAIN} | kind ${KIND_VERSION} | kubectl ${KUBECTL_VERSION} | helm ${HELM_VERSION}"

[[ "$OSTYPE" == "linux-gnu"* ]] || fail "This script is for Linux only."

# ── Prerequisites ─────────────────────────────────────────────────────────────
step "System prerequisites"
if command -v apt-get &>/dev/null; then
    sudo apt-get update -qq
    sudo apt-get install -y --no-install-recommends \
        curl ca-certificates git build-essential pkg-config \
        libssl-dev python3-pip shellcheck
    ok "apt prerequisites installed"
elif command -v dnf &>/dev/null; then
    sudo dnf install -y curl ca-certificates git gcc openssl-devel pkg-config python3-pip ShellCheck
    ok "dnf prerequisites installed"
else
    warn "Unknown package manager — ensure curl, git, gcc, openssl-dev and python3-pip are installed."
fi

# ── Rust / rustup ─────────────────────────────────────────────────────────────
step "Rust toolchain (${RUST_TOOLCHAIN})"
if ! command -v rustup &>/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
        -y --default-toolchain "${RUST_TOOLCHAIN}" --profile minimal
    # shellcheck source=/dev/null
    source "${HOME}/.cargo/env"
else
    rustup toolchain install "${RUST_TOOLCHAIN}" --profile minimal 2>/dev/null || true
    rustup default "${RUST_TOOLCHAIN}"
    ok "Rust $(rustc --version)"
fi
rustup component add rustfmt clippy 2>/dev/null || true
ok "rustfmt + clippy"

# ── kind ──────────────────────────────────────────────────────────────────────
step "kind ${KIND_VERSION}"
if command -v kind &>/dev/null && kind --version 2>/dev/null | grep -q "${KIND_VERSION}"; then
    ok "kind ${KIND_VERSION} already installed"
else
    ARCH="$(uname -m)"
    case "${ARCH}" in
        x86_64)  KIND_ARCH="amd64" ;;
        aarch64) KIND_ARCH="arm64" ;;
        *)        fail "Unsupported architecture: ${ARCH}" ;;
    esac
    curl -fsSL -o /tmp/kind \
        "https://kind.sigs.k8s.io/dl/v${KIND_VERSION}/kind-linux-${KIND_ARCH}"
    chmod +x /tmp/kind
    sudo mv /tmp/kind /usr/local/bin/kind
    ok "kind ${KIND_VERSION} installed"
fi

# ── kubectl ───────────────────────────────────────────────────────────────────
step "kubectl ${KUBECTL_VERSION}"
if command -v kubectl &>/dev/null; then
    GOT=$(kubectl version --client -o json 2>/dev/null | \
        python3 -c "import sys,json; print(json.load(sys.stdin)['clientVersion']['gitVersion'].lstrip('v'))" \
        2>/dev/null || echo "")
    if [[ "${GOT}" == "${KUBECTL_VERSION}"* ]]; then
        ok "kubectl ${KUBECTL_VERSION} already installed"
    else
        warn "kubectl ${GOT} installed; pinned ${KUBECTL_VERSION} — replacing."
        REINSTALL_KUBECTL=1
    fi
else
    REINSTALL_KUBECTL=1
fi
if [[ "${REINSTALL_KUBECTL:-0}" == "1" ]]; then
    ARCH="$(uname -m)"
    case "${ARCH}" in
        x86_64)  KUBE_ARCH="amd64" ;;
        aarch64) KUBE_ARCH="arm64" ;;
        *)        fail "Unsupported architecture: ${ARCH}" ;;
    esac
    curl -fsSL -o /tmp/kubectl \
        "https://dl.k8s.io/release/v${KUBECTL_VERSION}/bin/linux/${KUBE_ARCH}/kubectl"
    chmod +x /tmp/kubectl
    sudo mv /tmp/kubectl /usr/local/bin/kubectl
    ok "kubectl ${KUBECTL_VERSION} installed"
fi

# ── Helm ──────────────────────────────────────────────────────────────────────
step "Helm ${HELM_VERSION}"
if command -v helm &>/dev/null && helm version --short 2>/dev/null | grep -q "${HELM_VERSION}"; then
    ok "Helm ${HELM_VERSION} already installed"
else
    curl -fsSL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 \
        | DESIRED_VERSION="v${HELM_VERSION}" bash
    ok "Helm ${HELM_VERSION} installed"
fi

# ── k6 ────────────────────────────────────────────────────────────────────────
step "k6 ${K6_VERSION}"
if command -v k6 &>/dev/null && k6 version 2>/dev/null | grep -q "${K6_VERSION}"; then
    ok "k6 ${K6_VERSION} already installed"
else
    ARCH="$(uname -m)"
    case "${ARCH}" in
        x86_64)  K6_ARCH="amd64" ;;
        aarch64) K6_ARCH="arm64" ;;
        *)        fail "Unsupported architecture: ${ARCH}" ;;
    esac
    curl -fsSL -o /tmp/k6.tar.gz \
        "https://github.com/grafana/k6/releases/download/v${K6_VERSION}/k6-v${K6_VERSION}-linux-${K6_ARCH}.tar.gz"
    tar -xzf /tmp/k6.tar.gz -C /tmp
    sudo mv "/tmp/k6-v${K6_VERSION}-linux-${K6_ARCH}/k6" /usr/local/bin/k6
    rm -rf /tmp/k6.tar.gz "/tmp/k6-v${K6_VERSION}-linux-${K6_ARCH}"
    ok "k6 ${K6_VERSION} installed"
fi

# ── GitHub CLI ────────────────────────────────────────────────────────────────
step "GitHub CLI (gh)"
if command -v gh &>/dev/null; then
    ok "gh $(gh --version | head -1)"
else
    if command -v apt-get &>/dev/null; then
        curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
            | sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
        echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
            | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null
        sudo apt-get update -qq && sudo apt-get install -y gh
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y 'dnf-command(config-manager)'
        sudo dnf config-manager --add-repo https://cli.github.com/packages/rpm/gh-cli.repo
        sudo dnf install -y gh
    else
        warn "Install gh manually: https://github.com/cli/cli#installation"
    fi
    ok "gh installed"
fi

# ── pre-commit ────────────────────────────────────────────────────────────────
step "pre-commit"
if command -v pre-commit &>/dev/null; then
    ok "pre-commit $(pre-commit --version)"
else
    pip3 install --user pre-commit
    ok "pre-commit installed"
fi

# ── Cargo tools ───────────────────────────────────────────────────────────────
step "Cargo tools"
# shellcheck source=/dev/null
[[ -f "${HOME}/.cargo/env" ]] && source "${HOME}/.cargo/env"
for tool in cargo-audit cargo-watch; do
    if cargo "${tool#cargo-}" --version &>/dev/null 2>&1 || \
       "${HOME}/.cargo/bin/${tool}" --version &>/dev/null 2>&1; then
        ok "${tool} already installed"
    else
        echo "  Installing ${tool} …"
        cargo install --locked "${tool}"
        ok "${tool} installed"
    fi
done

# ── Pre-commit hooks ──────────────────────────────────────────────────────────
step "Pre-commit hooks"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "${REPO_ROOT}/.git/hooks/pre-commit" ]]; then
    ok "pre-commit hook already installed"
else
    (cd "${REPO_ROOT}" && pre-commit install && pre-commit install --hook-type pre-push)
    ok "pre-commit hooks installed"
fi

# ── Docker check ──────────────────────────────────────────────────────────────
step "Docker"
if command -v docker &>/dev/null && docker info &>/dev/null 2>&1; then
    ok "Docker daemon is running"
elif command -v docker &>/dev/null; then
    warn "Docker installed but daemon is NOT running — start it before running 'make quickstart'."
else
    warn "Docker not found. Install Docker Engine: https://docs.docker.com/engine/install/"
fi

# ── Local kind cluster (optional, idempotent) ─────────────────────────────────
step "Local kind cluster 'stellar-dev'"
if kind get clusters 2>/dev/null | grep -q "^stellar-dev$"; then
    ok "kind cluster 'stellar-dev' already exists"
else
    if docker info &>/dev/null 2>&1; then
        kind create cluster --name stellar-dev --wait 60s
        ok "kind cluster 'stellar-dev' created"
        kubectl apply -f "${REPO_ROOT}/config/crd/stellarnode-crd.yaml"
        ok "StellarNode CRD applied"
    else
        warn "Skipping cluster creation — Docker not running. Run 'make quickstart' later."
    fi
fi

# ── GitHub CLI auth reminder ──────────────────────────────────────────────────
step "GitHub CLI"
if gh auth status &>/dev/null 2>&1; then
    ok "gh already authenticated ($(gh auth status 2>&1 | grep 'Logged in' | head -1 | xargs))"
else
    warn "Run: gh auth login"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}=== Setup complete ===${NC}"
echo ""
echo "  Rust      : $(rustc --version 2>/dev/null)"
echo "  Cargo     : $(cargo --version 2>/dev/null)"
echo "  Docker    : $(docker --version 2>/dev/null || echo 'not running')"
echo "  kubectl   : $(kubectl version --client --short 2>/dev/null | head -1)"
echo "  kind      : $(kind --version 2>/dev/null)"
echo "  Helm      : $(helm version --short 2>/dev/null)"
echo "  k6        : $(k6 version 2>/dev/null | head -1)"
echo "  pre-commit: $(pre-commit --version 2>/dev/null)"
echo ""
echo "Next steps:"
echo "  make ci-local    — fmt + lint + audit + test + build"
echo "  make quickstart  — full kind cluster end-to-end"
