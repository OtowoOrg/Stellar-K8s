#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== Stellar-K8s macOS Development Setup ===${NC}\n"

# Check if running on macOS
if [[ "$OSTYPE" != "darwin"* ]]; then
    echo -e "${RED}Error: This script is for macOS only${NC}"
    exit 1
fi

# Install Homebrew if not present
if ! command -v brew &> /dev/null; then
    echo -e "${YELLOW}Installing Homebrew...${NC}"
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
else
    echo -e "${GREEN}✓ Homebrew already installed${NC}"
fi

# Install Rust
if ! command -v rustc &> /dev/null; then
    echo -e "${YELLOW}Installing Rust...${NC}"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo -e "${GREEN}✓ Rust already installed ($(rustc --version))${NC}"
fi

# Update Rust to latest stable
echo -e "${YELLOW}Updating Rust to latest stable...${NC}"
rustup update stable
rustup component add rustfmt clippy

# Install Docker
if ! command -v docker &> /dev/null; then
    echo -e "${YELLOW}Installing Docker...${NC}"
    brew install --cask docker
    echo -e "${YELLOW}Please start Docker Desktop from Applications folder${NC}"
else
    echo -e "${GREEN}✓ Docker already installed${NC}"
fi

# Install kubectl
if ! command -v kubectl &> /dev/null; then
    echo -e "${YELLOW}Installing kubectl...${NC}"
    brew install kubectl
else
    echo -e "${GREEN}✓ kubectl already installed ($(kubectl version --client --short 2>/dev/null || echo 'version check failed'))${NC}"
fi

# Install kind
if ! command -v kind &> /dev/null; then
    echo -e "${YELLOW}Installing kind...${NC}"
    brew install kind
else
    echo -e "${GREEN}✓ kind already installed${NC}"
fi

# Install Helm
if ! command -v helm &> /dev/null; then
    echo -e "${YELLOW}Installing Helm...${NC}"
    brew install helm
else
    echo -e "${GREEN}✓ Helm already installed ($(helm version --short 2>/dev/null || echo 'version check failed'))${NC}"
fi

# Install GitHub CLI
if ! command -v gh &> /dev/null; then
    echo -e "${YELLOW}Installing GitHub CLI...${NC}"
    brew install gh
else
    echo -e "${GREEN}✓ GitHub CLI already installed${NC}"
fi

# Install Make
if ! command -v make &> /dev/null; then
    echo -e "${YELLOW}Installing Make...${NC}"
    brew install make
else
    echo -e "${GREEN}✓ Make already installed${NC}"
fi

# Install cargo-audit
if ! cargo audit --version &> /dev/null; then
    echo -e "${YELLOW}Installing cargo-audit...${NC}"
    cargo install cargo-audit
else
    echo -e "${GREEN}✓ cargo-audit already installed${NC}"
fi

# Install pre-commit
if ! command -v pre-commit &> /dev/null; then
    echo -e "${YELLOW}Installing pre-commit...${NC}"
    brew install pre-commit
else
    echo -e "${GREEN}✓ pre-commit already installed${NC}"
fi

echo -e "\n${GREEN}=== Installation Complete ===${NC}\n"

# Verification
echo -e "${YELLOW}Verifying installations...${NC}"
echo "Rust: $(rustc --version)"
echo "Cargo: $(cargo --version)"
echo "Docker: $(docker --version 2>/dev/null || echo 'Not running')"
echo "kubectl: $(kubectl version --client --short 2>/dev/null || echo 'Not available')"
echo "kind: $(kind --version)"
echo "Helm: $(helm version --short 2>/dev/null || echo 'Not available')"
echo "GitHub CLI: $(gh --version 2>/dev/null | head -1)"
echo "Make: $(make --version | head -1)"
echo "pre-commit: $(pre-commit --version)"

echo -e "\n${YELLOW}Manual Steps:${NC}"
echo "1. Start Docker Desktop from Applications folder"
echo "2. Configure GitHub CLI: gh auth login"
echo "3. Clone the repository: git clone https://github.com/YOUR_USERNAME/stellar-k8s.git"
echo "4. Setup development environment: cd stellar-k8s && make dev-setup"
echo "5. Run tests: make test"

echo -e "\n${GREEN}Setup complete! Happy coding! 🚀${NC}"
