#!/usr/bin/env bats
#
# bats suite for scripts/dead-code-report.sh
#
# Covers:
#   - report generation from canned `cargo check` dead-code output (PATH shims)
#   - empty-input handling producing a well-formed empty report
#

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/dead-code-report.sh"

  TEST_TMP="$(mktemp -d)"
  SHIM_BIN="$TEST_TMP/bin"
  mkdir -p "$SHIM_BIN"

  # Isolate the report output inside the temp dir.
  export DEAD_CODE_REPORT_OUT="$TEST_TMP/dead-code-report.md"

  # Prepend the shim dir so the script picks up our canned cargo tools.
  export PATH="$SHIM_BIN:$PATH"
}

teardown() {
  rm -rf "$TEST_TMP"
}

# Write a PATH shim that emits the given stdout and exits with the given code.
make_shim() {
  local name="$1"
  local output="$2"
  local code="${3:-0}"
  cat >"$SHIM_BIN/$name" <<EOF
#!/usr/bin/env bash
cat <<'SHIM_OUT'
$output
SHIM_OUT
exit $code
EOF
  chmod +x "$SHIM_BIN/$name"
}

@test "script exists and is executable" {
  [ -f "$SCRIPT" ]
  [ -x "$SCRIPT" ]
}

@test "generates a report from canned cargo check output" {
  # The script runs `cargo check --all-targets --message-format short` and
  # keeps lines matching its dead-code diagnostic patterns, so the shim has to
  # emit such a line rather than `cargo metadata` JSON.
  make_shim cargo "src/lib.rs:10:5: warning: constant \`DEMO_UNUSED\` is never used"

  run "$SCRIPT"
  [ "$status" -eq 0 ]

  [ -f "$DEAD_CODE_REPORT_OUT" ]
  [ -s "$DEAD_CODE_REPORT_OUT" ]

  # The report should surface the diagnostic the shim produced.
  run grep -q "DEMO_UNUSED" "$DEAD_CODE_REPORT_OUT"
  [ "$status" -eq 0 ]

  # Both report sections must be present, not just the diagnostic body.
  run grep -q "## Dead code (rustc diagnostics)" "$DEAD_CODE_REPORT_OUT"
  [ "$status" -eq 0 ]
  run grep -q "## Config keys not referenced in src/" "$DEAD_CODE_REPORT_OUT"
  [ "$status" -eq 0 ]
}

@test "empty tool output yields a well-formed empty report" {
  make_shim cargo ''

  run "$SCRIPT"
  [ "$status" -eq 0 ]

  [ -f "$DEAD_CODE_REPORT_OUT" ]

  # A well-formed report is still produced even with no findings.
  run grep -qi "dead code" "$DEAD_CODE_REPORT_OUT"
  [ "$status" -eq 0 ]

  # The empty-input path must say so explicitly rather than list findings.
  run grep -q "No dead-code diagnostics reported" "$DEAD_CODE_REPORT_OUT"
  [ "$status" -eq 0 ]

  run grep -q "DEMO_UNUSED" "$DEAD_CODE_REPORT_OUT"
  [ "$status" -ne 0 ]
}
