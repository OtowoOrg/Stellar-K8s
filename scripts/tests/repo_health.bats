#!/usr/bin/env bats
# scripts/tests/repo_health.bats — Regression tests for scripts/repo-health.sh
#                                  and scripts/lib/errors.sh.
#
# Run:  bats scripts/tests/repo_health.bats
# Requires: bats-core (https://github.com/bats-core/bats-core)

REPO_HEALTH="${BATS_TEST_DIRNAME}/../repo-health.sh"
ERRORS_LIB="${BATS_TEST_DIRNAME}/../lib/errors.sh"

# ---------------------------------------------------------------------------
# Helper: build a stub directory with controlled cargo exit codes.
# python3 and shellcheck are NOT added, so optional steps are skipped.
# Usage: _make_stubs <dir> <fmt_exit> <clippy_exit> <test_exit>
# ---------------------------------------------------------------------------
_make_stubs() {
  local stub_dir="$1" fmt_exit="$2" clippy_exit="$3" test_exit="$4"
  mkdir -p "${stub_dir}"
  cat > "${stub_dir}/cargo" <<EOF
#!/usr/bin/env bash
case "\$1" in
  fmt)    exit ${fmt_exit} ;;
  clippy) exit ${clippy_exit} ;;
  test)   exit ${test_exit} ;;
  *)      exit 0 ;;
esac
EOF
  chmod +x "${stub_dir}/cargo"
}

# ---------------------------------------------------------------------------
# errors.sh library — unit tests
# ---------------------------------------------------------------------------

@test "sk8s_step prints step name and description" {
  run bash -c "source '${ERRORS_LIB}'; sk8s_step 'my-step' 'doing something'"
  [ "$status" -eq 0 ]
  [[ "$output" == *"[my-step]"* ]]
  [[ "$output" == *"doing something"* ]]
}

@test "sk8s_fail exits non-zero and includes the current step name" {
  run bash -c "source '${ERRORS_LIB}'; sk8s_step 'build' ''; sk8s_fail 'compilation failed'"
  [ "$status" -ne 0 ]
  [[ "$output" == *"ERROR [build]"* ]]
  [[ "$output" == *"compilation failed"* ]]
}

@test "sk8s_fail prints the optional hint when provided" {
  run bash -c "source '${ERRORS_LIB}'; sk8s_fail 'something broke' 'run make fix'"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Hint: run make fix"* ]]
}

@test "sk8s_fail omits the hint line when no hint is given" {
  run bash -c "source '${ERRORS_LIB}'; sk8s_fail 'something broke'"
  [ "$status" -ne 0 ]
  [[ "$output" != *"Hint:"* ]]
}

@test "sk8s_warn prints message but does not exit" {
  run bash -c "source '${ERRORS_LIB}'; sk8s_warn 'something looks odd'; echo 'still running'"
  [ "$status" -eq 0 ]
  [[ "$output" == *"WARN"* ]]
  [[ "$output" == *"something looks odd"* ]]
  [[ "$output" == *"still running"* ]]
}

@test "sk8s_step updates the step name used by subsequent sk8s_fail" {
  run bash -c "
    source '${ERRORS_LIB}'
    sk8s_step 'step-one' 'first'
    sk8s_step 'step-two' 'second'
    sk8s_fail 'oops'
  "
  [ "$status" -ne 0 ]
  [[ "$output" == *"ERROR [step-two]"* ]]
}

# ---------------------------------------------------------------------------
# repo-health.sh — happy path
# ---------------------------------------------------------------------------

@test "repo-health exits 0 when all required steps pass" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 0 0

  # Use stub_dir-only PATH so optional steps (python3, shellcheck) are skipped
  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"All repository health checks passed"* ]]

  rm -rf "${stub_dir}"
}

@test "repo-health prints numbered step progress" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 0 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"[1/"* ]]
  [[ "$output" == *"[2/"* ]]
  [[ "$output" == *"[3/"* ]]

  rm -rf "${stub_dir}"
}

@test "repo-health prints the repo root in its header" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 0 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"repo:"* ]]

  rm -rf "${stub_dir}"
}

# ---------------------------------------------------------------------------
# repo-health.sh — step failures
# ---------------------------------------------------------------------------

@test "repo-health exits non-zero when fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]

  rm -rf "${stub_dir}"
}

@test "repo-health reports FAILED when fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"FAILED"* ]]

  rm -rf "${stub_dir}"
}

@test "repo-health prints a fix hint when fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"make fmt"* ]]

  rm -rf "${stub_dir}"
}

@test "repo-health exits non-zero when lint fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 1 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]

  rm -rf "${stub_dir}"
}

@test "repo-health exits non-zero when tests fail" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 0 1

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]

  rm -rf "${stub_dir}"
}

@test "repo-health stops at step 1 and does not reach step 2 when fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]
  # Step 2 header must not appear after step 1 failure
  [[ "$output" != *"[2/"* ]]

  rm -rf "${stub_dir}"
}

@test "repo-health stops at step 2 and does not reach step 3 when lint fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 1 0

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -ne 0 ]
  [[ "$output" != *"[3/"* ]]

  rm -rf "${stub_dir}"
}

# ---------------------------------------------------------------------------
# repo-health.sh — optional steps skipped gracefully
# ---------------------------------------------------------------------------

@test "repo-health skips python3 docs check and still passes when python3 absent" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 0 0
  # python3 is intentionally absent from stub_dir

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"python3"* ]]

  rm -rf "${stub_dir}"
}

@test "repo-health skips shellcheck and still passes when shellcheck absent" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_stubs "${stub_dir}" 0 0 0
  # shellcheck is intentionally absent from stub_dir

  run env PATH="${stub_dir}" bash "${REPO_HEALTH}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"shellcheck"* ]]

  rm -rf "${stub_dir}"
}
