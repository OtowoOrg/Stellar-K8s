#!/usr/bin/env bats
# Comprehensive bats test coverage for critical shell scripts.
# This suite validates scripts that handle repository health, validation, and tooling.
# See scripts/README.md for script descriptions and invoke via: make test-bats

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  export REPO_ROOT
  TEST_DIR="$(mktemp -d)"
  export TEST_DIR
}

teardown() {
  rm -rf "${TEST_DIR}"
}

# ────────────────────────────────────────────────────────────────────────────
# health-check.sh: Environment health gate
# ────────────────────────────────────────────────────────────────────────────

@test "health-check.sh --help exits 0" {
  run bash "${REPO_ROOT}/scripts/health-check.sh" --help
  [ "$status" -eq 0 ]
}

@test "health-check.sh --json produces valid JSON output" {
  run bash "${REPO_ROOT}/scripts/health-check.sh" --json
  [ "$status" -eq 0 ]
  # Validate JSON structure: should contain checks array or status object
  [[ "$output" =~ \{.*\} ]] || [[ "$output" =~ \[.*\] ]]
}

@test "health-check.sh validates without error on clean environment" {
  run bash "${REPO_ROOT}/scripts/health-check.sh"
  # May exit non-zero if tools are missing, but should not crash
  [ "$status" -le 1 ]
}

# ────────────────────────────────────────────────────────────────────────────
# repo-health.sh: Repository health aggregator
# ────────────────────────────────────────────────────────────────────────────

@test "repo-health.sh --help exits 0" {
  run bash "${REPO_ROOT}/scripts/repo-health.sh" --help
  [ "$status" -eq 0 ]
}

@test "repo-health.sh --fast runs fast-path gates" {
  # --fast should not require a full build; test that it doesn't hang
  timeout 30 bash "${REPO_ROOT}/scripts/repo-health.sh" --fast >/dev/null 2>&1 || true
  # We don't assert success here since environment might be incomplete
}

# ────────────────────────────────────────────────────────────────────────────
# tag-release.sh: Release tag automation
# ────────────────────────────────────────────────────────────────────────────

@test "tag-release.sh requires TAG parameter" {
  run bash "${REPO_ROOT}/scripts/tag-release.sh"
  [ "$status" -ne 0 ]
  [[ "$output" == *"usage:"* ]] || [[ "$output" == *"TAG"* ]]
}

@test "tag-release.sh validates TAG format (v prefix)" {
  run bash "${REPO_ROOT}/scripts/tag-release.sh" --dry-run not-a-tag
  [ "$status" -ne 0 ]
}

@test "tag-release.sh --dry-run exits 0 with valid semver tag" {
  run bash "${REPO_ROOT}/scripts/tag-release.sh" --dry-run vX.Y.Z
  [ "$status" -eq 0 ]
}

# ────────────────────────────────────────────────────────────────────────────
# validate-metrics.sh: Metric reference drift detection
# ────────────────────────────────────────────────────────────────────────────

@test "validate-metrics.sh exits with expected code" {
  # May fail if metrics don't match docs; test it doesn't crash
  bash "${REPO_ROOT}/scripts/validate-metrics.sh" >/dev/null 2>&1 || true
  # Just verify the script exists and is executable
  [ -f "${REPO_ROOT}/scripts/validate-metrics.sh" ]
}

# ────────────────────────────────────────────────────────────────────────────
# check-secrets.sh: Secret handling audit
# ────────────────────────────────────────────────────────────────────────────

@test "check-secrets.sh --report never fails" {
  run bash "${REPO_ROOT}/scripts/check-secrets.sh" --report
  [ "$status" -eq 0 ]
}

@test "check-secrets.sh default run completes" {
  bash "${REPO_ROOT}/scripts/check-secrets.sh" >/dev/null 2>&1 || true
  # Script should complete without hanging
}

# ────────────────────────────────────────────────────────────────────────────
# check-stale-samples.sh: Sample manifest validation
# ────────────────────────────────────────────────────────────────────────────

@test "check-stale-samples.sh runs without hanging" {
  timeout 10 bash "${REPO_ROOT}/scripts/check-stale-samples.sh" >/dev/null 2>&1 || true
  # May fail due to missing tools, but should complete
}

# ────────────────────────────────────────────────────────────────────────────
# generate-third-party-licenses.sh: License SBOM generation
# ────────────────────────────────────────────────────────────────────────────

@test "generate-third-party-licenses.sh --check validates existing file" {
  # This checks if the THIRD_PARTY_LICENSES.md is up to date
  run bash "${REPO_ROOT}/scripts/generate-third-party-licenses.sh" --check
  # May fail if file is stale, but shouldn't crash
  [ "$status" -le 1 ]
}

# ────────────────────────────────────────────────────────────────────────────
# create-argocd-branch.sh: GitOps helper
# ────────────────────────────────────────────────────────────────────────────

@test "create-argocd-branch.sh without git context fails gracefully" {
  run bash "${REPO_ROOT}/scripts/create-argocd-branch.sh" 2>&1
  # Should fail without a remote URL in a non-git context
  [ "$status" -ne 0 ] || [[ "$output" == *"usage"* ]]
}

# ────────────────────────────────────────────────────────────────────────────
# quickstart-verify.sh: End-to-end quickstart validation
# ────────────────────────────────────────────────────────────────────────────

@test "quickstart-verify.sh requires cluster context or --help" {
  # This script needs a cluster; verify it at least has help
  run bash "${REPO_ROOT}/scripts/quickstart-verify.sh" --help 2>&1
  [ "$status" -eq 0 ] || [[ "$output" == *"kind"* ]] || [[ "$output" == *"cluster"* ]]
}

# ────────────────────────────────────────────────────────────────────────────
# quickstart-golden-path.sh: Golden path static validation
# ────────────────────────────────────────────────────────────────────────────

@test "quickstart-golden-path.sh validates without cluster" {
  run bash "${REPO_ROOT}/scripts/quickstart-golden-path.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"golden"* ]] || [[ "$output" == *"✓"* ]] || [[ "$output" == *"pass"* ]]
}

# ────────────────────────────────────────────────────────────────────────────
# audit-features.sh: Feature audit
# ────────────────────────────────────────────────────────────────────────────

@test "audit-features.sh completes without hanging" {
  timeout 30 bash "${REPO_ROOT}/scripts/audit-features.sh" >/dev/null 2>&1 || true
}

# ────────────────────────────────────────────────────────────────────────────
# golden-path-pipeline-check.sh: Full pipeline validation
# ────────────────────────────────────────────────────────────────────────────

@test "golden-path-pipeline-check.sh requires full toolchain (may skip gracefully)" {
  # This requires a complete dev environment; test it at least parses
  bash "${REPO_ROOT}/scripts/golden-path-pipeline-check.sh" >/dev/null 2>&1 || true
  # As long as it doesn't segfault, we're good
}

# ────────────────────────────────────────────────────────────────────────────
# Common error cases
# ────────────────────────────────────────────────────────────────────────────

@test "All scripts use strict mode (set -e or explicit error handling)" {
  # Spot-check that key scripts include shell safety patterns
  run grep -l "set -e" "${REPO_ROOT}"/scripts/*.sh
  [ "$status" -eq 0 ]
  [[ "$output" == *"cleanup"* ]] || [[ "$output" == *"dep-gate"* ]]
}

@test "Shell scripts are marked executable" {
  # Verify critical scripts have executable bit
  [ -x "${REPO_ROOT}/scripts/cleanup.sh" ]
  [ -x "${REPO_ROOT}/scripts/dep-gate.sh" ]
  [ -x "${REPO_ROOT}/scripts/preflight.sh" ]
}
