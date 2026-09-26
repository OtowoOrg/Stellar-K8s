# Stellar-K8s Repository Cleanup Status

## Completed Tasks ✅

### 1. Fixed Critical Compilation Errors
- **Version Mismatch**: Fixed k8s-openapi version from 0.26 to 0.22 to match kube 0.94
  - This was the root cause of 594 initial errors
  - Updated both main and dev dependencies

### 2. Resolved Syntax Errors
- Fixed `$ref` invalid identifier in `openapi.rs` (renamed to `ref_path` with serde rename)
- Fixed brace mismatch in `state_sync.rs` test module

### 3. Removed Duplicate Functions and Types
- Removed duplicate `build_pdb()` function in `resources.rs`
- Removed duplicate `ensure_pdb()` function in `resources.rs`  
- Removed duplicate `dashboard_metrics()` function in `dashboard_handlers.rs`
- Renamed conflicting `Condition` types (network policy vs node status)
- Renamed conflicting `AnomalyDetectionConfig` and `ComplianceStatus` types

### 4. Fixed Import and Module Issues
- Added missing `state_sync` module declaration in `controller/mod.rs`
- Fixed `bytes` crate from optional to required dependency
- Fixed `CanaryStrategy` removed from exports (doesn't exist in stellar_autoscaler)
- Fixed `sqlx::PgRow` import to use `sqlx::postgres::PgRow`

### 5. Data Model Fixes
- Fixed `EtlRecord` field access issues in `partitioning.rs` and `quality.rs`
  - Added helper functions to extract fields from payload JSON
  - Removed assumptions about direct struct fields
- Fixed `ResourceSpec` field access in `quota.rs`
  - Changed from HashMap-style `.get()` to direct field access (`cpu`, `memory`)
- Fixed `AuthError` enum field access in `gateway/mod.rs`
  - Changed from struct field access to proper enum matching

### 6. Added Missing Trait Implementations
- Added `PartialEq, Eq` to `PluginHook` enum
- Added `Clone` to authentication types: `JwtAuth`, `OAuth2Auth`, `ApiKeyAuth`, `AuthMiddleware`
- Added `Serialize, Deserialize` to `ApiVersion` struct
- Added `Display` implementation for `StellarNetwork` enum
- Added `#[schemars(skip)]` to k8s types that don't implement JsonSchema (`Volume`, `VolumeMount`)
- Added `#[serde(skip)]` to `Instant` field that can't be serialized

### 7. API Updates
- Updated prometheus-client API calls to use `Default::default()` instead of deprecated `Counter::new()`

## Progress Summary

### Error Reduction
- **Initial errors**: 594
- **After k8s-openapi fix**: 170  (-424)
- **After duplicate removal**: 91   (-79)
- **After data model fixes**: 63   (-28)
- **After trait implementations**: 42 (-21)
- **Current**: 42 errors remaining

### Error Breakdown (Current)
```
10 error[E0599]  - Method not found
5  error[E0433]  - Cannot find in scope  
5  error[E0425]  - Cannot find value/function
4  error[E0308]  - Type mismatch
4  error[E0277]  - Trait bound not satisfied
3  error[E0063]  - Missing struct fields
3  error[E0061]  - Wrong number of arguments
2  error[E0502]  - Cannot borrow as mutable
1  error[E0733]  - Recursion limit reached
1  error[E0560]  - Struct has no field
1  error[E0505]  - Cannot move out of borrowed
1  error[E0283]  - Type annotations needed
1  error[E0119]  - Conflicting implementations
1  error[E0107]  - Wrong number of type arguments
```

## Remaining Issues ⚠️

### Critical Issues

#### 1. Request/Response Body Cloning (E0599)
**Location**: `src/rest_api/gateway/mod.rs:119, 149`

**Problem**: `Request<Body>` and `Response<Body>` can't be cloned because `Body` doesn't implement `Clone`.

**Solution**: 
- Option A: Store only metadata in `PluginContext` (method, URI, headers) instead of full request
- Option B: Use `hyper::Body` wrapper that supports cloning
- Option C: Redesign plugin API to work with references

```rust
// Current (broken):
pub struct PluginContext {
    pub request: Request<Body>,  // Can't clone
    pub auth: AuthContext,
    pub state: Arc<ControllerState>,
}

// Suggested fix:
pub struct PluginContext {
    pub method: http::Method,
    pub uri: http::Uri,
    pub headers: http::HeaderMap,
    pub auth: AuthContext,
    pub state: Arc<ControllerState>,
}
```

#### 2. Missing AuthConfig From Implementation (E0277)
**Location**: `src/rest_api/gateway/handlers.rs:100`

**Problem**: `AuthMiddleware` doesn't implement `From<AuthMiddleware>` for `AuthConfig`.

**Solution**: Either implement the trait or fix the conversion code.

### Module Organization Issues

#### 3. Missing Imports/Functions (E0433, E0425)
Several files reference functions or types that don't exist or aren't imported:
- Check for missing `use` statements
- Verify all referenced modules are declared
- Ensure feature flags are enabled where needed

### Type Issues (E0308, E0277)
Some type mismatches remain - these need case-by-case review based on actual usage context.

## Recommendations

### Short Term (Before CI/CD)
1. **Fix the 42 remaining errors** - Focus on E0599 (10 errors) and E0433/E0425 (10 errors) first
2. **Address plugin architecture** - The Request/Response cloning issue affects the entire plugin system
3. **Run `cargo fmt`** - Format all code consistently
4. **Run `cargo clippy`** - Fix linter warnings

### Medium Term (CI/CD Improvement)
1. **Enable `fail-fast: false`** in CI workflows to see all errors at once
2. **Add pre-commit hooks** for formatting and basic checks
3. **Set up dependency caching** to speed up CI builds
4. **Add nightly rust checks** for early warning of API changes

### Long Term (Maintainability)
1. **Consolidate CRD definitions** - Reduce overlap between stellar_observability, stellar_aiops, stellar_security
2. **Simplify data pipeline** - EtlRecord structure is too generic, consider typed variants
3. **Review gateway architecture** - Plugin system needs refactoring for better type safety
4. **Document API version compatibility** - Clear matrix of kube/k8s-openapi versions

## Files Modified

### Core Fixes
- `Cargo.toml` - Version fixes, dependency changes
- `src/crd/mod.rs` - Renamed conflicting exports
- `src/controller/mod.rs` - Added state_sync module
- `src/controller/resources.rs` - Removed duplicates, fixed ResourceSpec usage
- `src/rest_api/dashboard_handlers.rs` - Removed duplicate function, fixed imports

### Data Layer
- `src/data_pipeline/partitioning.rs` - Fixed EtlRecord field access
- `src/data_pipeline/quality.rs` - Added helper functions for payload extraction
- `src/controller/quota.rs` - Fixed ResourceSpec field access

### API Layer
- `src/rest_api/gateway/openapi.rs` - Fixed $ref field naming
- `src/rest_api/gateway/mod.rs` - Fixed AuthError handling
- `src/rest_api/gateway/handlers.rs` - Fixed OpenApiGenerator usage
- `src/rest_api/gateway/auth.rs` - Added Clone derives
- `src/rest_api/gateway/router.rs` - Added Serialize/Deserialize
- `src/rest_api/gateway/plugin.rs` - Added PartialEq derive
- `src/rest_api/gateway/analytics.rs` - Updated prometheus API

### CRD Layer
- `src/crd/stellar_node.rs` - Added schema skips for k8s types
- `src/crd/types.rs` - Added Display impl for StellarNetwork
- `src/load_balancer.rs` - Added serde skip for Instant

### Tests
- `src/controller/state_sync.rs` - Fixed brace mismatch
- `src/controller/maintenance/query_profiler.rs` - Fixed PgRow import

### 8. Makefile Refactoring - Split Oversized Targets
- **Extracted shared variables**: Created `CLIPPY_BASE_FLAGS`, `CLIPPY_STRICT_FLAGS`, and `CLIPPY_FEATURES` to eliminate 40+ lines of duplication between `lint` and `lint-strict` targets
- **Split `quickstart` into composable phases**:
  - `quickstart-setup`: Prerequisite checks and kind cluster creation
  - `quickstart-build`: Build and load Docker image
  - `quickstart-deploy`: Deploy operator and sample resources
  - `quickstart`: Orchestrates all phases (backward compatible)
- **Split `bundle` into smaller targets**:
  - `bundle-render`: Render Helm chart to manifests
  - `bundle-generate`: Generate OLM bundle
  - `bundle-validate`: Validate bundle
  - `bundle`: Orchestrates all phases (backward compatible)
- **Split `completions` into per-shell targets**:
  - `completions-bash`: Generate bash completions
  - `completions-zsh`: Generate zsh completions
  - `completions-fish`: Generate fish completions
  - `completions`: Orchestrates all shells (backward compatible)
- **Split `dev-setup` into logical steps**:
  - `dev-setup-rust`: Install Rust toolchain and components
  - `dev-setup-tools`: Install development tools
  - `dev-setup-hooks`: Install git hooks
  - `dev-setup`: Orchestrates all steps (backward compatible)
- **Benefits**:
  - Improved maintainability and readability
  - Enables running individual phases (e.g., `make quickstart-setup` to just create cluster)
  - Easier debugging (can run each step independently)
  - Better composability for CI/CD pipelines
  - All original targets remain functional (backward compatible)

## Next Steps

To get to a green build:

1. **Run**: `cargo check --lib 2>&1 | grep "error\[E0599\]" -A 5` to see all method-not-found errors
2. **Fix PluginContext cloning** - This is blocking 2 errors
3. **Run**: `cargo check --lib 2>&1 | grep "error\[E0433\]\|error\[E0425\]" -A 3` for missing imports
4. **Address remaining 30 errors** individually
5. **Run full test suite**: `cargo test --workspace`
6. **Check CI workflows**: Verify they run with current changes

## CI/CD Pipeline Status

The following checks need to pass:
- [x] Formatting (`cargo fmt --check`)
- [ ] Compilation (`cargo check --workspace`) - **42 errors remaining**
- [ ] Linting (`cargo clippy`)
- [ ] Tests (`cargo test --workspace`)
- [ ] Security audit (`cargo audit`)
- [ ] Helm lint
- [ ] API docs generation

Current blocker: Compilation errors must be fixed before other checks can run.

## Wave Issues — Batch (Issues #900, #1114, #1115, #1116) ✅

### Issue #900 — Break Up Oversized Controller Modules

**Problem:** `src/controller/reconciler.rs` (4,344 lines) and
`src/controller/resources.rs` (4,873 lines) were monolithic files impossible
to navigate or test in isolation.

**Solution:** Each file was decomposed into a directory of focused sub-modules
with a `mod.rs` facade that re-exports the original public API unchanged:

- `src/controller/reconciler/`
  - `mod.rs` — re-exports; public API surface unchanged
  - `state.rs` — `ControllerState` definition
  - `batch.rs` — `BatchSummaryReport` + tests
  - `events.rs` — `emit_event!`, `publish_stellar_event!`, `Recorder` helpers
  - `dry_run.rs` — `apply_or_emit!`, `apply_or_emit_owned`, `ActionType`
  - `runner.rs` — `run_controller` entry-point, watch setup, background workers
  - `core.rs` — core `reconcile()` state machine

- `src/controller/resources/`
  - `mod.rs` — re-exports; public API surface unchanged
  - `meta.rs` — `standard_labels`, `owner_reference`, `resource_name`, annotation helpers
  - `probes.rs` — default liveness / readiness / startup probes + tests
  - `pvc.rs` — `build_pvc`, `ensure_pvc`, `delete_pvc` + tests
  - `config_map.rs` — `build_config_map`, `ensure_config_map`, `delete_config_map`
  - `service.rs` — `ensure_service`, `build_service`
  - `deployment.rs` — `ensure_deployment`, `build_deployment`
  - `statefulset.rs` — `ensure_statefulset`, `build_statefulset`
  - `reconcile.rs` — `reconcile_node_resources` top-level entry-point

No behavior changes were introduced. All public APIs are preserved via re-exports.

### Issue #1114 — Static Audit for Unused Crate Features and Dead Imports

**Added:** `scripts/audit-features.sh`

Four-step static audit:
1. Declared features with no `#[cfg(feature = "...")]` guard in `src/`.
2. Unused imports via `RUSTFLAGS="-D unused_imports" cargo check`.
3. Dead code via `RUSTFLAGS="-D dead_code" cargo check`.
4. Optional dependencies with no explicit feature activation in `[features]`.

Wired into `.github/workflows/ci.yml` as the `feature-audit` job (runs when
`rust_core` or `deps` files change).

### Issue #1115 — Standardize Logging Fields for CI and Runtime Diagnostics

**Added:** `src/logging/fields.rs`

Single source of truth for all structured log field name constants used across
CI pipelines and runtime tracing call-sites:

- `NODE`, `NAMESPACE`, `NODE_TYPE`, `CLUSTER`, `K8S_NODE`
- `RECONCILE_ID`, `PHASE`
- `ERROR`, `DURATION_MS`, `COMPONENT`
- `LEDGER`, `VERSION`, `REGION`
- `JOB_ID`, `AUDIT_ACTION`, `SCRUB_PATTERN`
- `TRACE_ID`, `SPAN_ID`
- `CI_STEP`, `GIT_SHA`, `FEATURES`

All constants are validated by unit tests (non-empty, lowercase snake_case,
unique). The module is exported from `src/logging/mod.rs` as `logging::fields`.

### Issue #1116 — Add Secure Secret-Handling Checks to All Pipeline Command Paths

**Added:** `scripts/check-secrets.sh`

Five-layer check:
1. Hard-coded credential patterns (Stellar seeds, AWS keys, PEM blocks, `password=` literals).
2. Shell script echo hygiene — detects `echo $SECRET_VAR` and `set -x`.
3. GitHub Actions workflow safety — detects unmasked `${{ secrets.X }}` echoes.
4. Dockerfile hygiene — detects `ENV`/`ARG` directives with secret-like names.
5. Rust source literals — detects Stellar seed strings outside test/fixture files.

Wired into `.github/workflows/ci.yml` as the `secret-checks` job (runs on
every push/PR) and gates the `test` and `coverage` jobs on its success.

### Issue #935 — Normalize Makefile Targets and Remove Deprecated Ones

**Fixed:**
- `docs-lint` was defined **twice** with byte-identical recipes (`Makefile:299`
  and `:306`). GNU make silently overrode the first and emitted an
  "overriding recipe" warning on every invocation, and `make help` listed the
  target twice. Removed the duplicate.
- `make help` used a **non-portable awk** field separator (`FS = ":.*?## "`).
  The lazy quantifier `?` is a GNU extension; under `mawk` (the default on
  Debian/Ubuntu) and BSD awk it is a literal `?`, so the separator never matched
  and the entire "All available targets" list silently vanished. Switched to the
  portable `FS = ":.*## "`. Verified no help text contains a second `## `, so
  greedy-vs-lazy splitting is equivalent.
- `make docker-multiarch` dispatched `gh workflow run multiarch-build.yml`, but
  **that workflow does not exist** in `.github/workflows/` — the target could
  never succeed. Replaced with a real local multi-arch build
  (`docker buildx build --platform linux/amd64,linux/arm64`), which also matches
  what `DEVELOPMENT.md` claimed the target did. The phantom workflow reference
  was corrected in `.github/CI_COMMANDS.md` and `.github/workflows/release.yml`
  (the `container` job in `release.yml` is the real multi-arch publisher).
- `make run-local` ran bare `./target/release/stellar-operator`, but
  `Args.command` is a **required** clap subcommand, so the binary exited with a
  usage error. `run-local` now passes `run`; `make run` is now a true alias of
  it, matching its own help text.
- `make security-fix` was documented as "Apply automated security fixes where
  possible" but only ran `cargo update --dry-run` and changed nothing. Help text
  and output corrected to say it is a report, not a fixer.
- Removed three duplicated recipe bodies: `pre-commit-install` and
  `dev-setup-hooks` were byte-identical; `validate` duplicated `health-fast`.
  Both are now declared as prerequisite-based aliases.

**Added:**
- `make list-doc-coverage` — documented in `docs/stale-docs-detector.md` but the
  target did not exist, so the documented command failed. Wired to the existing
  `doc-check list` subcommand.
- `make security-check` — gives the previously orphaned
  `scripts/security-check.sh` a first-class entry point (referenced by
  `SECURITY.md` with no way to run it through make).

**Fixed live CI break:**
- `.github/workflows/soak-test.yml:78` ran `bash scripts/soak-test.sh`, but that
  file only existed at `scripts/archive/soak-test.sh` (it is an operational
  script, not a one-off bootstrap script, so the archive was the wrong home).
  Restored with `git mv`; the script has no self-relative path references, so
  the move is safe.

**Docs synced:**
- `CONTRIBUTING.md` — `make install` → `make install-crd` (no `install` target
  exists); `ci-local` description now includes `docs-lint`.
- `docs/developer-onboarding/index.md` — `make deploy` → `make quickstart-deploy`
  (no `deploy` target exists).
- `docs/stale-docs-detector.md` — `check-stale-docs` is `--warn-only` and exits
  0 (it does **not** exit non-zero); the strict gate is `docs-check-strict`;
  removed the false claim that these targets are wired into `ci-local`.
- `DEVELOPMENT.md` — documented `health-fast`; corrected `ci-local`.
- `SECURITY.md` — replaced raw `cargo deny`/`cargo audit`/`cargo outdated` and
  the bare script path with the canonical make targets.
- `docs/development/makefile-refactoring.md` — updated the CI target list to the
  targets actually invoked by `.github/workflows/*.yml`.

**Verified:** all 80 `.PHONY` entries have a target definition, a recipe, and
`## ` help text; zero duplicate target definitions; no space-indented recipe
lines.

### Issue #936 — Audit and Update Third-Party License References

Scoped to non-breaking corrections. `THIRD_PARTY_LICENSES.md` is gated in CI by
a byte-exact diff (`make check-third-party-licenses`), and the generator
requires `cargo-license`, so the generator was deliberately **not** modified —
doing so without regenerating the file would turn the gate red.

**Corrected factual errors in `DEPENDENCY_SECURITY_AUDIT.md`:**
- The "23 known advisories" figure was wrong in three places. There is no single
  list; the four ignore lists hold 20 (`deny.toml`), 26 (`.cargo/audit.toml`),
  18 (`ci.yml`) and 15 (`dependency-review.yml`) entries. Replaced with the
  measured table.
- `anyhow 1.0.103` / `bytes 1.11.1` as "version pinning for security fixes" —
  `anyhow` was in fact pinned to a non-existent `1.0.108` that broke resolution
  outright (see the `Cargo.toml` fix above); now documented as `1.0.104`.
- `rustls-webpki` was listed as two versions; the lock contains three
  (`0.101.7`, `0.102.8`, `0.103.13`), and the target `>=0.103.12` is already met
  by one of them.
- `rand 0.9.2` — the lock has `0.8.6` and `0.9.4`.
- "✅ Explicit handling of copyleft licenses" was not substantiated; replaced
  with an accurate note that `ittapi`/`r-efi` copyleft options are permitted only
  because a permissive `OR` branch is allowlisted.

**`deny.toml` — removed two false claims:**
- It asserted it was "in sync with `.cargo/audit.toml`" (it is missing 6 entries)
  and "in sync with the workflow `cargo-audit --ignore` lists" (13 entries are
  missing, and 5 appear only in CI). Replaced with the measured divergence.
- Flagged the `pqcrypto` ignores (`RUSTSEC-2024-0380/-0381`) as dead: no
  `pqcrypto` package exists in `Cargo.toml` or `Cargo.lock`, so their
  "experimental pqcrypto KMS path" justification describes a component that is
  not present.

**New "Open Gaps" section** documenting, with evidence, seven unresolved items —
the most significant being that `rdkafka`, `sasl2-sys` and `async-nats` are
compiled by CI (`.pre-commit-config.yaml` runs `cargo clippy`/`cargo test
--workspace --all-features`) but are **absent from the license file**, because
the generator pins a narrower feature set than what is actually built.

**Noted but not changed:** no `deny.toml` license exceptions were added. None are
needed — cargo-deny satisfies an expression when any branch of an `OR` is
allowlisted, so `MIT OR Unlicense`, `BSD-3-Clause OR GPL-2.0` and
`Apache-2.0 OR BSL-1.0` already pass. Adding exceptions would have been
incorrect.

### Issue #934 — Add Integration Test Teardown to Avoid Resource Leaks

**Fixed two test files that could not compile.** Both had content appended
*inside* an unclosed function body, with a duplicated file header — `use`
statements are not legal inside a function:
- `tests/backup_restore_smoke_test.rs` — `fn stellar_operator()` was never
  closed and a second copy of the file header (lines 18-24) sat inside its body.
  Removed the dead helper, the duplicated header, and three unused imports
  (`std::fs`, `PathBuf`, `TempDir`). This target is invoked by `ci.yml`, so it
  was a hard failure.
- `tests/cli_examples_test.rs` — `fn invalid_command_fails()` was never closed and
  `use assert_cmd::Command;` was stranded at column 0 inside its body. Closed
  the function and moved the import to the top import block.

**Removed destructive side effects from ordinary `cargo test`.** Four unit tests
in `tests/common/mod.rs` built RAII guards to assert their fields and then let
them `Drop`. Because every guard's `Drop` shells out to `kubectl delete`, plain
`cargo test` was deleting namespaces, `StellarNode` CRs and ConfigMaps from
whatever cluster the developer's kubeconfig pointed at. Each test now ends with
`std::mem::forget`, which suppresses the destructor. No guard API changed, so the
E2E tests that depend on them are unaffected.

**Added the missing teardown primitive.** `ensure_kind_cluster` existed with no
counterpart, so every KinD-backed test leaked a Docker container, network and
volumes. Added to `tests/common/mod.rs`:
- `delete_kind_cluster(name)` — error-tolerant cluster deletion.
- `ClusterGuard` — RAII guard owning a cluster for the life of a test, honouring
  `SKIP_TEARDOWN=1` (which previously only suppressed inline teardown in one
  file, leaving `NamespaceGuard` drops active — an inconsistent contract).

**Wired `ClusterGuard` in:**
- `tests/quickstart_smoke_test.rs` — all three tests previously called
  `delete_kind_cluster` inline at the end of the body, so any failing
  `assert!`/`wait_for_*` leaked the cluster. Replaced with a function-scoped
  guard and removed the now-unused local `delete_kind_cluster` and
  `skip_teardown` helpers.
- `tests/dr_failover_e2e.rs` — guard registered immediately after cluster
  creation so `DrCleanup` (namespaces/CRs) drops first and the cluster last.

**`CONVENTIONS.md`** now documents `ClusterGuard` as mandatory, with the
correct/incorrect guard-scoping patterns and the `mem::forget` rule for
unit-testing guards.

**Known remaining leaks (not addressed here):**
- `tests/e2e_kind.rs` (9 tests) still uses five copy-pasted local guard types
  instead of the shared helpers, and no test deletes its KinD cluster. These
  tests share a cluster name, so a correct fix needs the per-test cluster
  ownership modelled before swapping in `ClusterGuard`.
- `tests/e2e_kind.rs` — `e2e_namespace_scoped_reconciliation` has no guard at
  all; the `kubectl port-forward` child at ~line 1488 is not process-guarded;
  `UpgradeCleanup` tracks only the old operator manifest.
- `tests/common/mod.rs::E2eTestGuard` remains unused outside its own unit test.
- `tests/chaos/run-chaos-tests.sh` and the `chaos-tests`, `soak-test`,
  `setup-kind-cluster` and `setup-perf-env` workflows create KinD clusters with
  no teardown step (only `verify-operator-boot.yml` and
  `scripts/quickstart-verify.sh` delete theirs).
- `dry_run_test.rs:180-187` sets and removes `TEST_DRY_RUN` without restoring it
  on panic.
