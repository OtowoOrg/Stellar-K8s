# Bats Test Coverage

Bats (Bash Automated Testing System) provides comprehensive test coverage for shell scripts in the Stellar-K8s repository.

## Running Bats Tests

All bats test suites are executed via the `make test-bats` target:

```bash
make test-bats
```

This runs every `.bats` file under `scripts/tests/` except `helm-drift.bats` (which requires Helm; use `make test-helm-drift` instead).

Individual suites can be run directly:

```bash
bats scripts/tests/cleanup.bats
bats scripts/tests/secret-rotation-check.bats
```

## Test Suites

### Core Repository Health

- **`cleanup.bats`**: Validates `scripts/cleanup.sh`, the single supported repository cleanup tool. Tests include artifact removal, obsolete path detection, and dry-run mode.
- **`preflight.bats`**: Tests the `preflight.sh` toolchain version gate. Validates minimum versions for Rust, kind, kubectl, Helm, and other required tools.
- **`dep-gate.bats`**: Validates the consolidated dependency gate (`scripts/dep-gate.sh`), including `cargo audit`, `cargo deny`, license, and build checks.
- **`shell-script-coverage.bats`**: Comprehensive coverage for critical scripts:
  - `health-check.sh`: Environment health gate with `--json` and `--fix` modes
  - `repo-health.sh`: Repository health aggregator with `--fast` mode
  - `tag-release.sh`: Release tag automation with semver validation
  - `validate-metrics.sh`: Metric reference drift detection
  - `check-secrets.sh`: Secret handling audit
  - `check-stale-samples.sh`: Sample manifest validation
  - `generate-third-party-licenses.sh`: SBOM generation and drift checks
  - `create-argocd-branch.sh`: GitOps helper automation
  - `quickstart-verify.sh`: End-to-end quickstart validation
  - `quickstart-golden-path.sh`: Golden path static validation
  - `audit-features.sh`: Feature audit and dead code detection
  - `golden-path-pipeline-check.sh`: Full pipeline validation

### Build, Release & Configuration

- **`bump-chart-version.bats`**: Tests SemVer chart bumping via conventional commits, including `min-bump` floor and `--dry-run` mode.
- **`helm-drift.bats`** (run via `make test-helm-drift`): Detects drift between Helm templates and committed renders. Tests both `--update` and validation modes.

### Operations & Incident Response

- **`dead-code-report.bats`**: Validates the dead-code report generation (`scripts/dead-code-report.sh`) without requiring a full build.
- **`secret-rotation-check.bats`**: Tests operator secret rotation readiness verification including `--dry-run` mode and argument validation.
- **`failure-diagnostics.bats`**: Verifies the unified failure diagnostics bundle (`ci/collect-failure-diagnostics.sh`), including manifest layout and sanitization.

## CI Integration

The `Bats Suites` GitHub Actions job (part of `.github/workflows/ci.yml`) runs `make test-bats` on every push and pull request. It validates:

- All shell script integration tests pass
- No regressions in script behavior
- Cross-platform shell syntax (via shellcheck)

## Adding New Bats Tests

To add a new bats test suite for a script:

1. **Create the test file** in `scripts/tests/<script-name>.bats`
2. **Add the canonical make target** if the script is meant to be invoked locally:
   ```bash
   <target-name>: ## Description (#issue-number)
       @echo "→ Running <target> tests..."
       @command -v bats >/dev/null 2>&1 || (echo "✗ bats not installed" && exit 1)
       @bats scripts/tests/<script-name>.bats
   ```
3. **Update `scripts/README.md`**: Add a row to the **Test suites** table
4. **Update `CHANGELOG.md`**: Document the new test coverage
5. **Verify locally**:
   ```bash
   make test-bats
   # or
   bats scripts/tests/<script-name>.bats
   ```

## Bats Test Structure

A typical bats test file:

```bash
#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  export REPO_ROOT
  TEST_DIR="$(mktemp -d)"
  export TEST_DIR
}

teardown() {
  rm -rf "${TEST_DIR}"
}

@test "descriptive test name" {
  run bash "${REPO_ROOT}/scripts/example.sh" --arg value
  [ "$status" -eq 0 ]
  [[ "$output" =~ "expected text" ]]
}
```

### Common Patterns

- **Setup/Teardown**: Use temp directories for isolated test runs
- **run**: Captures exit status and output of a command
- **Assertions**:
  - `[ "$status" -eq 0 ]`: Exit code is 0 (success)
  - `[ "$status" -ne 0 ]`: Exit code is non-zero (failure)
  - `[[ "$output" == *"text"* ]]`: Output contains text
  - `[[ "$output" =~ regex ]]`: Output matches regex
- **File Tests**: `[ -f file ]`, `[ -d dir ]`, `[ -x script ]`

## Coverage Gap Analysis

The following scripts currently have no bats test coverage:

- `setup-dev-env.sh`, `setup-linux.sh`, `setup-mac.sh` (OS-level bootstrap; tested via CI only)
- Scripts in `scripts/ci/` (CI-only; not intended for local execution)
- Long-running or cluster-dependent scripts:
  - `quickstart-verify.sh` (partial; needs `kind` cluster)
  - `verify-mtls.sh` (needs cluster + kubectl)
  - `sync-federation-secrets.sh` (needs cluster access)
  - `soak-test.sh` (long-running stability test)
  - `run-chaos-drill.sh` (needs Chaos Mesh)

## See Also

- [scripts/README.md](../../scripts/README.md) — Script index with coverage mapping
- [.github/CI_COMMANDS.md](../../.github/CI_COMMANDS.md) — CI pipeline architecture
- [CONTRIBUTING.md](../../CONTRIBUTING.md) — Contribution workflow
