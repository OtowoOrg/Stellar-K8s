# CI Pipeline Architecture & Reliability Guide

## Overview

This document describes the optimized CI/CD pipeline architecture.  It covers
the original cleanup wave (issues #700, #701, #703, #714) as well as the
follow-up hardening wave (issues #1136, #1137, #1138, #1139).

---

## Shared Composite Actions

All reusable logic lives under `.github/actions/`:

| Action | Purpose |
|--------|---------|
| `setup-rust` | Install Rust toolchain + system deps + Swatinem cache |
| `setup-kind-cluster` | Provision kind cluster, load image, install CRDs, deploy operator |
| `collect-e2e-logs` | Dump operator logs, K8s events, StellarNode status → artifact |
| `setup-perf-env` | Install k6/kind/kubectl, create cluster, deploy operator with RBAC, port-forward |
| `build-operator` | Build Rust binary + Docker image + artifact upload in one call (issue #1136) |

---

## Hardening Wave: Issues #1136–#1139

### #1136 — Consolidate duplicated command bootstrap across CI workflows

The `chaos-tests`, `soak-test`, `performance`, and `verify-operator-boot`
workflows previously each contained their own copy of:

```
setup-rust → cargo build --release → docker build → docker save → upload-artifact
```

This is now consolidated into `.github/actions/build-operator/action.yml`.
Each workflow calls the composite action with the appropriate `image-tag`,
`cache-key`, and optional `binary-only` / `upload-artifact` flags.

Additionally, `stale-docs.yml` previously had its own `dtolnay/rust-toolchain`
install + manual `actions/cache` block.  It now uses `setup-rust` for
consistency.

**Verification:** grep for `dtolnay/rust-toolchain` outside of
`.github/actions/setup-rust/action.yml` — the only remaining hit should be
`release.yml` (cross-compilation matrix targets require direct toolchain
installation per platform).

### #1137 — Enforce command parity between README, Makefile, and CI jobs

Missing Makefile targets that were declared in `.PHONY` but had no recipe
body:

| Target | Fix |
|--------|-----|
| `docker-multiarch` | Recipe builds a real multi-arch image locally via `docker buildx` (it previously dispatched a `multiarch-build.yml` workflow that no longer exists — see #935) |
| `run` | Added recipe as a documented alias for `run-local` (matches README references) |
| `update-doc-baseline` | New target to run `doc-check --update-baseline` |
| `docs-check-strict` | New target that runs `doc-check status` without `--warn-only` (hard fail) |
| `docs-lint` | New target that runs `cargo doc` with `RUSTDOCFLAGS="-D warnings"` |
| `sort-manifests` | New target that invokes `scripts/sort-manifests.py` on stdin |

**Verification:** `make help` — all targets declared in `.PHONY` now have a
corresponding recipe and description.

### #1138 — Add strict failure-on-warning policy for Rust lint and docs stages

Two changes enforce a zero-tolerance warning policy:

1. **`ci.yml` lint job** — a new "Check rustdoc (warnings as errors)" step runs
   `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace …` immediately
   after the existing clippy steps.  A missing or malformed doc comment now
   fails CI.

2. **`docs-deploy.yml`** — removed the `continue-on-error: true` guard on the
   `cargo doc` step and added `RUSTDOCFLAGS="-D warnings"`.  Broken docs can no
   longer silently pass and be published.

3. **Makefile `ci-local`** — `docs-lint` is now part of the local CI pipeline
   so contributors catch doc warnings before pushing.

**Verification:**
```bash
# Local check
make docs-lint

# Simulate CI
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace \
  --features "rest-api,metrics,admission-webhook,k8s-v1-30"
```

### #1139 — Create deterministic ordering for generated manifests in pipelines

Two sources of non-determinism have been addressed:

1. **`crd-gen` Makefile target** — output of `cargo run --bin crdgen` is now
   piped through `scripts/sort-manifests.py`, which sorts all YAML mapping keys
   recursively and orders documents by `(kind, namespace, name)`.

2. **`bundle-render` Makefile target** — output of `helm template` is sorted the
   same way before being written to `rendered/manifests.yaml`.

3. **`ci.yml` `manifest-order` job** — new CI job that:
   - Verifies `config/crd/stellarnode-crd.yaml` is already in canonical sorted
     order (fails if uncommitted CRD changes are present without sorting).
   - Renders the Helm chart twice and diffs the sorted outputs to confirm
     idempotence.

**Verification:**
```bash
# Sort an existing manifest and check for diffs
python3 scripts/sort-manifests.py config/crd/stellarnode-crd.yaml \
  | diff - config/crd/stellarnode-crd.yaml && echo "Already sorted"

# Regenerate CRD with deterministic output
make crd-gen
git diff config/crd/stellarnode-crd.yaml  # should be empty if already sorted
```

---

### `ci.yml`
- **Change detection** gates expensive jobs (helm-lint, api-docs, examples-smoke-test,
  security-audit) so they only run when relevant files change.
- **Unified Rust cache** via `setup-rust` composite action with per-job `shared-key`.
- **Removed duplicate** system-dependency install blocks (now in `setup-rust`).
- **Removed duplicate** `actions/checkout@v6` references (standardised on `@v4`).
- `lint` and `security-audit` run in **parallel** (both depend only on `changes`).
- `test` runs on every PR; `coverage` runs on **main pushes only** (tarpaulin is slow).
- Removed standalone `pre-commit.yml` and `commit-lint.yml` workflows — lint/format
  is covered by the main `ci.yml` `lint` job.

### Estimated time reduction
Parallel lint + audit + test/coverage, combined with shared caching, reduces
the critical path by ~35–40% compared to the previous sequential layout.

---

## Heavy Validation Workflows (#703)

### `chaos-tests.yml`
- **Extracted** cluster provisioning into `setup-kind-cluster` composite action.
- **Parallel execution**: experiments 01–02 (pod-kill, network partition) run in
  `chaos-kill-network` job; experiments 03–05 (latency, peer-partition, disk-fill)
  run in `chaos-latency-disk` job simultaneously.
- **Consolidated logging** via `collect-e2e-logs` composite action.
- Binary built once in a `build` job and downloaded as an artifact by both
  parallel jobs — no duplicate Rust compilation.

### `soak-test.yml`
- Uses `setup-kind-cluster` for cluster provisioning.
- Uses `collect-e2e-logs` for failure-time log collection.
- Removed duplicated Rust toolchain + apt-get blocks.

### `verify-operator-boot.yml`
- Uses `setup-rust` composite action.
- Runs on **main pushes** and `workflow_dispatch` only (kind-cluster boot check is
  too heavy for every contributor PR).
- Artifact name includes `github.run_id` to avoid collisions.

---

## Performance & Benchmark Workflows (#701)

### `performance.yml` (unified pipeline)
- **Replaces** the former `benchmark.yml`, `performance-regression.yml`, and
  `webhook-benchmark.yml` with a single matrix-driven workflow.
- Runs on **main pushes** (path-filtered) and `workflow_dispatch` — not on PRs.
- **Shared build job** produces the operator binary and Docker image once; all
  three suites (operator, regression, webhook) download the same artifact.
- **Matrix execution** runs operator and regression suites via `setup-perf-env`,
  and the webhook suite directly (no kind cluster required).
- **Shared baseline comparison** via `.github/actions/compare-benchmarks`
  composite action wrapping `compare_benchmarks.py`.

---

## Release & Multi-Arch Workflows (#665)

### `multiarch-build.yml`
> **Correction (issue #935):** this workflow no longer exists in
> `.github/workflows/`. The multi-arch container image is published by the
> `container` job in `release.yml` (QEMU + `docker buildx`). Local
> multi-arch builds use `make docker-multiarch`, which runs `docker buildx
> build --platform linux/amd64,linux/arm64`. The notes below are kept as the
> original design intent.
- Runs on **main pushes** (path-filtered) and `workflow_dispatch` — not on PRs.
- Per-platform GHA cache scopes (`multiarch-amd64`, `multiarch-arm64`) prevent
  cross-arch cache pollution and improve cache hit rates.
- `arch-benchmark` jobs use `setup-rust` composite action.
- Combined manifest build pulls from both per-platform caches.

### `release.yml`
- **Eliminated duplicate Docker build**: `container` job first attempts to
  re-tag the `sha-<sha>` image already published by `multiarch-build.yml`.
  A fresh build only runs as a fallback when the sha image is unavailable.
- **Fail-safe**: `validate` job enforces semver format AND Cargo.toml version
  match before any build or publish step runs. A mismatch is now a hard error
  (previously a warning).
- `release` job depends on ALL of: `build-artifacts`, `container`, `security`,
  `helm` — broken builds can never be tagged for release.
- Standardised on `actions/upload-artifact@v4` / `actions/download-artifact@v4`.

---

## Action Version Standardisation & Security

All workflows now use consistent, security-hardened action versions:

| Action | Version | Security Notes |
|--------|---------|----------------|
| `actions/checkout` | `@v7` | Latest with security patches |
| `actions/setup-node` | `@v4` | Stable, consistent |
| `actions/setup-python` | `@v6` | **Fixed inconsistency** (was mixed v5/v6) |
| `actions/upload-artifact` | `@v4` | Consistent across all workflows |
| `actions/download-artifact` | `@v4` | Consistent across all workflows |
| `actions/cache` | `@v4` | Stable caching |
| `helm/kind-action` | `v1.14.0` | Pinned for stability |
| `docker/build-push-action` | `@v7` | Latest with security improvements |
| `aquasecurity/trivy-action` | `@v0.36.0` | **Fixed inconsistency** (was mixed v0.35.0/v0.36.0) |
| `Swatinem/rust-cache` | `@v2` | **Optimized configuration** |

### Security Hardening Applied

#### Docker Image Security
- **Valid base image digest**: Fixed dummy SHA256 → actual `debian:bookworm-slim` digest
- **Supply chain verification**: Ensures reproducible, verified builds
- **SBOM generation**: Enabled for all release artifacts
- **Provenance attestation**: Cryptographic build provenance for containers

#### Dependency Security
- **Centralized audit config**: Moved from inline CLI ignores to documented `.cargo/audit.toml`
- **Justified ignores**: Each security advisory ignore includes:
  - Technical rationale for why it's safe to ignore
  - Conditions for removal
  - Review date for re-evaluation
- **Eliminated phantom entries**: Removed non-existent future-year RUSTSEC IDs

---

## Reliability Testing & Monitoring

### New CI Reliability Test (`ci-reliability-test.yml`)
Validates pipeline stability and hardening:

- ✅ **Docker config validation**: Verifies base image digests are valid
- ✅ **Security audit testing**: Confirms audit configuration is functional  
- ✅ **Action version consistency**: Detects version drift across workflows
- ✅ **Cache configuration**: Validates deprecated settings are removed
- ✅ **Retry logic testing**: Confirms error handling patterns exist
- ✅ **Documentation completeness**: Ensures troubleshooting guides exist

### Troubleshooting Documentation
New comprehensive guide: `.github/CI_TROUBLESHOOTING.md`

**Covers common failure scenarios:**
- Docker build failures and digest issues
- Security audit failures and ignore management  
- Test timeouts and performance regressions
- Cache restoration problems
- Action version conflicts

**Includes local reproduction steps:**
```bash
# Reproduce CI failures locally
docker build --target runtime --platform linux/amd64 .
cargo test --all-features --workspace
cargo audit  # Uses .cargo/audit.toml config
```

---

## Monitoring & Success Metrics

### Target Reliability Metrics
- **Success rate**: >95% on main branch
- **Build duration**: <45 minutes end-to-end
- **Cache hit rate**: >80% for Rust builds
- **Security audit**: 0 unaddressed critical/high CVEs

### Alert Conditions
- 3+ consecutive main branch failures
- Individual job runtime >60 minutes  
- Cache hit rate <60% (indicates configuration issues)
- New high/critical CVEs not addressed within 7 days
