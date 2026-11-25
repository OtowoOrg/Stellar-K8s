#!/usr/bin/env bats
#
# bats suite for scripts/repo-health.sh
#
# Uses `--fast` so only cargo-backed steps run, and shims cargo via PATH to
# keep the suite hermetic (no toolchain, network, or cluster required).

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/repo-health.sh"

  TEST_TMP="$(mktemp -d)"
  SHIM_BIN="$TEST_TMP/bin"
  mkdir -p "$SHIM_BIN"
  export PATH="$SHIM_BIN:$PATH"
}

teardown() {
  rm -rf "$TEST_TMP"
}

# fail_fmt=1 makes the `cargo fmt --all --check` step fail.
make_cargo_shim() {
  local fail_fmt="${1:-0}"
  cat >"$SHIM_BIN/cargo" <<EOF
#!/usr/bin/env bash
if [[ "${fail_fmt}" == "1" && "\$1" == "fmt" ]]; then
  echo "shim: fmt check failed" >&2
  exit 1
fi
exit 0
EOF
  chmod +x "$SHIM_BIN/cargo"
}

@test "script exists and is executable" {
  [ -f "$SCRIPT" ]
  [ -x "$SCRIPT" ]
}

@test "happy path: --fast succeeds when every cargo step passes" {
  make_cargo_shim 0

  run "$SCRIPT" --fast
  [ "$status" -eq 0 ]
  [[ "$output" == *"Stellar-K8s repository health check (fast)"* ]]
  [[ "$output" == *"Format check (cargo fmt --all --check) passed"* ]]
  [[ "$output" == *"All repository health checks passed (3/3)"* ]]
}

@test "failure path: a failing fmt step exits non-zero with a clear error" {
  make_cargo_shim 1

  run "$SCRIPT" --fast
  [ "$status" -ne 0 ]
  [[ "$output" == *"Code is not formatted"* ]]
  [[ "$output" == *"Run 'make fmt' to auto-format Rust sources."* ]]
}
