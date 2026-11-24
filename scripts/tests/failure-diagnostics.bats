#!/usr/bin/env bats
# failure-diagnostics.bats — unit tests for the unified CI diagnostics collector (#1151)

setup() {
  REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
  SCRIPT="${REPO_ROOT}/scripts/ci/collect-failure-diagnostics.sh"
  BUNDLE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ci-diag-XXXXXX")"
  EXTRA_FILE="$(mktemp "${TMPDIR:-/tmp}/ci-extra-XXXXXX.log")"
  echo "sample triage line" >"$EXTRA_FILE"
}

teardown() {
  rm -rf "$BUNDLE_DIR" "$EXTRA_FILE"
}

@test "collector assembles required bundle layout without cluster" {
  run bash "$SCRIPT" \
    --no-cluster \
    --bundle-dir "$BUNDLE_DIR" \
    --job-name "bats-test" \
    --run-id "42" \
    --sha "deadbeef" \
    --extra "$EXTRA_FILE"

  [ "$status" -eq 0 ]
  [ -f "$BUNDLE_DIR/manifest.json" ]
  [ -f "$BUNDLE_DIR/summary.txt" ]
  [ -f "$BUNDLE_DIR/env/sanitized.env" ]
  [ -f "$BUNDLE_DIR/cluster/SKIPPED.txt" ]
  [ -d "$BUNDLE_DIR/extras" ]

  grep -q 'stellar-k8s.ci-diagnostics/v1' "$BUNDLE_DIR/manifest.json"
  grep -q '"issue": 1151' "$BUNDLE_DIR/manifest.json"
  grep -q 'bats-test' "$BUNDLE_DIR/manifest.json"
  grep -q 'deadbeef' "$BUNDLE_DIR/manifest.json"
  grep -q 'Stellar-K8s CI Failure Diagnostics Bundle' "$BUNDLE_DIR/summary.txt"
}

@test "collector copies extra files into extras/" {
  run bash "$SCRIPT" \
    --no-cluster \
    --bundle-dir "$BUNDLE_DIR" \
    --extra "$EXTRA_FILE"

  [ "$status" -eq 0 ]
  # Exactly one copied extra should exist
  count="$(find "$BUNDLE_DIR/extras" -type f | wc -l | tr -d ' ')"
  [ "$count" -eq 1 ]
  grep -q 'sample triage line' "$BUNDLE_DIR/extras"/* 
}

@test "collector rejects unknown flags with usage exit" {
  run bash "$SCRIPT" --not-a-real-flag
  [ "$status" -eq 2 ]
}

@test "sanitized env omits secret-like keys" {
  export FAKE_SECRET_VALUE="should-not-leak"
  export GITHUB_TOKEN="ghs_should-not-leak"
  export SAFE_DIAG_VAR="ok-to-keep"

  run bash "$SCRIPT" --no-cluster --bundle-dir "$BUNDLE_DIR"
  [ "$status" -eq 0 ]

  ! grep -q 'FAKE_SECRET_VALUE' "$BUNDLE_DIR/env/sanitized.env"
  ! grep -q 'GITHUB_TOKEN' "$BUNDLE_DIR/env/sanitized.env"
  grep -q 'SAFE_DIAG_VAR' "$BUNDLE_DIR/env/sanitized.env"
}
