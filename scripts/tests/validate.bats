#!/usr/bin/env bats
# scripts/tests/validate.bats — Regression tests for scripts/validate.sh
#
# Run:  bats scripts/tests/validate.bats
# Requires: bats-core (https://github.com/bats-core/bats-core)

VALIDATE="${BATS_TEST_DIRNAME}/../validate.sh"

# ---------------------------------------------------------------------------
# Helper: write a cargo stub that controls per-subcommand exit codes.
# Usage: _make_cargo_stub <dir> <fmt_exit> <clippy_exit> <test_exit>
# ---------------------------------------------------------------------------
_make_cargo_stub() {
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
# All-pass
# ---------------------------------------------------------------------------

@test "validate exits 0 when all steps pass" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 0 0 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Validation complete"* ]]

  rm -rf "${stub_dir}"
}

@test "validate prints a step header for each phase" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 0 0 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"format check"* ]]
  [[ "$output" == *"lint"* ]]
  [[ "$output" == *"compile check"* ]]

  rm -rf "${stub_dir}"
}

# ---------------------------------------------------------------------------
# Format check failures
# ---------------------------------------------------------------------------

@test "validate exits non-zero when cargo fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]

  rm -rf "${stub_dir}"
}

@test "validate prints an error when fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"ERROR"* ]]

  rm -rf "${stub_dir}"
}

@test "validate does not reach lint step when fmt fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 1 0 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]
  # The lint step header must not appear after an fmt failure
  [[ "$output" != *"--> [lint]"* ]]

  rm -rf "${stub_dir}"
}

# ---------------------------------------------------------------------------
# Lint failures
# ---------------------------------------------------------------------------

@test "validate exits non-zero when clippy fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 0 1 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]

  rm -rf "${stub_dir}"
}

@test "validate does not reach compile check when lint fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 0 1 0

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]
  # compile check header must not appear
  [[ "$output" != *"--> [compile check]"* ]]

  rm -rf "${stub_dir}"
}

# ---------------------------------------------------------------------------
# Compile check failures
# ---------------------------------------------------------------------------

@test "validate exits non-zero when compile check fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 0 0 1

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]

  rm -rf "${stub_dir}"
}

@test "validate prints an error when compile check fails" {
  local stub_dir
  stub_dir=$(mktemp -d)
  _make_cargo_stub "${stub_dir}" 0 0 1

  run env PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"ERROR"* ]]

  rm -rf "${stub_dir}"
}

# ---------------------------------------------------------------------------
# Environment variable propagation
# ---------------------------------------------------------------------------

@test "validate defaults K8S_OPENAPI_ENABLED_VERSION to 1.30" {
  local stub_dir
  stub_dir=$(mktemp -d)
  # Stub cargo echoes the env var so we can assert it was set
  cat > "${stub_dir}/cargo" <<'EOF'
#!/usr/bin/env bash
echo "K8S_OPENAPI_VERSION=${K8S_OPENAPI_ENABLED_VERSION}"
exit 0
EOF
  chmod +x "${stub_dir}/cargo"

  run env -u K8S_OPENAPI_ENABLED_VERSION PATH="${stub_dir}:${PATH}" bash "${VALIDATE}"
  [[ "$output" == *"K8S_OPENAPI_VERSION=1.30"* ]]

  rm -rf "${stub_dir}"
}

@test "validate honours a caller-supplied K8S_OPENAPI_ENABLED_VERSION" {
  local stub_dir
  stub_dir=$(mktemp -d)
  cat > "${stub_dir}/cargo" <<'EOF'
#!/usr/bin/env bash
echo "K8S_OPENAPI_VERSION=${K8S_OPENAPI_ENABLED_VERSION}"
exit 0
EOF
  chmod +x "${stub_dir}/cargo"

  run env PATH="${stub_dir}:${PATH}" K8S_OPENAPI_ENABLED_VERSION=1.29 bash "${VALIDATE}"
  [[ "$output" == *"K8S_OPENAPI_VERSION=1.29"* ]]

  rm -rf "${stub_dir}"
}
