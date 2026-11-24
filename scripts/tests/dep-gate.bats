#!/usr/bin/env bats
#
# Bats coverage for scripts/dep-gate.sh
#
# Exercises the allow and deny decision paths of the dependency gate.
# Package manager invocations are stubbed via PATH shims so the suite
# never touches the network or a real package manager.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  DEP_GATE="$REPO_ROOT/scripts/dep-gate.sh"

  # Isolated workspace so the gate never mutates the real checkout.
  WORKDIR="$(mktemp -d)"
  cd "$WORKDIR" || return 1

  # PATH shim directory: stub out package managers the gate may call.
  SHIM_DIR="$WORKDIR/shims"
  mkdir -p "$SHIM_DIR"
  for pm in npm yarn pnpm bun pip pip3 poetry cargo go; do
    cat >"$SHIM_DIR/$pm" <<'SHIM'
#!/usr/bin/env bash
# Stub package manager: record the call and succeed without network access.
echo "stub:$0 $*" >>"${DEP_GATE_STUB_LOG:-/dev/null}"
exit 0
SHIM
    chmod +x "$SHIM_DIR/$pm"
  done

  export PATH="$SHIM_DIR:$PATH"
  export DEP_GATE_STUB_LOG="$WORKDIR/pm-calls.log"
  : >"$DEP_GATE_STUB_LOG"
}

teardown() {
  if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    rm -rf "$WORKDIR"
  fi
}

@test "dep-gate.sh exists and is executable" {
  [ -f "$DEP_GATE" ]
  [ -x "$DEP_GATE" ]
}

# dep-gate.sh takes a mode, not an allow list: `--audit-only` runs just
# `cargo audit`, which the cargo shim answers without touching the network.

@test "allow path: gate passes when cargo audit succeeds" {
  run bash "$DEP_GATE" --audit-only

  [ "$status" -eq 0 ]
  [[ "$output" == *"All lockfile gate checks passed"* ]]
}

@test "deny path: gate fails when cargo audit reports vulnerabilities" {
  # Replace the cargo shim with one whose `audit` subcommand fails.
  cat >"$SHIM_DIR/cargo" <<'SHIM'
#!/usr/bin/env bash
echo "stub:$0 $*" >>"${DEP_GATE_STUB_LOG:-/dev/null}"
[ "$1" = "audit" ] && exit 1
exit 0
SHIM
  chmod +x "$SHIM_DIR/cargo"

  run bash "$DEP_GATE" --audit-only

  [ "$status" -ne 0 ]
  [[ "$output" == *"cargo audit found unignored vulnerabilities"* ]]
}

@test "package manager calls are stubbed via PATH" {
  run bash "$DEP_GATE" --audit-only

  grep -q 'stub:.*cargo audit' "$DEP_GATE_STUB_LOG"
}
