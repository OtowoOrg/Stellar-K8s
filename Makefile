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
	fmt fmt-check lint lint-strict shellcheck audit security-scan security-all security-fix security-report security-check \
	build test ci-local quick watch \
	docker-build docker-build-ci docker-multiarch \
	dev-setup dev-setup-rust dev-setup-tools dev-setup-hooks pre-commit pre-commit-install run run-local run-dev \
	install-crd apply-samples crd-gen regenerate completions completions-bash completions-zsh completions-fish \
	helm-lint link-check link-check-all changelog \
	generate-api-docs check-api-docs check-stale-docs update-doc-baseline list-doc-coverage docs-check-strict docs-lint \
	third-party-licenses check-third-party-licenses sort-manifests \
	benchmark benchmark-upgrade benchmark-webhook benchmark-webhook-health \
	benchmark-webhook-compare benchmark-webhook-save benchmark-all \
	compose-up compose-dev compose-down compose-logs \
	bundle bundle-render bundle-generate bundle-validate bundle-build \
	quickstart quickstart-setup quickstart-build quickstart-deploy \
	health health-fast validate preflight test-preflight test-shell test-repo-health all \
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

# Clippy configuration (shared between lint and lint-strict)
CLIPPY_BASE_FLAGS := \
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

CLIPPY_STRICT_FLAGS := \
	-D clippy::complexity \
	-A clippy::cognitive_complexity \
	-A clippy::too_many_lines \
	-A clippy::type_complexity

CLIPPY_FEATURES := "rest-api,metrics,admission-webhook,k8s-v1-30,reconciler-fuzz"

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
	@echo '  Security: make security-all       Complete security audit suite'
	@echo '  Security: make audit              Vulnerability scan + policy check'
	@echo '  Security: make security-report    Generate security report'
	@echo '  Docker:   make docker-build       Local Docker image'
	@echo '  Clean:    make clean              Remove build artifacts'
	@echo ''
	@echo 'Workflows:'
	@echo '  make quickstart                  End-to-end local quickstart (kind cluster)'
	@echo '  make health                      Full contributor health gate'
	@echo '  make all                         CI checks + build + Docker image'
	@echo ''
	@echo 'All available targets:'
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_][a-zA-Z0-9_-]+:.*## / {printf "  %-28s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# ── Formatting & Linting ──────────────────────────────────────────────────────

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	@echo "→ Checking format..."
	@$(CARGO) fmt --all --check && echo "✓ Format OK" || (echo "✗ Run: make fmt" && exit 1)

lint: ## Run clippy
	@echo "→ Running clippy..."
	@K8S_OPENAPI_ENABLED_VERSION=1.30 $(CARGO) clippy --workspace --all-targets \
		--features $(CLIPPY_FEATURES) -- \
		$(CLIPPY_BASE_FLAGS)

lint-strict: ## Run clippy (adds complexity checks on top of lint; same base exceptions)
	@echo "→ Running clippy (strict mode)..."
	@K8S_OPENAPI_ENABLED_VERSION=1.30 $(CARGO) clippy --workspace --all-targets \
		--features $(CLIPPY_FEATURES) -- \
		$(CLIPPY_BASE_FLAGS) \
		$(CLIPPY_STRICT_FLAGS)

# ── Security ──────────────────────────────────────────────────────────────────

audit: ## Security audit (cargo audit + deny)
	@echo "→ Running security audit..."
	@echo "  Installing security tools if needed..."
	@command -v cargo-audit >/dev/null 2>&1 || cargo install --locked cargo-audit
	@command -v cargo-deny >/dev/null 2>&1 || cargo install --locked cargo-deny
	@echo "  Checking for known vulnerabilities..."
	@$(CARGO) audit --deny unsound || echo "⚠️  Security issues found - review .cargo/audit.toml for justified exceptions"
	@echo "  Checking dependency policies..."
	@$(CARGO) deny check

security-scan: ## Run security scan (audit + dependency policy + shellcheck)
	@echo "→ Running comprehensive security scan..."
	$(MAKE) audit
	$(MAKE) shellcheck
	@echo "  Checking for outdated dependencies..."
	@command -v cargo-outdated >/dev/null 2>&1 || cargo install --locked cargo-outdated
	@$(CARGO) outdated --root-deps-only || true

security-all: ## Run all security checks (audit + policy + scan + SBOM)
	@echo "→ Running complete security audit suite..."
	$(MAKE) audit
	$(MAKE) shellcheck
	@echo "  Generating Software Bill of Materials..."
	@mkdir -p security/sbom
	@$(CARGO) tree --format "{p} {l}" > security/sbom/dependencies.txt
	@$(CARGO) deny list --format json > security/sbom/licenses.json 2>/dev/null || true
	@echo "  ✅ Security audit complete - SBOM available in security/sbom/"

security-fix: ## Report available dependency updates (dry run; applies no changes)
	@echo "→ Checking for available dependency updates (dry run)..."
	@$(CARGO) update --dry-run
	@echo "  ⚠️  Nothing was changed. Review the output above, then run 'cargo update' manually."

security-report: ## Generate comprehensive security report  
	@echo "→ Generating security report..."
	@mkdir -p security/reports
	@echo "# Security Report - $(shell date)" > security/reports/security-report.md
	@echo "" >> security/reports/security-report.md
	@echo "## Vulnerability Scan" >> security/reports/security-report.md
	@$(CARGO) audit --format json > security/reports/audit.json 2>/dev/null || true
	@echo "" >> security/reports/security-report.md  
	@echo "## Dependency Policy Check" >> security/reports/security-report.md
	@$(CARGO) deny check --format json > security/reports/deny.json 2>/dev/null || true
	@echo "" >> security/reports/security-report.md
	@echo "## License Compliance" >> security/reports/security-report.md
	@$(CARGO) deny list >> security/reports/security-report.md 2>/dev/null || true
	@echo "  📊 Security report generated in security/reports/"

shellcheck: ## Run shellcheck on all shell scripts
	@echo "→ Running shellcheck..."
	@find scripts -type f -name "*.sh" -print0 | xargs -0 shellcheck -S error || true

security-check: ## Run the standalone security-check.sh audit script
	@bash scripts/security-check.sh

# ── Test & Build ──────────────────────────────────────────────────────────────

test: ## Run tests
	@echo "→ Running tests..."
	@$(CARGO) test --workspace --features $(CLIPPY_FEATURES) --tests --lib --bins --verbose
	@echo "→ Running doc tests..."
	@$(CARGO) test --doc --workspace --features $(CLIPPY_FEATURES)

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

docker-multiarch: ## Build the multi-arch (linux/amd64 + linux/arm64) image locally via buildx
	@echo "→ Building multi-arch image for linux/amd64,linux/arm64..."
	@$(DOCKER) buildx version >/dev/null 2>&1 || { \
		echo "✗ docker buildx not available. Install the buildx builder plugin."; \
		exit 1; \
	}
	DOCKER_BUILDKIT=1 $(DOCKER) buildx build \
		--platform linux/amd64,linux/arm64 \
		--target runtime-local \
		-t $(IMAGE_NAME):$(IMAGE_TAG) .

# CI publishes the multi-arch image from the `container` job in
# .github/workflows/release.yml (QEMU + buildx). The `make docker-multiarch`
# target above is the local equivalent.

health: ## Run common repository health checks (format, lint, test, docs, links)
	@bash scripts/repo-health.sh

health-fast: ## Fast health gate (format, lint, compile only)
	@bash scripts/repo-health.sh --fast

validate: health-fast ## Fast validation (alias for health-fast)

# ── Quality & Health ───────────────────────────────────────────────────────────

link-check: ## Check markdown links (internal anchors + relative paths)
	@echo "→ Running markdown link checker..."
	@python3 scripts/check-links.py

link-check-all: ## Repo-wide link check (markdown + source + configs) via lychee
	@echo "→ Running repo-wide link checker (lychee)..."
	@command -v lychee >/dev/null 2>&1 || { \
		echo "lychee not found. Install with: cargo install lychee --locked"; \
		exit 1; \
	}
	@lychee --config lychee.toml --no-progress --cache \
		'./**/*.md' './**/*.rs' './**/*.toml' \
		'./**/*.yaml' './**/*.yml' './**/*.sh' './**/*.html'

changelog: ## Generate/update CHANGELOG.md using git-cliff
	@echo "→ Generating changelog..."
	@command -v git-cliff >/dev/null 2>&1 || cargo install git-cliff
	git-cliff --output CHANGELOG.md

ci-local: fmt-check lint docs-lint audit test build link-check ## Run full CI locally (includes strict docs lint)
	@echo ""
	@echo "✓ All CI checks passed!"

third-party-licenses: ## Regenerate THIRD_PARTY_LICENSES.md from Cargo dependency tree
	@bash scripts/generate-third-party-licenses.sh

check-third-party-licenses: ## Verify THIRD_PARTY_LICENSES.md is up to date (used in CI)
	@bash scripts/generate-third-party-licenses.sh --check

quick: fmt-check ## Quick pre-commit check
	@$(CARGO) check --workspace
	@echo "✓ Quick checks passed"

pre-commit: ## Run pre-commit hooks manually
	@echo "→ Running pre-commit hooks..."
	@command -v pre-commit >/dev/null 2>&1 || (echo "✗ pre-commit not installed. Run: make dev-setup" && exit 1)
	@pre-commit run --all-files

pre-commit-install: dev-setup-hooks ## Install pre-commit hooks (alias for dev-setup-hooks)
	@echo "✓ pre-commit hooks installed"

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

check-stale-docs: ## Check for documentation that has fallen behind source code (warns; use docs-check-strict to fail)
	@echo "→ Checking for stale documentation..."
	@$(CARGO) run --bin doc-check -- --warn-only

update-doc-baseline: ## Update the .doc-hashes.toml baseline after deliberate doc updates
	@echo "→ Updating doc-check baseline hashes..."
	@$(CARGO) run --bin doc-check -- --update-baseline
	@echo "✓ Baseline updated. Commit .doc-hashes.toml to record the new state."

list-doc-coverage: ## Print all doc → source coverage mappings from doc-coverage.toml
	@$(CARGO) run --bin doc-check -- list

docs-check-strict: ## Fail CI if any doc is stale (no --warn-only; used in strict CI stages)
	@echo "→ Running strict documentation staleness check..."
	@$(CARGO) run --bin doc-check -- status

docs-lint: ## Run rustdoc with warnings-as-errors (issue #1138: strict docs quality gate)
	@echo "→ Running cargo doc with RUSTDOCFLAGS=-D warnings..."
	@RUSTDOCFLAGS="-D warnings" K8S_OPENAPI_ENABLED_VERSION=1.30 \
		$(CARGO) doc --no-deps --workspace \
		--features "rest-api,metrics,admission-webhook,k8s-v1-30"
	@echo "✓ rustdoc passed — no documentation warnings"

# ── Kubernetes ────────────────────────────────────────────────────────────────

install-crd: ## Install CRDs
	$(KUBECTL) apply -f config/crd/stellarnode-crd.yaml

apply-samples: install-crd ## Apply samples
	$(KUBECTL) apply -f config/samples/

crd-gen: ## Generate CRDs (output is sorted for deterministic diffs)
	@echo "→ Generating CRDs..."
	@$(CARGO) run --bin crdgen | python3 scripts/sort-manifests.py > config/crd/stellarnode-crd.yaml
	@echo "✓ CRD written to config/crd/stellarnode-crd.yaml (deterministic order)"

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

test-repo-health: ## Run bats unit tests for scripts/repo-health.sh
	@echo "→ Running repo-health bats tests..."
	@command -v bats >/dev/null 2>&1 || (echo "✗ bats not installed. See https://github.com/bats-core/bats-core" && exit 1)
	@bats scripts/tests/repo-health.bats

# ── Completions ────────────────────────────────────────────────────────────────

completions: completions-bash completions-zsh completions-fish ## Generate all shell completion scripts

completions-bash: ## Generate bash completion script
	@echo "→ Generating bash completions..."
	@mkdir -p completions
	@$(CARGO) run --bin stellar-completions completions bash > completions/stellar-operator.bash
	@echo "✓ Bash completions generated: completions/stellar-operator.bash"

completions-zsh: ## Generate zsh completion script
	@echo "→ Generating zsh completions..."
	@mkdir -p completions
	@$(CARGO) run --bin stellar-completions completions zsh > completions/_stellar-operator
	@echo "✓ Zsh completions generated: completions/_stellar-operator"

completions-fish: ## Generate fish completion script
	@echo "→ Generating fish completions..."
	@mkdir -p completions
	@$(CARGO) run --bin stellar-completions completions fish > completions/stellar-operator.fish
	@echo "✓ Fish completions generated: completions/stellar-operator.fish"

# ── Helm ──────────────────────────────────────────────────────────────────────

helm-lint: ## Helm lint check
	@echo "→ Linting Helm charts..."
	helm lint charts/stellar-operator --strict
	@echo "→ Validating Helm template rendering..."
	helm template stellar-operator charts/stellar-operator > /dev/null
	@echo "✓ Helm charts passed linting and validation"

# ── Development Setup ─────────────────────────────────────────────────────────

dev-setup: dev-setup-rust dev-setup-tools dev-setup-hooks ## Setup dev environment

dev-setup-rust: ## Install Rust toolchain and components
	@echo "→ Setting up Rust toolchain..."
	rustup update stable
	rustup default stable
	rustup component add clippy rustfmt

dev-setup-tools: ## Install development tools
	@echo "→ Installing development tools..."
	cargo install cargo-audit cargo-watch

dev-setup-hooks: ## Install git hooks
	@echo "→ Installing git hooks..."
	@command -v pre-commit >/dev/null 2>&1 || pip install pre-commit
	pre-commit install
	pre-commit install --hook-type pre-push

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
	RUST_LOG=info ./target/release/stellar-operator run

run: run-local ## Run the operator (alias for run-local; matches README and CI references)

run-dev: ## Run operator in dev mode with hot reload
	RUST_LOG=debug cargo watch -x run


# ── Bundle ────────────────────────────────────────────────────────────────────

bundle: bundle-render bundle-generate bundle-validate ## Generate bundle manifests and metadata, then validate

bundle-render: ## Render Helm chart to manifests (sorted for deterministic pipeline diffs)
	@echo "→ Generating manifests from Helm chart..."
	@mkdir -p rendered
	@helm template stellar-operator charts/stellar-operator \
		| python3 scripts/sort-manifests.py > rendered/manifests.yaml
	@echo "✓ Rendered manifests written to rendered/manifests.yaml (deterministic order)"

bundle-generate: ## Generate OLM bundle from manifests
	@echo "→ Generating bundle..."
	@operator-sdk generate kustomize manifests -q
	@kustomize build config/manifests | operator-sdk generate bundle -q --overwrite --version $(VERSION) --channels $(CHANNELS) --default-channel $(DEFAULT_CHANNEL)

bundle-validate: ## Validate generated bundle
	@echo "→ Validating bundle..."
	@operator-sdk bundle validate ./bundle
	@rm -rf rendered

bundle-build: ## Build the bundle image.
	docker build -f bundle.Dockerfile -t $(BUNDLE_IMG) .

sort-manifests: ## Sort and normalise a YAML manifest stream (reads stdin, writes stdout)
	@python3 scripts/sort-manifests.py

# ── Quickstart ────────────────────────────────────────────────────────────────

quickstart: quickstart-setup quickstart-build quickstart-deploy ## End-to-end local quickstart

quickstart-setup: ## Create kind cluster and check prerequisites
	@echo "→ Checking prerequisites..."
	@command -v kind >/dev/null 2>&1 || (echo "✗ kind not found. Install: https://kind.sigs.k8s.io/docs/user/quick-start/#installation" && exit 1)
	@command -v kubectl >/dev/null 2>&1 || (echo "✗ kubectl not found. Install: https://kubernetes.io/docs/tasks/tools/" && exit 1)
	@command -v helm >/dev/null 2>&1 || (echo "✗ helm not found. Install: https://helm.sh/docs/intro/install/" && exit 1)
	@echo "→ Creating kind cluster 'stellar-dev'..."
	@kind create cluster --name stellar-dev --wait 120s || echo "  (cluster may already exist, continuing)"

quickstart-build: ## Build and load operator image into kind
	@echo "→ Building operator image..."
	@$(MAKE) build
	@DOCKER_BUILDKIT=1 $(DOCKER) build --target runtime-local -t stellar-operator:dev .
	@echo "→ Loading image into kind cluster..."
	@kind load docker-image stellar-operator:dev --name stellar-dev

quickstart-deploy: ## Deploy operator and sample resources
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