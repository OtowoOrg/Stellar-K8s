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
# scripts/setup-mac.sh — macOS developer environment bootstrap (Homebrew)
# Idempotent: safe to re-run. Installs Rust, Docker, kubectl, kind, helm, tools.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/versions.sh
source "${SCRIPT_DIR}/lib/versions.sh"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info(){ echo -e "${GREEN}→${NC} $*"; }
warn(){ echo -e "${YELLOW}⚠${NC} $*"; }

ensure_brew() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "Homebrew not found. Install from https://brew.sh then re-run."; exit 1
  fi
}

install_rust() {
  if command -v cargo >/dev/null 2>&1; then info "Rust already installed: $(cargo --version)"; return; fi
  if command -v brew >/dev/null 2>&1; then
    info "Installing Rust via rustup (preferred)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env" || true
  fi
  rustup component add clippy rustfmt || true
}

main() {
  echo "╔════════════════════════════════════════════════════════════════╗"
  echo "║         Stellar-K8s macOS Setup                              ║"
  echo "╚════════════════════════════════════════════════════════════════╝"
  ensure_brew
  info "Installing tools via Homebrew..."
  brew update || true
  brew install kubectl kind helm pre-commit shellcheck jq || true
  brew install rustup 2>/dev/null || true
  if ! command -v cargo >/dev/null 2>&1; then
    install_rust
  else
    info "cargo: $(cargo --version)"
  fi
  rustup component add clippy rustfmt 2>/dev/null || true
  echo ""
  info "Running make dev-setup (Rust components + hooks)..."
  make dev-setup || warn "make dev-setup failed — run manually"
  echo ""
  info "Verifying with health-check..."
  bash "${SCRIPT_DIR}/health-check.sh" || true
  echo ""
  echo -e "${GREEN}✓ macOS setup complete${NC}"
  echo "Next: make preflight && make health-check && make quick"
}

main "$@"
