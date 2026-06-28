#!/usr/bin/env bats

setup() {
  export REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
  export SCRIPT_DIR="${BATS_TEST_DIRNAME}/.."
  # shellcheck source=scripts/lib/errors.sh
  source "${SCRIPT_DIR}/lib/errors.sh"
  # shellcheck source=scripts/lib/health-steps.sh
  source "${SCRIPT_DIR}/lib/health-steps.sh"
}

@test "health-steps defines shared clippy deny rules" {
  [[ "${#SK8S_CLIPPY_DENY[@]}" -ge 4 ]]
  [[ " ${SK8S_CLIPPY_DENY[*]} " == *" clippy::correctness "* ]]
}

@test "health-steps defines CI cargo features" {
  [[ "${SK8S_CARGO_FEATURES}" == *"rest-api"* ]]
  [[ "${SK8S_CARGO_FEATURES}" == *"k8s-v1-30"* ]]
}

@test "repo-health.sh --help exits zero" {
  run bash "${SCRIPT_DIR}/repo-health.sh" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"--fast"* ]]
}

@test "validate.sh delegates to repo-health --fast" {
  run bash "${SCRIPT_DIR}/validate.sh" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"--fast"* ]]
}

@test "repo-health.sh rejects unknown flags" {
  run bash "${SCRIPT_DIR}/repo-health.sh" --not-a-real-flag
  [ "$status" -eq 2 ]
  [[ "$output" == *"Unknown option"* ]]
}
