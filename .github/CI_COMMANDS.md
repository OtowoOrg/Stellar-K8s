# CI Pipeline Architecture & Optimization Guide

## Overview

This document describes the optimized CI/CD pipeline architecture introduced
to address issues #700, #701, #703, and #714.

---

## Shared Composite Actions

All reusable logic lives under `.github/actions/`:

| Action | Purpose |
|--------|---------|
| `setup-rust` | Install Rust toolchain + system deps + Swatinem cache |
| `setup-kind-cluster` | Provision kind cluster, load image, install CRDs, deploy operator |
| `collect-e2e-logs` | Dump operator logs, K8s events, StellarNode status → artifact |
| `setup-perf-env` | Install k6/kind/kubectl, create cluster, deploy operator with RBAC, port-forward |

---

## Core CI Workflows (#700)

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

## Action Version Standardisation

All workflows now use consistent, valid action versions:

| Action | Version |
|--------|---------|
| `actions/checkout` | `v4` |
| `actions/setup-node` | `v4` |
| `actions/setup-python` | `v5` |
| `actions/upload-artifact` | `v4` |
| `actions/download-artifact` | `v4` |
| `actions/cache` | `v4` |
| `helm/kind-action` | `v1.14.0` |
| `docker/build-push-action` | `v6` |
| `Swatinem/rust-cache` | `v2` |
