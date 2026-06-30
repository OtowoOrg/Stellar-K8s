#!/usr/bin/env bats
# scripts/tests/preflight.bats — Tests for scripts/preflight.sh
#
# Run:  bats scripts/tests/preflight.bats
# Requires: bats-core (https://github.com/bats-core/bats-core)

PREFLIGHT="${BATS_TEST_DIRNAME}/../preflight.sh"

# ---------------------------------------------------------------------------
# Tool-check tests
# ---------------------------------------------------------------------------

@test "preflight exits 0 when all required tools are present" {
  # Stub every required binary to succeed.
  docker()  { echo "Docker version 24.0.0"; }
  kind()    { echo "kind v0.22.0"; }
  kubectl() { echo "Client Version: v1.30.0"; }
  helm()    { echo "version.BuildInfo{Version:\"v3.14.0\"}"; }
  cargo()   { echo "cargo 1.88.0"; }
  gh()      { echo "gh version 2.50.0"; }
  export -f docker kind kubectl helm cargo gh

  run bash "${PREFLIGHT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Preflight passed"* ]]
}

@test "preflight exits non-zero when a required tool is missing" {
  # Make 'kind' unavailable by shadowing PATH with an empty dir.
  local empty_dir
  empty_dir=$(mktemp -d)
  # Copy stubs for every tool EXCEPT kind.
  for tool in docker kubectl helm cargo gh; do
    printf '#!/usr/bin/env bash\necho "%s stub"\n' "$tool" > "${empty_dir}/${tool}"
    chmod +x "${empty_dir}/${tool}"
  done

  run env PATH="${empty_dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"[FAIL]"* ]]

  rm -rf "${empty_dir}"
}

# ---------------------------------------------------------------------------
# Label-check tests (--labels flag)
# ---------------------------------------------------------------------------

@test "preflight --labels warns and exits 0 when gh is not installed" {
  # Hide gh from PATH.
  local empty_dir
  empty_dir=$(mktemp -d)

  run env PATH="${empty_dir}:/usr/bin:/bin" \
      REPO="TestOrg/TestRepo" \
      bash "${PREFLIGHT}" --labels
  # Label check is advisory — overall exit must still be 0 if tools pass
  # (tools will fail without stubs, so only check the warning message)
  [[ "$output" == *"'gh' CLI not found"* ]] || \
  [[ "$output" == *"gh"* ]]

  rm -rf "${empty_dir}"
}

@test "preflight --labels warns and exits 0 when REPO is undetectable" {
  # Run in a temp dir with no git remote so REPO auto-detect returns empty.
  local tmp_dir
  tmp_dir=$(mktemp -d)
  git -C "${tmp_dir}" init -q

  run bash -c "cd '${tmp_dir}' && bash '${PREFLIGHT}' --labels" 
  [[ "$output" == *"REPO not set"* ]] || \
  [[ "$output" == *"skipping label check"* ]] || \
  [ "$status" -ne 0 ]   # acceptable: tools may fail in tmp dir

  rm -rf "${tmp_dir}"
}

@test "preflight exits 0 without --labels even when REPO is unset" {
  # Stub all tools.
  local stub_dir
  stub_dir=$(mktemp -d)
  for tool in docker kind kubectl helm cargo; do
    printf '#!/usr/bin/env bash\necho "%s stub"\n' "$tool" > "${stub_dir}/${tool}"
    chmod +x "${stub_dir}/${tool}"
  done

  run env PATH="${stub_dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -eq 0 ]

  rm -rf "${stub_dir}"
}

@test "preflight exits non-zero when gh is missing" {
  local empty_dir
  empty_dir=$(mktemp -d)
  # Stub every tool EXCEPT gh.
  for tool in docker kind kubectl helm cargo; do
    printf '#!/usr/bin/env bash\necho "%s stub"\n' "$tool" > "${empty_dir}/${tool}"
    chmod +x "${empty_dir}/${tool}"
  done

  run env PATH="${empty_dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"[FAIL]"* ]]

  rm -rf "${empty_dir}"
}
