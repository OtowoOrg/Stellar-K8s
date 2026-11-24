#!/usr/bin/env bats
# Unit tests for scripts/cleanup.sh — the single supported cleanup tool.

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  CLEANUP="${REPO_ROOT}/scripts/cleanup.sh"
  TEST_DIR="$(mktemp -d)"
  export TEST_DIR
}

teardown() {
  rm -rf "${TEST_DIR}"
}

@test "cleanup.sh --help exits 0 and mentions single cleanup tool" {
  run bash "${CLEANUP}" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"Single supported repository cleanup tool"* ]]
}

@test "cleanup.sh --dry-run succeeds on a clean tree" {
  run bash "${CLEANUP}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"ok: absent scripts/archive"* ]]
  [[ "$output" == *"ok: absent scripts/lib/batch.sh"* ]]
  [[ "$output" == *"Supported cleanup entrypoint"* ]]
}

@test "cleanup.sh removes root scratch artifacts" {
  local scratch="${TEST_DIR}/scratch-repo"
  mkdir -p "${scratch}/scripts/lib"
  cp "${REPO_ROOT}/scripts/cleanup.sh" "${scratch}/scripts/cleanup.sh"
  cp "${REPO_ROOT}/scripts/lib/errors.sh" "${scratch}/scripts/lib/errors.sh"
  touch "${scratch}/log.txt" "${scratch}/check.log"

  run bash -c "cd '${scratch}' && bash scripts/cleanup.sh"
  [ "$status" -eq 0 ]
  [ ! -f "${scratch}/log.txt" ]
  [ ! -f "${scratch}/check.log" ]
}

@test "cleanup.sh fails when obsolete archive path is present" {
  local scratch="${TEST_DIR}/with-archive"
  mkdir -p "${scratch}/scripts/lib" "${scratch}/scripts/archive"
  cp "${REPO_ROOT}/scripts/cleanup.sh" "${scratch}/scripts/cleanup.sh"
  cp "${REPO_ROOT}/scripts/lib/errors.sh" "${scratch}/scripts/lib/errors.sh"
  touch "${scratch}/scripts/archive/create_batch_2_issues.sh"

  run bash -c "cd '${scratch}' && bash scripts/cleanup.sh"
  [ "$status" -ne 0 ]
  [[ "$output" == *"obsolete path still present: scripts/archive"* ]]
}
