.PHONY: help build test fmt lint clean docker-build install-crd apply-samples dev-setup ci-local benchmark run-dev

# Default target
.DEFAULT_GOAL := help

# Variables
CARGO := cargo
KUBECTL := kubectl
DOCKER := docker
IMAGE_NAME := stellar-operator
IMAGE_TAG := latest

help: ## Show this help
	@echo 'Usage: make [target]'
	@echo ''
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  %-20s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	@echo "→ Checking format..."
	@$(CARGO) fmt --all --check && echo "✓ Format OK" || (echo "✗ Run: make fmt" && exit 1)

lint: ## Run clippy
	@echo "→ Running clippy..."
	@$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

audit: ## Security audit
	@echo "→ Running security audit..."
	@command -v cargo-audit >/dev/null 2>&1 || cargo install --locked cargo-audit
	@$(CARGO) audit --deny unsound || echo "⚠️  Security issues found - review before production"

test: ## Run tests
	@echo "→ Running tests..."
	@$(CARGO) test --workspace --all-features --verbose
	@echo "→ Running doc tests..."
	@$(CARGO) test --doc --workspace

build: ## Build release
	@echo "→ Building release..."
	@$(CARGO) build --release --locked

docker-build: ## Build Docker image
	@echo "→ Building Docker image..."
	$(DOCKER) build -t $(IMAGE_NAME):$(IMAGE_TAG) .

docker-multiarch: ## Build multi-arch Docker image
	$(DOCKER) buildx build --platform linux/amd64,linux/arm64 -t $(IMAGE_NAME):$(IMAGE_TAG) .

ci-local: fmt-check lint audit test build ## Run full CI locally
	@echo ""
	@echo "✓ All CI checks passed!"

quick: fmt-check ## Quick pre-commit check
	@$(CARGO) check --workspace
	@echo "✓ Quick checks passed"

clean: ## Clean build artifacts
	$(CARGO) clean

install-crd: ## Install CRDs
	$(KUBECTL) apply -f config/crd/stellarnode-crd.yaml

apply-samples: install-crd ## Apply samples
	$(KUBECTL) apply -f config/samples/

dev-setup: ## Setup dev environment
	rustup update stable
	rustup default stable
	rustup component add clippy rustfmt
	cargo install cargo-audit cargo-watch

watch: ## Watch and rebuild
	cargo watch -x check -x test -x build

benchmark: ## Run k6 performance benchmarks
	@echo "→ Running k6 benchmarks..."
	@command -v k6 >/dev/null 2>&1 || (echo "✗ k6 not installed. Install: https://k6.io/docs/get-started/installation/" && exit 1)
	cd benchmarks && k6 run k6/operator-load-test.js

run: build ## Run locally
	RUST_LOG=info ./target/release/stellar-operator

run-dev: ## Run operator in dev mode with hot reload
	RUST_LOG=debug cargo watch -x run

all: ci-local docker-build ## Full build pipeline
