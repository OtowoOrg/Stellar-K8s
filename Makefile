# =============================================================================
# Stellar-K8s Makefile
#
# Canonical Command Flow:
#   Setup:    make dev-setup                  # One-time environment setup
#   Check:    make quick                      # Fast pre-commit (fmt check + compile)
#   CI:       make ci-local                   # Full CI pipeline (fmt + lint + audit + test + build + links)
#   Format:   make fmt                        # Auto-format code
#   Build:    make build                      # Release binary build
#   Test:     make test                       # Run all tests
#   Security: make security-all               # Audit + scan
#   Docker:   make docker-build               # Local Docker image
#   Clean:    make clean                      # Remove build artifacts
#   Health:   make health                     # Full health check
#   Help:     make help                       # Show all targets
#
# See DEVELOPMENT.md for full workflow details.
# =============================================================================

.PHONY: help \
	fmt fmt-check lint lint-strict shellcheck audit security-audit security-scan security-all \
	build test ci-local quick watch \
	docker-build docker-build-ci docker-multiarch \
	dev-setup pre-commit pre-commit-install run run-local run-dev \
	install-crd apply-samples crd-gen regenerate completions \
	helm-lint link-check link-check-all changelog release-notes \
	generate-api-docs check-api-docs \
	third-party-licenses check-third-party-licenses \
	benchmark benchmark-upgrade benchmark-webhook benchmark-webhook-health \
	benchmark-webhook-compare benchmark-webhook-save benchmark-all \
	compose-up compose-dev compose-down compose-logs \
	bundle bundle-build \
	quickstart validate preflight health test-preflight test-shell \
	test-validate test-repo-health test-scripts all \
	clean

.DEFAULT_GOAL := help

# Variables
CARGO := cargo
KUBECTL := kubectl
DOCKER := docker
IMAGE_NAME := stellar-operator
IMAGE_TAG ?= latest

# Bundle variables
VERSION ?= 0.1.0
BUNDLE_IMG ?= $(IMAGE_NAME)-bundle:v$(VERSION)
CHANNELS ?= "alpha"
DEFAULT_CHANNEL ?= "alpha"

help: ## Show this help and the canonical command flow
	@echo 'Stellar-K8s Makefile'
	@echo ''
	@echo 'Canonical Command Flow:'
	@echo '  Setup:    make dev-setup         One-time environment setup'
	@echo '  Check:    make quick             Fast pre-commit (fmt + cargo check)'
	@echo '  CI:       make ci-local          Full CI pipeline locally'
	@echo '  Format:   make fmt               Auto-format code'
	@echo '  Build:    make build              Release binary build'
	@echo '  Test:     make test               Run all tests'
	@echo '  Security: make security-all       Audit + scan'
	@echo '  Docker:   make docker-build       Local Docker image'
	@echo '  Clean:    make clean              Remove build artifacts'
	@echo ''
	@echo 'Workflows:'
	@echo '  make quickstart                  End-to-end local quickstart (kind cluster)'
	@echo '  make health                      Full contributor health gate'
	@echo '  make all                         CI checks + build + Docker image'
	@echo ''
	@echo 'All available targets:'
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_][a-zA-Z0-9_-]+:.*?## / {printf "  %-28s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# ── Formatting & Linting ──────────────────────────────────────────────────────

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	@echo "→ Checking format..."
	@$(CARGO) fmt --all --check && echo "✓ Format OK" || (echo "✗ Run: make fmt" && exit 1)

lint: ## Run clippy
	@echo "→ Running clippy..."
	@K8S_OPENAPI_ENABLED_VERSION=1.30 $(CARGO) clippy --workspace --all-targets \
		--features "rest-api,metrics,admission-webhook,k8s-v1-30,reconciler-fuzz" -- \
		-D clippy::correctness \
		-D clippy::suspicious \
		-D clippy::perf \
		-D clippy::style \
		-A clippy::new_without_default \
		-A clippy::match_like_matches_macro \
		-A clippy::match_result_ok \
		-A clippy::needless_borrow \
		-A clippy::get_first \
		-A clippy::format_in_format_args \
		-A clippy::single_match \
		-A clippy::redundant_closure \
		-A clippy::items_after_test_module \
		-A clippy::approx_constant \
		-A clippy::should_implement_trait

lint-strict: ## Run clippy (adds complexity checks on top of lint; same base exceptions)
	@echo "→ Running clippy (strict mode)..."
	@K8S_OPENAPI_ENABLED_VERSION=1.30 $(CARGO) clippy --workspace --all-targets \
		--features "rest-api,metrics,admission-webhook,k8s-v1-30,reconciler-fuzz" -- \
		-D clippy::correctness \
		-D clippy::suspicious \
		-D clippy::perf \
		-D clippy::style \
		-D clippy::complexity \
		-A clippy::new_without_default \
		-A clippy::match_like_matches_macro \
		-A clippy::match_result_ok \
		-A clippy::needless_borrow \
		-A clippy::get_first \
		-A clippy::format_in_format_args \
		-A clippy::single_match \
		-A clippy::redundant_closure \
		-A clippy::items_after_test_module \
		-A clippy::approx_constant \
		-A clippy::should_implement_trait \
		-A clippy::cognitive_complexity \
		-A clippy::too_many_lines \
		-A clippy::type_complexity

# ── Security ──────────────────────────────────────────────────────────────────

audit: ## Security audit (cargo audit)
	@echo "→ Running security audit..."
	@command -v cargo-audit >/dev/null 2>&1 || cargo install --locked cargo-audit
	@$(CARGO) audit --deny unsound || echo "⚠️  Security issues found - review before production"

security-audit: audit ## Alias for audit

security-scan: ## Run security scan (audit + shellcheck)
	$(MAKE) audit
	$(MAKE) shellcheck

security-all: ## Run all security checks (audit + shellcheck)
	$(MAKE) audit
	$(MAKE) shellcheck

shellcheck: ## Run shellcheck on all shell scripts
	@echo "→ Running shellcheck..."
	@find scripts -type f -name "*.sh" -print0 | xargs -0 shellcheck -S error || true

# ── Test & Build ──────────────────────────────────────────────────────────────

test: ## Run tests
	@echo "→ Running tests..."
	@$(CARGO) test --workspace --features "rest-api,metrics,admission-webhook,k8s-v1-30,reconciler-fuzz" --tests --lib --bins --verbose
	@echo "→ Running doc tests..."
	@$(CARGO) test --doc --workspace --features "rest-api,metrics,admission-webhook,k8s-v1-30"

build: ## Build release
	@echo "→ Building release..."
	@$(CARGO) build --release --locked

# ── Docker ────────────────────────────────────────────────────────────────────

docker-build: ## Fast local Docker build using host release binaries
	@echo "→ Building Docker image (fast local mode)..."
	@if [ ! -f target/release/stellar-operator ] || [ ! -f target/release/kubectl-stellar ]; then \
		echo "→ Release binaries not found, building once..."; \
		$(MAKE) build; \
	fi
	DOCKER_BUILDKIT=1 $(DOCKER) build --target runtime-local -t $(IMAGE_NAME):$(IMAGE_TAG) .

docker-build-ci: ## Reproducible CI Docker build (builds binaries in container)
	@echo "→ Building Docker image (CI mode)..."
	DOCKER_BUILDKIT=1 $(DOCKER) build --target runtime -t $(IMAGE_NAME):$(IMAGE_TAG) .

docker-multiarch: ## Build multi-arch Docker image
	$(DOCKER) buildx build --platform linux/amd64 -t $(IMAGE_NAME):$(IMAGE_TAG) .

# ── Quality & Health ───────────────────────────────────────────────────────────

link-check: ## Check markdown links repository-wide (docs + examples); install markdown-link-check via npm for full external checks
	@echo "→ Running local link checker (docs + examples)..."
	@python3 scripts/check-links.py --dirs docs examples
	@if command -v markdown-link-check >/dev/null 2>&1; then \
		echo "→ Running markdown-link-check on all .md files..."; \
		find . -name "*.md" \
			-not -path "*/node_modules/*" \
			-not -path "*/.git/*" \
			-not -path "*/target/*" \
			-not -path "*/.kiro/*" \
			-exec markdown-link-check -c mlc_config.json -q {} +; \
	else \
		echo "  (skipping markdown-link-check; install with: npm install -g markdown-link-check)"; \
	fi

link-check-all: ## Repo-wide link check (markdown + HTML) via lychee
	@echo "→ Running repo-wide link checker (lychee)..."
	@command -v lychee >/dev/null 2>&1 || { \
		echo "lychee not found. Install with: cargo install lychee --locked"; \
		exit 1; \
	}
	@lychee --config lychee.toml --no-progress --cache \
		'./**/*.md' './**/*.rs' './**/*.toml' \
		'./**/*.yaml' './**/*.yml' './**/*.sh' './**/*.html'

changelog: ## Regenerate CHANGELOG.md from full git history (requires git-cliff: cargo install git-cliff)
	@echo "→ Generating changelog..."
	@command -v git-cliff >/dev/null 2>&1 || { echo "git-cliff not found. Install with: cargo install git-cliff"; exit 1; }
	git-cliff --output CHANGELOG.md

release-notes: ## Generate release notes for the latest tag into release-notes.md (requires git-cliff)
	@echo "→ Generating release notes for the latest tag..."
	@command -v git-cliff >/dev/null 2>&1 || { echo "git-cliff not found. Install with: cargo install git-cliff"; exit 1; }
	git-cliff --latest --strip header --output release-notes.md
	@echo "✓ Written to release-notes.md"

ci-local: fmt-check lint audit test build link-check ## Run full CI locally
	@echo ""
	@echo "✓ All CI checks passed!"

third-party-licenses: ## Regenerate THIRD_PARTY_LICENSES.md from Cargo dependency tree
	@bash scripts/generate-third-party-licenses.sh

check-third-party-licenses: ## Verify THIRD_PARTY_LICENSES.md is up to date (used in CI)
	@bash scripts/generate-third-party-licenses.sh --check

health: ## Run common repository health checks (format, lint, test, docs)
	@bash scripts/repo-health.sh

quick: fmt-check ## Quick pre-commit check
	@$(CARGO) check --workspace
	@echo "✓ Quick checks passed"

pre-commit: ## Run pre-commit hooks manually
	@echo "→ Running pre-commit hooks..."
	@command -v pre-commit >/dev/null 2>&1 || (echo "✗ pre-commit not installed. Run: make dev-setup" && exit 1)
	@pre-commit run --all-files

pre-commit-install: ## Install pre-commit hooks
	@command -v pre-commit >/dev/null 2>&1 || pip install pre-commit
	pre-commit install
	pre-commit install --hook-type pre-push

clean: ## Clean build artifacts
	$(CARGO) clean

# ── API Documentation ─────────────────────────────────────────────────────────

generate-api-docs: ## Generate API reference docs from CRD schema
	@echo "→ Generating API reference docs..."
	@python3 scripts/generate-api-docs.py \
		--crd config/crd/stellarnode-crd.yaml \
		--output docs/api-reference.md
	@echo "✓ Generated docs/api-reference.md"

check-api-docs: ## Check API docs are up to date (used in CI)
	@echo "→ Checking API reference docs are up to date..."
	@python3 scripts/generate-api-docs.py \
		--crd config/crd/stellarnode-crd.yaml \
		--output docs/api-reference.md \
		--check

# ── Kubernetes ────────────────────────────────────────────────────────────────

install-crd: ## Install CRDs
	$(KUBECTL) apply -f config/crd/stellarnode-crd.yaml

apply-samples: install-crd ## Apply samples
	$(KUBECTL) apply -f config/samples/

crd-gen: ## Generate CRDs
	@echo "→ Generating CRDs..."
	@$(CARGO) run --bin crdgen > config/crd/stellarnode-crd.yaml

regenerate: crd-gen generate-api-docs bundle ## Regenerate all derived artifacts (CRDs, API docs, OLM bundle)
	@echo "✓ All generated artifacts are up to date"
	@echo "  See docs/development/regeneration-guide.md for details"

preflight: ## Check that required tools are installed (pass --labels to also verify repo labels)
	@bash scripts/preflight.sh $(ARGS)

test-preflight: ## Run bats unit tests for scripts/preflight.sh
	@echo "→ Running preflight bats tests..."
	@command -v bats >/dev/null 2>&1 || (echo "✗ bats not installed. See https://github.com/bats-core/bats-core" && exit 1)
	@bats scripts/tests/preflight.bats

test-shell: ## Run bats unit tests for shared shell helpers
	@echo "→ Running shell helper bats tests..."
	@command -v bats >/dev/null 2>&1 || (echo "✗ bats not installed. See https://github.com/bats-core/bats-core" && exit 1)
	@bats scripts/tests/common.bats

test-validate: ## Run bats regression tests for scripts/validate.sh
	@echo "→ Running validate script bats tests..."
	@command -v bats >/dev/null 2>&1 || (echo "✗ bats not installed. See https://github.com/bats-core/bats-core" && exit 1)
	@bats scripts/tests/validate.bats

test-repo-health: ## Run bats regression tests for scripts/repo-health.sh and lib/errors.sh
	@echo "→ Running repo-health bats tests..."
	@command -v bats >/dev/null 2>&1 || (echo "✗ bats not installed. See https://github.com/bats-core/bats-core" && exit 1)
	@bats scripts/tests/repo_health.bats

test-scripts: test-shell test-preflight test-validate test-repo-health ## Run all shell script bats tests
	@echo "✓ All shell script tests passed"

# ── Completions ────────────────────────────────────────────────────────────────

completions: ## Generate shell completion scripts
	@echo "→ Generating shell completions..."
	@mkdir -p completions
	@$(CARGO) run --bin stellar-completions completions bash > completions/stellar-operator.bash
	@$(CARGO) run --bin stellar-completions completions zsh > completions/_stellar-operator
	@$(CARGO) run --bin stellar-completions completions fish > completions/stellar-operator.fish
	@echo "✓ Completions generated in ./completions/"
	@echo "  Bash: source completions/stellar-operator.bash"
	@echo "  Zsh:  Copy completions/_stellar-operator to your fpath"
	@echo "  Fish: Copy completions/stellar-operator.fish to ~/.config/fish/completions/"

# ── Helm ──────────────────────────────────────────────────────────────────────

helm-lint: ## Helm lint check
	@echo "→ Linting Helm charts..."
	helm lint charts/stellar-operator --strict
	@echo "→ Validating Helm template rendering..."
	helm template stellar-operator charts/stellar-operator > /dev/null
	@echo "✓ Helm charts passed linting and validation"

# ── Development Setup ─────────────────────────────────────────────────────────

dev-setup: ## Setup dev environment
	rustup update stable
	rustup default stable
	rustup component add clippy rustfmt
	cargo install cargo-audit cargo-watch
	@command -v pre-commit >/dev/null 2>&1 || pip install pre-commit
	pre-commit install
	pre-commit install --hook-type pre-push
	@$(MAKE) preflight

# ── Watch ──────────────────────────────────────────────────────────────────────

watch: ## Watch and rebuild
	cargo watch -x check -x test -x build

# ── Benchmarks ────────────────────────────────────────────────────────────────

benchmark: ## Run k6 performance benchmarks
	@echo "→ Running k6 benchmarks..."
	@command -v k6 >/dev/null 2>&1 || (echo "✗ k6 not installed. Install: https://k6.io/docs/get-started/installation/" && exit 1)
	cd benchmarks && k6 run k6/operator-load-test.js

benchmark-webhook: ## Run webhook performance benchmarks
	@echo "→ Running webhook benchmarks..."
	@command -v k6 >/dev/null 2>&1 || (echo "✗ k6 not installed. Install: https://k6.io/docs/get-started/installation/" && exit 1)
	@./benchmarks/run-webhook-benchmark.sh run

benchmark-webhook-health: ## Check webhook health
	@./benchmarks/run-webhook-benchmark.sh health

benchmark-webhook-compare: ## Compare webhook results with baseline
	@./benchmarks/run-webhook-benchmark.sh compare

benchmark-webhook-save: ## Save current results as baseline
	@./benchmarks/run-webhook-benchmark.sh save-baseline

benchmark-all: benchmark benchmark-webhook ## Run all benchmarks

benchmark-upgrade: ## Run upgrade load test with k6
	@echo "→ Running upgrade load test..."
	@command -v k6 >/dev/null 2>&1 || (echo "✗ k6 not installed. Install: https://k6.io/docs/get-started/installation/" && exit 1)
	cd benchmarks && k6 run k6/upgrade-load-test.js

# ── Running the Operator ──────────────────────────────────────────────────────

run-local: build ## Run operator locally from built release binary
	RUST_LOG=info ./target/release/stellar-operator

run: run-local ## Alias for run-local

run-dev: ## Run operator in dev mode with hot reload
	RUST_LOG=debug cargo watch -x run


# ── Bundle ────────────────────────────────────────────────────────────────────

bundle: ## Generate bundle manifests and metadata, then validate generated files.
	@echo "→ Generating manifests from Helm chart..."
	@mkdir -p rendered
	@helm template stellar-operator charts/stellar-operator > rendered/manifests.yaml
	@echo "→ Generating bundle..."
	@operator-sdk generate kustomize manifests -q
	@kustomize build config/manifests | operator-sdk generate bundle -q --overwrite --version $(VERSION) --channels $(CHANNELS) --default-channel $(DEFAULT_CHANNEL)
	@echo "→ Validating bundle..."
	@operator-sdk bundle validate ./bundle
	@rm -rf rendered

bundle-build: ## Build the bundle image.
	docker build -f bundle.Dockerfile -t $(BUNDLE_IMG) .

# ── Quickstart ────────────────────────────────────────────────────────────────

quickstart: ## End-to-end local quickstart: kind cluster + CRD + operator + sample StellarNode
	@echo "→ Checking prerequisites..."
	@command -v kind >/dev/null 2>&1 || (echo "✗ kind not found. Install: https://kind.sigs.k8s.io/docs/user/quick-start/#installation" && exit 1)
	@command -v kubectl >/dev/null 2>&1 || (echo "✗ kubectl not found. Install: https://kubernetes.io/docs/tasks/tools/" && exit 1)
	@command -v helm >/dev/null 2>&1 || (echo "✗ helm not found. Install: https://helm.sh/docs/intro/install/" && exit 1)
	@echo "→ Creating kind cluster 'stellar-dev'..."
	@kind create cluster --name stellar-dev --wait 120s || echo "  (cluster may already exist, continuing)"
	@echo "→ Building operator image..."
	@$(MAKE) build
	@DOCKER_BUILDKIT=1 $(DOCKER) build --target runtime-local -t stellar-operator:dev .
	@echo "→ Loading image into kind cluster..."
	@kind load docker-image stellar-operator:dev --name stellar-dev
	@echo "→ Installing CRD..."
	@$(KUBECTL) apply -f config/crd/stellarnode-crd.yaml
	@echo "→ Creating namespace stellar-system..."
	@$(KUBECTL) create namespace stellar-system --dry-run=client -o yaml | $(KUBECTL) apply -f -
	@echo "→ Deploying operator via Helm..."
	@helm upgrade --install stellar-operator charts/stellar-operator \
		--namespace stellar-system \
		--set image.tag=dev \
		--set image.pullPolicy=Never \
		--wait --timeout 120s
	@echo "→ Applying sample StellarNode..."
	@$(KUBECTL) apply -f config/samples/test-stellarnode.yaml
	@echo ""
	@echo "✓ Quickstart complete!"
	@echo "  Watch nodes:    kubectl get stellarnode -n stellar-system -w"
	@echo "  View resources: kubectl get deploy,sts,svc,pvc -n stellar-system"
	@echo "  Cleanup:        kind delete cluster --name stellar-dev"

validate: ## Run local validation script (format + lint + compile)
	@bash scripts/validate.sh

# ── Full Pipeline ──────────────────────────────────────────────────────────────

all: ci-local docker-build ## Full build pipeline: CI checks + Docker image

# ── Docker Compose ────────────────────────────────────────────────────────────

compose-up: ## Start Docker Compose development environment
	@echo "→ Starting Docker Compose environment..."
	@docker-compose up -d
	@echo "✓ Environment started. Use 'make compose-logs' to view logs"

compose-dev: ## Start Docker Compose with hot-reloading
	@echo "→ Starting Docker Compose with hot-reloading..."
	@docker-compose -f docker-compose.yml -f docker-compose.dev.yml up

compose-down: ## Stop Docker Compose environment
	@echo "→ Stopping Docker Compose environment..."
	@docker-compose down

compose-logs: ## View Docker Compose logs
	@docker-compose logs -f stellar-operator
