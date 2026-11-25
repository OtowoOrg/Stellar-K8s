#!/usr/bin/env bats
# scripts/tests/repo-health.bats — Tests for scripts/repo-health.sh
#
# Run:  bats scripts/tests/repo-health.bats
# Requires: bats-core (https://github.com/bats-core/bats-core)

REPO_HEALTH="${BATS_TEST_DIRNAME}/../repo-health.sh"

@test "repo-health.sh --help exits 0 and prints usage" {
  run "${REPO_HEALTH}" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"Usage:"* ]]
  [[ "$output" == *"--fast"* ]]
}

@test "repo-health.sh -h exits 0 and prints usage" {
  run "${REPO_HEALTH}" -h
  [ "$status" -eq 0 ]
  [[ "$output" == *"Usage:"* ]]
}

@test "repo-health.sh exits non-zero on unknown option" {
  run "${REPO_HEALTH}" --unknown-flag
  [ "$status" -ne 0 ]
  [ "$status" -eq 2 ]
  [[ "$output" == *"Unknown option: --unknown-flag"* ]]
}

@test "repo-health.sh --fast succeeds when cargo checks pass" {
  local tmpdir
  tmpdir="$(mktemp -d)"
  printf '#!/usr/bin/env bash\nexit 0\n' > "${tmpdir}/cargo"
  chmod +x "${tmpdir}/cargo"

  PATH="${tmpdir}:${PATH}" run "${REPO_HEALTH}" --fast
  rm -rf "${tmpdir}"

  [ "$status" -eq 0 ]
  [[ "$output" == *"Repository health checks passed"* ]]
}

@test "repo-health.sh --fast exits non-zero when cargo check fails" {
  local tmpdir
  tmpdir="$(mktemp -d)"
  printf '#!/usr/bin/env bash\nexit 1\n' > "${tmpdir}/cargo"
  chmod +x "${tmpdir}/cargo"

  PATH="${tmpdir}:${PATH}" run "${REPO_HEALTH}" --fast
  rm -rf "${tmpdir}"

  [ "$status" -ne 0 ]
}
