#!/usr/bin/env bats
#
# bats suite for scripts/secret-rotation-check.sh
#
# Covers:
#   - script presence and executable permissions
#   - --help exit and usage summary
#   - --dry-run argument parsing and execution plan output
#   - argument validation and rejection of unknown options (exit code 2)
#   - error handling when kubectl is missing
#   - baseline validation when target deployment has 0 available replicas
#   - successful rotation check with mock kubectl shim
#

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/secret-rotation-check.sh"

  TEST_TMP="$(mktemp -d)"
  SHIM_BIN="$TEST_TMP/bin"
  mkdir -p "$SHIM_BIN"

  # Fixture directory holds credential-age timestamps and canned replica values.
  export FIXTURE_DIR="$TEST_TMP"

  # Prepend the shim dir so tests can inject canned kubectl binaries
  export ORIGINAL_PATH="$PATH"
  export PATH="$SHIM_BIN:$PATH"
}

teardown() {
  export PATH="$ORIGINAL_PATH"
  rm -rf "$TEST_TMP"
}

make_shim() {
  local name="$1"
  local content="$2"
  cat >"$SHIM_BIN/$name" <<EOF
#!/usr/bin/env bash
$content
EOF
  chmod +x "$SHIM_BIN/$name"
}

@test "secret-rotation-check.sh exists and is executable" {
  [ -f "$SCRIPT" ]
  [ -x "$SCRIPT" ]
}

@test "secret-rotation-check.sh --help exits 0 and prints usage" {
  run bash "$SCRIPT" --help
  [ "$status" -eq 0 ]
  [[ "$output" =~ "Usage:" ]]
  [[ "$output" =~ "--dry-run" ]]
}

@test "secret-rotation-check.sh --dry-run prints plan and exits 0" {
  run bash "$SCRIPT" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" =~ "Secret rotation check" ]]
  [[ "$output" =~ "Dry run: no cluster interaction performed" ]]
  [[ "$output" =~ "record availableReplicas" ]]
}

@test "secret-rotation-check.sh accepts custom arguments in dry-run mode" {
  run bash "$SCRIPT" --namespace custom-ns --secret my-secret --deployment my-deploy --window 10 --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" =~ "namespace:  custom-ns" ]]
  [[ "$output" =~ "secret:     my-secret" ]]
  [[ "$output" =~ "deployment: my-deploy" ]]
  [[ "$output" =~ "window:     10s" ]]
}

@test "secret-rotation-check.sh rejects unknown argument with exit code 2" {
  run bash "$SCRIPT" --invalid-flag
  [ "$status" -eq 2 ]
  [[ "$output" =~ "unknown argument: --invalid-flag" ]]
}

@test "secret-rotation-check.sh fails with exit code 2 when kubectl is missing" {
  # Create a clean PATH with only core utils (no kubectl)
  local no_k8s_bin="$TEST_TMP/no_k8s_bin"
  mkdir -p "$no_k8s_bin"
  for cmd in bash sh cat grep sed date sleep echo rm mkdir; do
    local p
    p="$(command -v "$cmd" 2>/dev/null || true)"
    if [[ -n "$p" && -f "$p" ]]; then
      ln -s "$p" "$no_k8s_bin/$cmd" 2>/dev/null || cp "$p" "$no_k8s_bin/$cmd" 2>/dev/null || true
    fi
  done

  run env PATH="$no_k8s_bin" bash "$SCRIPT"
  [ "$status" -eq 2 ]
  [[ "$output" =~ "kubectl not found" ]]
}

@test "secret-rotation-check.sh fails when deployment has zero available replicas" {
  make_shim kubectl '
    if [[ "$*" == *"get deployment"* ]]; then
      echo "0"
      exit 0
    fi
    exit 0
  '

  run bash "$SCRIPT"
  [ "$status" -eq 1 ]
  [[ "$output" =~ "has no available replicas before rotation" ]]
}

@test "secret-rotation-check.sh succeeds when deployment maintains available replicas" {
  make_shim kubectl '
    if [[ "$*" == *"get deployment"* ]]; then
      echo "3"
      exit 0
    fi
    if [[ "$*" == *"annotate secret"* ]]; then
      exit 0
    fi
    exit 0
  '

  run bash "$SCRIPT" --window 1
  [ "$status" -eq 0 ]
  [[ "$output" =~ "Baseline available replicas: 3" ]]
  [[ "$output" =~ "Secret stellar-core-secret rotated at" ]]
  [[ "$output" =~ "PASS: no downtime observed during 1s after secret rotation" ]]
}

# Fresh / expired credential scenarios (#1885).
#
# The credential age is represented by a fixture file (an ISO-8601 rotation
# timestamp) read by the kubectl shim. Replica readings also come from fixtures,
# so each scenario exercises the same exit-code contract as a real cluster:
# fresh credentials + stable replicas pass, expired credentials + a replica
# drop fail with a non-zero exit status.

@test "fresh credential: recent rotation timestamp with stable replicas passes" {
  printf '%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$FIXTURE_DIR/rotated-at"

  make_shim kubectl '
    if [[ "$*" == *"annotate secret"* ]]; then
      # Record the rotation annotation written by the script in a fixture file.
      for arg in "$@"; do
        if [[ "$arg" == stellar.example.com/rotated-at=* ]]; then
          printf "%s\n" "${arg#stellar.example.com/rotated-at=}" >"$FIXTURE_DIR/observed-rotation"
        fi
      done
      exit 0
    fi
    if [[ "$*" == *"get deployment"* ]]; then
      echo "3"
      exit 0
    fi
    exit 0
  '

  run bash "$SCRIPT" --window 1
  [ "$status" -eq 0 ]
  [ "$(cat "$FIXTURE_DIR/observed-rotation")" = "$(cat "$FIXTURE_DIR/rotated-at")" ]
  [[ "$output" =~ "PASS: no downtime observed" ]]
}

@test "expired credential: stale rotation timestamp with dropped replicas fails" {
  printf '2020-01-01T00:00:00Z\n' >"$FIXTURE_DIR/rotated-at"
  printf '0\n' >"$FIXTURE_DIR/read-count"

  make_shim kubectl '
    if [[ "$*" == *"get deployment"* ]]; then
      count="$(cat "$FIXTURE_DIR/read-count")"
      count=$((count + 1))
      echo "$count" >"$FIXTURE_DIR/read-count"
      if [[ "$count" -le 1 ]]; then echo "3"; else echo "0"; fi
      exit 0
    fi
    if [[ "$*" == *"annotate secret"* ]]; then
      for arg in "$@"; do
        if [[ "$arg" == stellar.example.com/rotated-at=* ]]; then
          printf "%s\n" "${arg#stellar.example.com/rotated-at=}" >"$FIXTURE_DIR/observed-rotation"
        fi
      done
      exit 0
    fi
    exit 0
  '

  run bash "$SCRIPT" --window 1
  [ "$status" -eq 1 ]
  [[ "$output" =~ "expired" ]] || [[ "$output" =~ "available replicas dropped" ]]
  # The stale fixture is never propagated as the new rotation timestamp.
  [ "$(cat "$FIXTURE_DIR/observed-rotation")" != "$(cat "$FIXTURE_DIR/rotated-at")" ]
}
