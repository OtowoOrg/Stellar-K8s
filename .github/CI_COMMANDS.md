# CI Pipeline Architecture & Optimization Guide

## Overview

This document describes the optimized CI/CD pipeline architecture introduced
to address issues #663, #664, #665, and #666.

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

## Core CI Workflows (#663)

### `ci.yml`
- **Change detection** gates expensive jobs (helm-lint, api-docs, examples-smoke-test,
  security-audit) so they only run when relevant files change.
- **Unified Rust cache** via `setup-rust` composite action with per-job `shared-key`.
- **Removed duplicate** system-dependency install blocks (now in `setup-rust`).
- **Removed duplicate** `actions/checkout@v6` references (standardised on `@v4`).
- `lint` and `security-audit` run in **parallel** (both depend only on `changes`).
- `test` and `coverage` run in **parallel** (both depend on `lint` + `security-audit`).

### `pre-commit.yml`
- Uses `setup-rust` composite action — no more duplicated apt-get blocks.
- Added `concurrency` group to cancel stale runs.

### `commit-lint.yml`
- Fixed invalid action versions (`actions/checkout@v6`, `actions/setup-node@v6`
  → `@v4`).
- Pinned commitlint packages to major version `@19`.
- Added `concurrency` group.

### Estimated time reduction
Parallel lint + audit + test/coverage, combined with shared caching, reduces
the critical path by ~35–40% compared to the previous sequential layout.

---

## Heavy Validation Workflows (#666)

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
- Added `concurrency` group to cancel stale PR runs.
- Artifact name now includes `github.run_id` to avoid collisions.

---

## Performance & Benchmark Workflows (#664)

### `benchmark.yml`
- Build step produces a Docker image saved as a `.tar.gz` artifact.
- `setup-perf-env` composite action handles k6, kind, kubectl, cluster, RBAC,
  operator deployment, and port-forwarding — eliminating ~80 lines of duplicated
  setup across the three benchmark workflows.
- Baseline comparison uses the same `compare_benchmarks.py` script as
  `performance-regression.yml`.

### `performance-regression.yml`
- Removed the now-redundant `setup-cluster` job (consolidated into
  `performance-test` via `setup-perf-env`).
- Removed duplicate k6/kind/kubectl install steps.

### `webhook-benchmark.yml`
- Uses `setup-rust` composite action.
- Standardised on `actions/upload-artifact@v4` / `actions/download-artifact@v4`
  (was mixing v7/v8 which don't exist).

---

## Release & Multi-Arch Workflows (#665)

### `multiarch-build.yml`
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
