# Scripts Reference

Developer and CI helper scripts live in [`scripts/`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/scripts).
CI-only helpers are in `scripts/ci/`, shared shell libraries in `scripts/lib/`,
and the bats / unittest suites that cover them in `scripts/tests/`.

This page mirrors the index in
[`scripts/README.md`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/README.md);
update both when a script is added, renamed or removed. When a script and a
`make` target both exist, prefer the `make` target: it pins the workspace feature flags
so the result matches CI.
Developer and CI helper scripts live in `scripts/`. For a comprehensive directory
index of all available scripts, their usage, and coverage, see the
[Operational Scripts Index](../../scripts/README.md).

Library helpers shared by several scripts are in `scripts/lib/` (`common.sh`,
`errors.sh`, `health-steps.sh`, `batch.sh`). Bats tests for the shell helpers
are in `scripts/tests/`.

Run the standard gates locally before pushing:

```bash
bash scripts/preflight.sh        # required binaries and versions
make health-fast                 # fmt + clippy + compile
make health                      # full gate incl. docs and links
make ci-local                    # fmt-check lint audit test build
make docs-ci-build               # strict MkDocs build, same as the docs CI job
```

## Conventions

- Shell scripts use `kebab-case.sh`; Python tools use `kebab-case.py`
  (`issue_template_lint.py`, `crd_migration_lint.py`, and `check_mkdocs_nav.py`
  predate the rule and are kept for compatibility).
- Every shell script must pass `shellcheck -S error` (`make shellcheck`) and
  `scripts/check-shell-safety.py` (`make shell-safety`).
- Anything in [`scripts/ci/`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/scripts/ci) is **CI-only**: those scripts are designed to
  run inside a GitHub Actions runner and have no `make` target or local wrapper.
  They are listed separately below and marked `CI-only`.
- Tests live in [`scripts/tests/`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/scripts/tests): `*.bats` for shell scripts and
  `test_*.py` (unittest) for Python tools. The [Test suites](#test-suites-scriptstests)
  table below lists what each suite covers.
- Scripts that need a live cluster (kind/minikube), or network access, are
  marked accordingly. Do not run them as part of a fast local gate.

## Repository health and developer setup

| Script | Purpose | Invocation |
| --- | --- | --- |
| [`setup-dev-env.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/setup-dev-env.sh) | OS dispatcher: runs `setup-linux.sh` on Linux, `setup-mac.sh` on macOS, otherwise errors. | `./scripts/setup-dev-env.sh` |
| [`setup-linux.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/setup-linux.sh) | Idempotent Linux bootstrap (Ubuntu/Debian/Fedora): Rust, Docker deps, kubectl, kind, helm, tools. | `./scripts/setup-linux.sh` |
| [`setup-mac.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/setup-mac.sh) | Idempotent macOS bootstrap via Homebrew. | `./scripts/setup-mac.sh` |
| [`preflight.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/preflight.sh) | Verifies required tools are installed at the pinned minimum versions in `lib/versions.sh`; `--labels` also verifies GitHub repo labels. | `make preflight`, `./scripts/preflight.sh [--labels]` |
| [`health-check.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/health-check.sh) | Environment health check for installed components (`--json`, `--fix`, `--outdated-only`). | `make health-check` / `make health-check-json` / `make health-check-fix` |
| [`repo-health.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/repo-health.sh) | Single entry point for repository health gates: fmt, clippy, test, API docs, shellcheck, issue templates, links, audit, helm. | `make health`, `make health-fast`, `bash scripts/repo-health.sh [--fast |
| [`cleanup.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/cleanup.sh) | The single supported cleanup entrypoint: removes scratch artifacts and fails if obsolete archive helpers reappear. | `make cleanup`, `make cleanup DRY_RUN=1`, `./scripts/cleanup.sh [--dry-run]` |
| [`dep-gate.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/dep-gate.sh) | Consolidated dependency gate: `cargo audit` + `cargo deny` + license and build checks. | `make audit` (`bash scripts/dep-gate.sh`), `bash scripts/dep-gate.sh --quick`, `--audit-only` |
| [`generate-third-party-licenses.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/generate-third-party-licenses.sh) | Regenerates `THIRD_PARTY_LICENSES.md` from the Cargo dependency tree; `--check` fails when the file is stale. | `make third-party-licenses`, `make check-third-party-licenses` |
| [`create-argocd-branch.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/create-argocd-branch.sh) | One-off helper that creates a branch, commits the Argo CD GitOps docs/examples, and pushes to `origin` or the given remote URL. | `./scripts/create-argocd-branch.sh [remote-url] [branch-name]` |

## Validation and linting

| Script | Purpose | Invocation |
| --- | --- | --- |
| [`issue_template_lint.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/issue_template_lint.py) | Lints GitHub Issue Forms in `.github/ISSUE_TEMPLATE/`: required keys, `body` field types, and `config.yml` shape. | `make issue-template-lint` (`python3 scripts/issue_template_lint.py`) |
| [`check-shell-safety.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-shell-safety.py) | Static analysis gate for unsafe shell patterns (unguarded `rm -rf`, `eval`, `curl \| bash`, missing strict mode). | `make shell-safety`, `make test-shell-safety` |
| [`check-secrets.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-secrets.sh) | Secret-handling audit across the repository; `--report` never fails. | `bash scripts/check-secrets.sh [--report]` |
| [`trivy-fs.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/trivy-fs.sh) | Local Trivy filesystem + IaC scan mirroring the `vulnerability-scan.yml` CI jobs; advisory only, never fails the build. Needs `trivy`. | `bash scripts/trivy-fs.sh [--severity HIGH,CRITICAL] [--fs-only\|--config-only]` |
| [`validate-yaml-manifests.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/validate-yaml-manifests.py) | Repository-wide schema validation of YAML manifests (syntax + Kubernetes/CRD schema + waivers). | `make validate-yaml`, `make test-yaml-validation` |
| [`validate-k8s-manifests.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/validate-k8s-manifests.py) | kubeconform-based validation of `config/`, `examples/`, `bundle/`, and Helm renders against locally generated CRD JSON schemas. | `python3 scripts/validate-k8s-manifests.py` (runs from the `k8s-manifest-crd-validation` pre-commit hook) |
| [`check-stale-samples.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-stale-samples.sh) | Detects (and can fix) sample manifests under `config/samples/` that are invalid or out of sync with the CRD schema. | `bash scripts/check-stale-samples.sh` |
| [`check-helm-drift.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-helm-drift.sh) | Detects drift between Helm templates and the committed renders in `charts/stellar-operator/rendered/`; `--update` regenerates the goldens. | `make helm-drift`, `make helm-drift-update`, `make test-helm-drift` |
| [`crd_migration_lint.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/crd_migration_lint.py) | Backward-compatibility linter for CRD evolution: flags removed fields, type changes, and version changes. | `python3 scripts/crd_migration_lint.py [--against REF] [--crd-dir DIR]` |
| [`check-crd-compatibility.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-crd-compatibility.sh) | Shell CRD compatibility gate. **Local/ad-hoc only** — the canonical PR gate is `crd_migration_lint.py` in `quickstart-validation.yml`. | `bash scripts/check-crd-compatibility.sh` |
| [`check-license-headers.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-license-headers.py) | Enforces the Apache-2.0 header on Rust/Shell/YAML files; `--fix` inserts missing headers, `--report` never fails. | `make license-headers` (`python3 scripts/check-license-headers.py`) |
| [`check-links.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-links.py) | Checks Markdown links (anchors and relative paths); optional `--check-external`, `--dir`, `--exclude`. CI uses [lychee](https://github.com/OtowoOrg/Stellar-K8s/blob/main/lychee.toml) instead. | `make link-check` (`python3 scripts/check-links.py`) |
| [`check-unreachable-modules.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-unreachable-modules.sh) | Static check for unreachable modules and dead code paths; `--report`, `--warn-only`, `--strict-dead-paths`. | `make check-unreachable-modules` |
| [`check-pipeline-log-redaction.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-pipeline-log-redaction.sh) | Ensures secrets are redacted from pipeline command logs; `--report`, `--fixture`, `--scrub`. | `make check-pipeline-log-redaction` |
| [`audit-features.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/audit-features.sh) | Audits unused crate features, implicit feature propagation, and dead imports/code. | `./scripts/audit-features.sh [--report]` |
| [`dead-code-report.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/dead-code-report.sh) | Informational dead-code and unused-config report written to `target/reports/dead-code-report.md`; always exits 0. `SKIP_CARGO=1` skips the compiler pass. | `SKIP_CARGO=1 ./scripts/dead-code-report.sh` |
| [`semver_gate.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/semver_gate.py) | Release SemVer gate for charts, images, and CRDs. Subcommands: `required-bump`, `check --version X`. | `python3 scripts/semver_gate.py required-bump [--base REF]`, `python3 scripts/semver_gate.py check --version X [--base REF]` |
| [`golden-path-pipeline-check.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/golden-path-pipeline-check.sh) | Runs the full canonical command sequence (fmt, clippy, issue templates, API docs, helm, release gate, quickstart golden path) from a clean checkout. Needs a full toolchain. | `bash scripts/golden-path-pipeline-check.sh` |
| [`check_mkdocs_nav.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check_mkdocs_nav.py) | Fails when a local `mkdocs.yml` nav entry points to a missing file under `docs/`. Needs PyYAML. | `make nav-coverage`, `python3 scripts/check_mkdocs_nav.py [mkdocs.yml]` |
| [`validate-metrics.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/validate-metrics.sh) | Cross-checks metric names registered in `src/controller/metrics.rs` against [`docs/observability/metric-reference.md`](../observability/metric-reference.md). | `./scripts/validate-metrics.sh` |
| [`validate-crd-dryrun.mjs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/validate-crd-dryrun.mjs) | Generates a 3-zone topology manifest from the frontend builder, checks it against the StellarNode CRD, and runs `kubectl apply --dry-run=client`. Needs Node.js and `kubectl`. | `node scripts/validate-crd-dryrun.mjs` |
| `repo-health.sh` | Single entry point for repo health checks (`--fast`, `--with-audit`, `--with-links`, `--with-helm`) | `bash scripts/repo-health.sh`; `make health` / `make health-fast` / `make health-full` |
| `validate.sh` | Fast local validation (delegates to `repo-health.sh --fast`) | `bash scripts/validate.sh`; `make validate` |
| `golden-path-pipeline-check.sh` | Golden-path full pipeline command test from a clean checkout (fmt, clippy, issue templates, API docs, helm, release gate, quickstart) | `bash scripts/golden-path-pipeline-check.sh`; `make golden-path-pipeline` |
| `release-gate.sh` | Release validation mirroring `RELEASE_CHECKLIST.md` | `VERSION=1.2.0 bash scripts/release-gate.sh` |
| `check-links.py` | Internal markdown file+anchor link check | `python3 scripts/check-links.py`; `make link-check`; `ci.yml` docs-link-check |
| `check-stale-samples.sh` | Flags sample manifests that drifted from sources | `ci.yml` |
| `crd_migration_lint.py` | Lints CRD migrations | `python3 scripts/crd_migration_lint.py --against origin/main`; `quickstart-validation.yml` |

## Code generation and documentation

| Script | Purpose | Invocation |
| --- | --- | --- |
| [`generate-api-docs.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/generate-api-docs.py) | Generates `docs/api-reference.md` from the CRD OpenAPI schema; `--check` exits 1 when the docs are stale. | `make generate-api-docs`, `make check-api-docs` |
| [`generate-openapi-spec.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/generate-openapi-spec.py) | Validates `docs/api/openapi.yaml` and asserts that every operator REST route is documented; `--check` is the CI drift gate. | `make generate-openapi-spec`, `make check-openapi-spec` |
| [`generate-observability-instrumentation.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/generate-observability-instrumentation.py) | Generates the observability instrumentation Rust source for a service from its contract. | `python3 scripts/generate-observability-instrumentation.py --service NAME --out FILE` |
| [`sort-manifests.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/sort-manifests.py) | Deterministic YAML manifest sorter (recursive key sort, documents ordered by kind/namespace/name). Reads stdin or a file argument. | `make crd-gen`, `make bundle-render`, `python3 scripts/sort-manifests.py FILE`, `helm template ... \| python3 scripts/sort-manifests.py` |
| [`update-cap-matrix.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/update-cap-matrix.py) | Refreshes the stellar-core release/CAP matrix in `config/caps.toml` from GitHub releases. Needs network access. | `python3 scripts/update-cap-matrix.py` (scheduled by `cap-matrix-update.yml`) |
| `check-secrets.sh` | Secret-handling checks for CI/runtime command paths | `bash scripts/check-secrets.sh`; `ci.yml` |
| `secret-rotation-check.sh` | Verifies secret rotation without downtime (`--dry-run` for CI) | `make secret-rotation-check [DRY_RUN=1]`; `quickstart-validation.yml` |
| `audit-features.sh` | Static audit for unused crate features and dead imports (`--report`) | `bash scripts/audit-features.sh --report`; `ci.yml` |
| `dead-code-report.sh` | Periodic dead-code report to `target/reports/dead-code-report.md` (always exits 0) | `make dead-code-report`; `dead-code-report.yml` |
| `generate-third-party-licenses.sh` | Regenerates `THIRD_PARTY_LICENSES.md` (requires `cargo-license`) | `make third-party-licenses`; `make check-third-party-licenses` in CI |

## Build, release, and packaging

| Script | Purpose | Invocation |
| --- | --- | --- |
| [`bump-chart-version.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/bump-chart-version.sh) | Derives a SemVer chart bump from conventional commits since the last `chart-v*` tag, honouring the `versioning.min-bump` floor. Supports `--dry-run`, `--since REF`, `--bump-override`, `--chart-path`, `--output-env`. | `bash scripts/bump-chart-version.sh [OPTIONS]`, `make test-helm-bump` |
| [`generate-release-notes.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/generate-release-notes.sh) | Previews the GitHub Release notes the `release` job will publish (same `git-cliff --latest --strip header` output). Needs `git-cliff`. | `bash scripts/generate-release-notes.sh [--output FILE]` |
| [`tag-release.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tag-release.sh) | Creates an annotated `vX.Y.Z[-pre]` tag per [`docs/release-process.md`](../release-process.md): version must match `Cargo.toml`, main only, clean tree. | `make tag-release TAG=vX.Y.Z [PUSH=1] [DRY_RUN=1]`, `bash scripts/tag-release.sh <tag> [--push] [--dry-run]` |
| [`check-api-contract.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-api-contract.py) | OpenAPI contract validation, endpoint coverage, and breaking-change detection. Subcommands: `check`, `coverage`, `breaking`. | `make check-api-contract`, `make check-api-coverage`, `make check-breaking-changes` |
| [`benchmark-crd-validation.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/benchmark-crd-validation.py) | CRD validation performance benchmark producing the baseline JSON. | `make benchmark-crd` |
| [`benchmark-api.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/benchmark-api.py) | Operator REST API throughput benchmark. Requires a running operator. | `make benchmark-api` |
| [`benchmark-helm.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/benchmark-helm.sh) | Helm rendering performance benchmark against a stored baseline. | `make benchmark-helm` |
| [`bench-helm-render.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/bench-helm-render.sh) | Times `helm template` for CI performance tracking (`--iterations N`, `--output FILE`). | `bash scripts/bench-helm-render.sh --output results/helm-render-benchmark.json` |
| [`check-benchmark-sanity.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-benchmark-sanity.sh) | Reproducible benchmark sanity check: runs a quick suite and compares it to stored baselines. | `bash scripts/check-benchmark-sanity.sh` |
| [`check-crd-performance.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-crd-performance.py) | Fails when CRD validation performance regresses beyond `--threshold` percent against a baseline. | `python3 scripts/check-crd-performance.py --current FILE --baseline FILE [--threshold N]` |
| [`collect-criterion-results.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/collect-criterion-results.py) | Converts `cargo bench` criterion output into the repository baseline JSON shape. | `python3 scripts/collect-criterion-results.py --criterion-dir target/criterion --output results/crd-benchmark.json` |

## Operations and incident response

| Script | Purpose | Invocation |
| --- | --- | --- |
| [`health-check.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/health-check.sh) | See [Repository health](#repository-health-and-developer-setup). | `make health-check` |
| [`quickstart-verify.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/quickstart-verify.sh) | End-to-end quickstart verification against a real kind cluster (creates and destroys it unless `SKIP_CLEANUP=1`). | `bash scripts/quickstart-verify.sh` (`CLUSTER_NAME=`, `SKIP_CLEANUP=`) |
| [`quickstart-golden-path.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/quickstart-golden-path.sh) | Validates the documented quickstart path without a cluster: entry points exist, scripts parse, sample manifests are valid YAML. | `./scripts/quickstart-golden-path.sh` |
| [`secret-rotation-check.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/secret-rotation-check.sh) | Checks operator secret rotation readiness. `--dry-run` validates arguments and prints the plan without a cluster. | `./scripts/secret-rotation-check.sh [--namespace NS] [--secret NAME] [--deployment NAME] [--window SECONDS] [--dry-run]` |
| [`migrate-seed-key.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/migrate-seed-key.sh) | Adds the standardized `STELLAR_CORE_SEED` key to secrets that only have the legacy `SEED` key (the legacy key is kept); never prints secret values. Needs `kubectl`. | `scripts/migrate-seed-key.sh [--dry-run] <namespace> <secret-name>`, `scripts/migrate-seed-key.sh [--dry-run] --all <namespace>` |
| [`verify-mtls.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/verify-mtls.sh) | Verifies mTLS Certificates/Secrets and rotation readiness in `NAMESPACE` (default `stellar-system`). Needs `kubectl` and a cluster. | `NAMESPACE=stellar-system bash scripts/verify-mtls.sh` |
| [`sync-federation-secrets.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/sync-federation-secrets.sh) | Copies a federation secret from a source cluster/namespace to a target cluster/namespace. Needs `kubectl` access to both clusters. | `./scripts/sync-federation-secrets.sh --source-cluster SRC --target-cluster DST --name NAME --namespace NS [--dry-run]` |
| [`soak-test.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/soak-test.sh) | Long-running stability test: samples operator memory over `SOAK_DURATION` seconds and fails on RSS growth beyond the threshold. Needs a cluster. | `OPERATOR_NAMESPACE=stellar-system SOAK_DURATION=3600 ./scripts/soak-test.sh` |
| [`run-chaos-drill.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/run-chaos-drill.sh) | Runs a single chaos drill (`DRILL_TYPE`, `DURATION`, `TARGET`, `NAMESPACE`, or the same values as positional arguments) and writes a JSON result. Needs a cluster. | `./scripts/run-chaos-drill.sh node-kill 60 validator stellar-chaos` |
| [`aggregate-chaos-results.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/aggregate-chaos-results.sh) | Aggregates recorded drill results into `RESULTS.md`; exits non-zero if a drill missed its RTO target. | `./scripts/aggregate-chaos-results.sh [results-dir]` |

## CI-only scripts (`scripts/ci/`)

| Script | Purpose | Invoked by |
| --- | --- | --- |
| [`ci/check-cache-keys.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/check-cache-keys.sh) | Verifies workflow cache keys follow the naming/scope conventions. | `ci.yml` → `repo-hygiene` |
| [`ci/validate-config-samples.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/validate-config-samples.sh) | kubeconform validation of `config/samples/`. | `ci.yml` → `repo-hygiene` |
| [`ci/check-stale-todos.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/check-stale-todos.sh) | Fails on stale or unlinked issue references in work-item comments under critical paths. | `ci.yml` → `repo-hygiene` |
| [`ci/extract-crd-json-schemas.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/extract-crd-json-schemas.py) | Derives `schemas/crd/*.json` from the repository's own CRDs; `--check` fails on drift. | `ci.yml` → `repo-hygiene` |
| [`ci/lint-observability-contract.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/lint-observability-contract.py) | Blocks resource attributes that are absent from the published observability JSON Schema. | `ci.yml` → `lint` (`make lint-observability-contract`) |
| [`ci/validate-yaml.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/validate-yaml.sh) | yamllint + CRD JSON-schema drift + kubeconform on Helm-rendered manifests. | `ci.yml` → `yaml-schema` (`make yaml-schema-validate`) |
| [`ci/lint.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/lint.sh) | Unified lint runner: shellcheck, shell-safety, scripts-convention, yamllint, and optionally Rust lints. Flags: `--fast` (skip Rust), `--rust-only`. | `ci.yml` → `scripts-convention` (`make lint-all`) |
| [`ci/check-scripts-convention.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/check-scripts-convention.sh) | Enforces CONVENTIONS.md §Shell scripts: kebab-case naming, no root-level scripts, Usage: headers, and lib/ placement rules. `--report` never fails. | `ci.yml` → `scripts-convention` (`make scripts-convention-check`) |
| [`ci/test-db-migrations.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/test-db-migrations.sh) | Forward/rollback SQL migration harness. Requires a `DATABASE_URL` pointing at an isolated Postgres instance. | `ci.yml` → `db-migrations` (`make test-db-migrations`) |
| [`ci/helm-upgrade-test.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/helm-upgrade-test.sh) | Asserts that Helm upgrade overrides preserve resources, selectors, affinity, and ports. | `ci.yml` → `helm-test` (`make helm-upgrade-test`) |
| [`ci/k8s-compat-smoke-tests.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/k8s-compat-smoke-tests.sh) | Kubernetes version compatibility smoke tests for a given `TARGET_VERSION`. | `k8s-compat.yml` |
| [`ci/generate-badge.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/generate-badge.sh) | Renders an SVG compatibility badge from a `"passed/total"` string. | `k8s-compat-matrix-advanced.yml` |
| [`ci/collect-failure-diagnostics.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/ci/collect-failure-diagnostics.sh) | Assembles the unified diagnostics bundle (`manifest.json`, `summary.txt`, sanitized env, cluster dumps, extras) for failing runs. | `ci.yml` → `failure-diagnostics`; see [`docs/ci-failure-diagnostics.md`](../ci-failure-diagnostics.md) |

## Shared shell libraries (`scripts/lib/`)

| Library | Purpose |
| --- | --- |
| [`lib/errors.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/lib/errors.sh) | Step-aware diagnostics helpers (`sk8s_step`, `sk8s_fail`) with consistent `[step] detail` messages. |
| [`lib/health-steps.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/lib/health-steps.sh) | Shared health-check steps and the pinned clippy feature/flag sets used by `repo-health.sh`. Includes `sk8s_health_issue_templates`, which runs `issue_template_lint.py`. |
| [`lib/versions.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/lib/versions.sh) | Single source of truth for pinned minimum tool versions (Rust, kind, kubectl, Helm). Bump here and every consumer picks it up. |

## Test suites (`scripts/tests/`)

| Suite | Covers |
| --- | --- |
| [`tests/preflight.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/preflight.bats) | `preflight.sh` tool-presence and version-pin checks |
| [`tests/cleanup.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/cleanup.bats) | `cleanup.sh` (the single supported cleanup tool) |
| [`tests/dep-gate.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/dep-gate.bats) | `dep-gate.sh` dependency gate |
| [`tests/dead-code-report.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/dead-code-report.bats) | `dead-code-report.sh` report generation |
| [`tests/helm-drift.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/helm-drift.bats) | `check-helm-drift.sh` (requires `helm` and `python3`) |
| [`tests/bump-chart-version.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/bump-chart-version.bats) | `bump-chart-version.sh` SemVer rules and `min-bump` floor |
| [`tests/secret-rotation-check.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/secret-rotation-check.bats) | `secret-rotation-check.sh` non-destructive secret rotation verification |
| [`tests/failure-diagnostics.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/failure-diagnostics.bats) | `ci/collect-failure-diagnostics.sh` bundle layout |
| [`tests/test_check_shell_safety.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/test_check_shell_safety.py) | `check-shell-safety.py` rules (both positive and negative cases) |
| [`tests/test_validate_yaml_manifests.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/test_validate_yaml_manifests.py) | `validate-yaml-manifests.py` validation layers |
| [`tests/test_crd_migration_lint.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/test_crd_migration_lint.py) | `crd_migration_lint.py` backward-compatibility rules |
| [`tests/test_semver_gate.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/test_semver_gate.py) | `semver_gate.py` required-bump and release checks |
| [`tests/test_lint_observability_contract.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/test_lint_observability_contract.py) | `ci/lint-observability-contract.py` attribute gate |

## Adding a script

1. Place operational scripts in `scripts/`; CI-only helpers go in `scripts/ci/`
   and shared shell code in `scripts/lib/`.
2. Add a `Usage:` block to the script header and wire a `make` target if the
   script is meant to be run by contributors.
3. Add a `scripts/tests/<script-name>.bats` or `scripts/tests/test_<script>.py`
   suite and reference it from the test suites table.
4. Add a row to the relevant table above and in `scripts/README.md`, then run
   `make shellcheck`, `make shell-safety`, and `make test-bats` before pushing.
