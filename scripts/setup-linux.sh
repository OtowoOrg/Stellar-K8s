#!/usr/bin/env bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
# scripts/setup-linux.sh — Linux developer environment bootstrap (Ubuntu/Debian/Fedora)
# Idempotent: safe to re-run. Installs Rust, Docker deps, kubectl, kind, helm, tools.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/versions.sh
source "${SCRIPT_DIR}/lib/versions.sh"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info(){ echo -e "${GREEN}→${NC} $*"; }
warn(){ echo -e "${YELLOW}⚠${NC} $*"; }
fail(){ echo -e "${RED}✗${NC} $*"; }

detect_os() {
  if [[ -f /etc/os-release ]]; then . /etc/os-release; echo "${ID:-linux}"; else echo "linux"; fi
}

install_rust() {
  if command -v cargo >/dev/null 2>&1; then info "Rust already installed: $(cargo --version)"; return; fi
  info "Installing Rust via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env" || true
  rustup component add clippy rustfmt || true
}

install_system_deps() {
  local os; os=$(detect_os)
  info "Installing system dependencies for ${os}..."
  if command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -y
    sudo apt-get install -y curl git build-essential pkg-config libssl-dev jq python3-pip || true
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y curl git gcc openssl-devel jq python3-pip || true
  else
    warn "Unknown package manager — please install curl, git, build-essential, libssl-dev manually"
  fi
}

install_kubectl() {
  if command -v kubectl >/dev/null 2>&1; then info "kubectl already installed: $(kubectl version --client 2>&1 | head -1)"; return; fi
  info "Installing kubectl v${KUBCTL_VERSION:-${KUBECTL_VERSION}}..."
  # Use version from versions.sh
  local ver="${KUBECTL_VERSION}"
  curl -fsSL -o /tmp/kubectl "https://dl.k8s.io/release/v${ver}/bin/linux/amd64/kubectl"
  chmod +x /tmp/kubectl
  sudo mv /tmp/kubectl /usr/local/bin/kubectl
  info "kubectl installed: $(kubectl version --client 2>&1 | head -1)"
}

install_kind() {
  if command -v kind >/dev/null 2>&1; then info "kind already installed: $(kind version)"; return; fi
  info "Installing kind v${KIND_VERSION}..."
  curl -fsSL -o /tmp/kind "https://kind.sigs.k8s.io/dl/v${KIND_VERSION}/kind-linux-amd64"
  chmod +x /tmp/kind
  sudo mv /tmp/kind /usr/local/bin/kind
  info "kind installed: $(kind version)"
}

install_helm() {
  if command -v helm >/dev/null 2>&1; then info "helm already installed: $(helm version --short 2>&1 | head -1)"; return; fi
  info "Installing helm v${HELM_VERSION}..."
  curl -fsSL "https://get.helm.sh/helm-v${HELM_VERSION}-linux-amd64.tar.gz" | tar xz -C /tmp
  sudo mv /tmp/linux-amd64/helm /usr/local/bin/helm
  info "helm installed: $(helm version --short 2>&1 | head -1)"
}

install_precommit() {
  if command -v pre-commit >/dev/null 2>&1; then info "pre-commit already installed"; return; fi
  info "Installing pre-commit via pip..."
  pip install --user pre-commit || pip3 install --user pre-commit || warn "pip install failed — install pre-commit manually"
}

main() {
  echo "╔════════════════════════════════════════════════════════════════╗"
  echo "║         Stellar-K8s Linux Setup                              ║"
  echo "╚════════════════════════════════════════════════════════════════╝"
  install_system_deps
  install_rust
  install_kubectl
  install_kind
  install_helm
  install_precommit
  echo ""
  info "Running make dev-setup (Rust components + hooks)..."
  make dev-setup || warn "make dev-setup failed — run manually"
  echo ""
  info "Verifying with health-check..."
  bash "${SCRIPT_DIR}/health-check.sh" || true
  echo ""
  echo -e "${GREEN}✓ Linux setup complete${NC}"
  echo "Next: make preflight && make health-check && make quick"
}

main "$@"
