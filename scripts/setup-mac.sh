#!/bin/bash
# Repeatable macOS dev environment bootstrap for Stellar-K8s.
# Safe to run multiple times — skips steps already at the right version.
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

echo -e "${GREEN}=== Stellar-K8s macOS Dev Setup ===${NC}"
echo    "    Rust ${RUST_TOOLCHAIN} | kind ${KIND_VERSION} | kubectl ${KUBECTL_VERSION} | helm ${HELM_VERSION}"

[[ "$OSTYPE" == "darwin"* ]] || fail "This script is for macOS only."

# ── Homebrew ──────────────────────────────────────────────────────────────────
step "Homebrew"
if ! command -v brew &>/dev/null; then
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
else
    ok "Homebrew $(brew --version | head -1)"
fi

# ── Rust / rustup ─────────────────────────────────────────────────────────────
step "Rust toolchain (${RUST_TOOLCHAIN})"
if ! command -v rustup &>/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
        -y --default-toolchain "${RUST_TOOLCHAIN}" --profile minimal
    # shellcheck source=/dev/null
    source "${HOME}/.cargo/env"
else
    # Ensure the pinned toolchain is installed and set as default.
    rustup toolchain install "${RUST_TOOLCHAIN}" --profile minimal 2>/dev/null || true
    rustup default "${RUST_TOOLCHAIN}"
    ok "Rust $(rustc --version)"
fi
rustup component add rustfmt clippy 2>/dev/null || true
ok "rustfmt + clippy"

# ── Homebrew packages (idempotent: brew install is a no-op if present) ────────
step "Homebrew packages"
BREW_PKGS=(
    docker
    kind
    kubectl
    helm
    gh
    pre-commit
    shellcheck
    k6
)
for pkg in "${BREW_PKGS[@]}"; do
    if brew list --formula "$pkg" &>/dev/null || brew list --cask "$pkg" &>/dev/null; then
        ok "$pkg already installed"
    else
        echo "  Installing $pkg …"
        brew install "$pkg" 2>/dev/null || brew install --cask "$pkg"
        ok "$pkg installed"
    fi
done

# ── Version pins: warn if the installed version differs from pinned ────────────
step "Version checks"
_check_version() {
    local tool=$1 want=$2 got=$3
    if [[ "$got" == "$want"* ]]; then
        ok "${tool} ${got}"
    else
        warn "${tool}: installed ${got}, pinned ${want} — run: brew upgrade ${tool}"
    fi
}

KIND_GOT=$(kind --version 2>/dev/null | awk '{print $3}' || echo "missing")
_check_version "kind" "${KIND_VERSION}" "${KIND_GOT}"

KUBECTL_GOT=$(kubectl version --client -o json 2>/dev/null | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['clientVersion']['gitVersion'].lstrip('v'))" \
    2>/dev/null || echo "missing")
_check_version "kubectl" "${KUBECTL_VERSION}" "${KUBECTL_GOT}"

HELM_GOT=$(helm version --short 2>/dev/null | sed 's/v//' | cut -d'+' -f1 || echo "missing")
_check_version "helm" "${HELM_VERSION}" "${HELM_GOT}"

K6_GOT=$(k6 version 2>/dev/null | awk '{print $2}' | sed 's/v//' || echo "missing")
_check_version "k6" "${K6_VERSION}" "${K6_GOT}"

# ── Cargo tools ───────────────────────────────────────────────────────────────
step "Cargo tools"
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

# ── Docker Desktop check ──────────────────────────────────────────────────────
step "Docker"
if docker info &>/dev/null 2>&1; then
    ok "Docker daemon is running"
else
    warn "Docker daemon is NOT running."
    warn "Start Docker Desktop from your Applications folder, then re-run this script."
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
