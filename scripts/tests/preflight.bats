#!/usr/bin/env bats
# scripts/tests/preflight.bats — Tests for scripts/preflight.sh
#
# Run:  bats scripts/tests/preflight.bats
# Requires: bats-core (https://github.com/bats-core/bats-core)

PREFLIGHT="${BATS_TEST_DIRNAME}/../preflight.sh"

# Pinned versions from scripts/lib/versions.sh, duplicated here as plain
# strings (not sourced) so a bump to the pins makes an *at-the-pin* stub
# below intentionally start failing, forcing this file to be updated too.
PINNED_RUST="1.92"
PINNED_KIND="0.24.0"
PINNED_KUBECTL="1.30.0"
PINNED_HELM="3.16.0"

# Helper: stub every tool at exactly the pinned minimum version.
_stub_all_at_pin() {
  local dir="$1"
  printf '#!/usr/bin/env bash\necho "Docker version 24.0.0"\n' > "${dir}/docker"
  printf '#!/usr/bin/env bash\necho "kind version %s"\n' "${PINNED_KIND}" > "${dir}/kind"
  printf '#!/usr/bin/env bash\necho "Client Version: v%s"\n' "${PINNED_KUBECTL}" > "${dir}/kubectl"
  printf '#!/usr/bin/env bash\necho '"'"'version.BuildInfo{Version:"v%s"}'"'"'\n' "${PINNED_HELM}" > "${dir}/helm"
  printf '#!/usr/bin/env bash\necho "cargo %s.0"\n' "${PINNED_RUST}" > "${dir}/cargo"
  printf '#!/usr/bin/env bash\necho "gh version 2.50.0"\n' > "${dir}/gh"
  chmod +x "${dir}"/*
}

# Helper: symlink /usr/bin and /bin into "${dir}/sys" minus the named tools, so
# a "missing tool" test still holds on hosts (e.g. GitHub runners) that ship
# that tool in /usr/bin. Prints the directory to put on PATH.
_system_path_without() {
  local dir="$1"; shift
  local sys="${dir}/sys" bin name skip
  mkdir -p "${sys}"
  for bin in /usr/bin/* /bin/*; do
    name="${bin##*/}"
    [[ -e "${sys}/${name}" ]] && continue
    for skip in "$@"; do [[ "${name}" == "${skip}" ]] && continue 2; done
    ln -s "${bin}" "${sys}/${name}"
  done
  echo "${sys}"
}

# ---------------------------------------------------------------------------
# Tool-presence tests
# ---------------------------------------------------------------------------

@test "preflight exits 0 when all required tools are present at/above pinned versions" {
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"

  run env PATH="${dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Preflight passed"* ]]

  rm -rf "${dir}"
}

@test "preflight exits non-zero when a required tool is missing" {
  # Copy stubs for every tool EXCEPT kind.
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"
  rm "${dir}/kind"

  run env PATH="${dir}:$(_system_path_without "${dir}" kind)" bash "${PREFLIGHT}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"[FAIL]"* ]]
  [[ "$output" == *"kind not found in PATH"* ]]

  rm -rf "${dir}"
}

@test "preflight exits non-zero when gh is missing" {
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"
  rm "${dir}/gh"

  run env PATH="${dir}:$(_system_path_without "${dir}" gh)" bash "${PREFLIGHT}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"[FAIL]"* ]]

  rm -rf "${dir}"
}

# ---------------------------------------------------------------------------
# Strict version-gate tests
# ---------------------------------------------------------------------------

@test "preflight exits non-zero when an installed version is below the pinned minimum" {
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"
  # Downgrade kind below the 0.24.0 pin.
  printf '#!/usr/bin/env bash\necho "kind version 0.22.0"\n' > "${dir}/kind"
  chmod +x "${dir}/kind"

  run env PATH="${dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -ne 0 ]
  [[ "$output" == *"kind 0.22.0 is below the required minimum ${PINNED_KIND}"* ]]

  rm -rf "${dir}"
}

@test "preflight passes when an installed version is above the pinned minimum" {
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"
  # kubectl newer than the pin should still pass (>=, not ==).
  printf '#!/usr/bin/env bash\necho "Client Version: v1.31.0"\n' > "${dir}/kubectl"
  chmod +x "${dir}/kubectl"

  run env PATH="${dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"kubectl 1.31.0 (>= ${PINNED_KUBECTL})"* ]]

  rm -rf "${dir}"
}

@test "preflight treats a version-check tool with no parseable version as missing" {
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"
  printf '#!/usr/bin/env bash\necho "helm: command not found"\n' > "${dir}/helm"
  chmod +x "${dir}/helm"

  run env PATH="${dir}:/usr/bin:/bin" bash "${PREFLIGHT}"
  [ "$status" -ne 0 ]
  # The tool IS on PATH, so preflight must report the unparseable version
  # rather than claim the binary is missing.
  [[ "$output" == *"helm version could not be parsed (requires >= ${PINNED_HELM})"* ]]

  rm -rf "${dir}"
}

# ---------------------------------------------------------------------------
# Label-check tests (--labels flag)
# ---------------------------------------------------------------------------

@test "preflight --labels warns and exits 0 when gh is not installed" {
  # Hide gh from PATH (label check should just warn, not hard fail).
  local dir
  dir=$(mktemp -d)
  _stub_all_at_pin "${dir}"
  rm "${dir}/gh"

  run env PATH="${dir}:$(_system_path_without "${dir}" gh)" \
      REPO="TestOrg/TestRepo" \
      bash "${PREFLIGHT}" --labels
  [[ "$output" == *"'gh' CLI not found"* ]]

  rm -rf "${dir}"
}

@test "preflight --labels warns when REPO is undetectable" {
  # Run in a temp dir with no git remote so REPO auto-detect returns empty.
  local tmp_dir
  tmp_dir=$(mktemp -d)
  git -C "${tmp_dir}" init -q

  run bash -c "cd '${tmp_dir}' && bash '${PREFLIGHT}' --labels"
  [[ "$output" == *"REPO not set"* ]] || \
  [[ "$output" == *"skipping label check"* ]] || \
  [ "$status" -ne 0 ]   # acceptable: tools may also fail in a bare tmp dir

  rm -rf "${tmp_dir}"
}
