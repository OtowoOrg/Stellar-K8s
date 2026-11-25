#!/usr/bin/env bats
# Tests for scripts/ci/check-readme-badges.sh (#1896).
# Run with: make test-bats  (or: bats scripts/tests/check-readme-badges.bats)

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  SCRIPT="${REPO_ROOT}/scripts/ci/check-readme-badges.sh"
  TEST_DIR="$(mktemp -d)"
  mkdir -p "${TEST_DIR}/workflows"
  touch "${TEST_DIR}/workflows/ci.yml" "${TEST_DIR}/workflows/release.yml"
  export WORKFLOWS_DIR="${TEST_DIR}/workflows"
}

teardown() {
  rm -rf "${TEST_DIR}"
}

write_readme() {
  printf '%s\n' "$@" > "${TEST_DIR}/README.md"
}

CI_BADGE='<a href="https://github.com/OtowoOrg/Stellar-K8s/actions/workflows/ci.yml"><img src="https://github.com/OtowoOrg/Stellar-K8s/actions/workflows/ci.yml/badge.svg" alt="CI Status" /></a>'

@test "passes on the repository README" {
  unset WORKFLOWS_DIR
  run bash "${SCRIPT}"
  [ "$status" -eq 0 ]
  [[ "$output" == *"CI badge present"* ]]
  [[ "$output" == *"ci.yml"* ]]
}

@test "passes when every badge points at an existing workflow" {
  write_readme "${CI_BADGE}" \
    '<img src="https://img.shields.io/github/actions/workflow/status/OtowoOrg/Stellar-K8s/release.yml?style=for-the-badge" />'
  run bash "${SCRIPT}" "${TEST_DIR}/README.md"
  [ "$status" -eq 0 ]
  [[ "$output" == *"release.yml"* ]]
}

@test "fails when the CI badge is missing" {
  write_readme '<img src="https://img.shields.io/github/actions/workflow/status/OtowoOrg/Stellar-K8s/release.yml" />'
  run bash "${SCRIPT}" "${TEST_DIR}/README.md"
  [ "$status" -eq 1 ]
  [[ "$output" == *"CI badge missing"* ]]
}

@test "fails when a shields.io badge points at a missing workflow" {
  write_readme "${CI_BADGE}" \
    '<img src="https://img.shields.io/github/actions/workflow/status/OtowoOrg/Stellar-K8s/removed.yml?branch=main" />'
  run bash "${SCRIPT}" "${TEST_DIR}/README.md"
  [ "$status" -eq 1 ]
  [[ "$output" == *"removed.yml is referenced in the README"* ]]
}

@test "fails when a workflow link points at a missing workflow" {
  write_readme "${CI_BADGE}" '[runs](https://github.com/OtowoOrg/Stellar-K8s/actions/workflows/old-ci.yaml)'
  run bash "${SCRIPT}" "${TEST_DIR}/README.md"
  [ "$status" -eq 1 ]
  [[ "$output" == *"old-ci.yaml"* ]]
}

@test "fails when the README does not exist" {
  run bash "${SCRIPT}" "${TEST_DIR}/missing.md"
  [ "$status" -eq 1 ]
  [[ "$output" == *"README not found"* ]]
}
