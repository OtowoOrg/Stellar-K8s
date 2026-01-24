.PHONY: help build test fmt lint clean docker-build install-crd apply-samples dev-setup

# Default target
.DEFAULT_GOAL := help

# Variables
CARGO := cargo
KUBECTL := kubectl
DOCKER := docker
IMAGE_NAME := stellar-operator
IMAGE_TAG := latest

help: ## Show this help message
	@echo 'Usage: make [target]'
	@echo ''
	@echo 'Available targets:'
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  %-20s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the operator binary
	$(CARGO) build --release

test: ## Run tests
	$(CARGO) test --all-features --verbose

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check code formatting
	$(CARGO) fmt --all -- --check

lint: ## Run clippy linter
	$(CARGO) clippy --all-targets --all-features -- -D warnings

audit: ## Run security audit
	$(CARGO) audit

clean: ## Clean build artifacts
	$(CARGO) clean
	rm -rf target/

docker-build: ## Build Docker image
	$(DOCKER) build -t $(IMAGE_NAME):$(IMAGE_TAG) .

docker-run: docker-build ## Run operator in Docker
	$(DOCKER) run --rm -p 8080:8080 -p 9090:9090 $(IMAGE_NAME):$(IMAGE_TAG)

install-crd: ## Install CRDs to cluster
	$(KUBECTL) apply -f config/crd/stellarnode-crd.yaml

uninstall-crd: ## Uninstall CRDs from cluster
	$(KUBECTL) delete -f config/crd/stellarnode-crd.yaml

apply-samples: install-crd ## Apply sample resources
	$(KUBECTL) apply -f config/samples/

delete-samples: ## Delete sample resources
	$(KUBECTL) delete -f config/samples/ --ignore-not-found

dev-setup: ## Setup development environment
	rustup component add clippy rustfmt
	$(CARGO) install cargo-audit cargo-watch

watch: ## Watch for changes and rebuild
	cargo watch -x check -x test -x build

benchmark: ## Run benchmarks
	@echo "Running benchmarks..."
	cd benchmarks && k6 run k6/operator-load-test.js

run: build ## Run the operator locally
	RUST_LOG=info ./target/release/stellar-operator

run-dev: ## Run the operator in dev mode with hot reload
	RUST_LOG=debug cargo watch -x run

all: fmt lint test build ## Format, lint, test, and build
