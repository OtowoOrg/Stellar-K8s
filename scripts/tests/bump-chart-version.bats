#!/usr/bin/env bats
# scripts/tests/bump-chart-version.bats — Tests for scripts/bump-chart-version.sh (#1319)
#
# The bump script analyses git commit history (conventional commits) since either
# the last chart-v* tag or an explicit --since ref, then determines a SemVer bump:
#   major — breaking change (feat!/BREAKING CHANGE:)
#   minor — feat:
#   patch — fix/perf/refactor/revert
#   none  — docs/chore/ci/test/style/build only
#
# It also honours the `versioning.min-bump` annotation in Chart.yaml as a minimum
# bump floor (patch/minor/major), promoting a too-small bump up to the floor.
#
# Run:  bats scripts/tests/bump-chart-version.bats
# Requires: bats-core (https://github.com/bats-core/bats-core)

BUMP="${BATS_TEST_DIRNAME}/../bump-chart-version.sh"

# The bump script runs git (tag/log) against the current working directory, so
# every invocation is executed from inside the throwaway repo.
run_bump() {
  run bash -c "cd '${WORK_DIR}' && bash '${BUMP}' \"\$@\"" -- "$@"
}

setup() {
  # Build a throwaway git repo so tests control the commit history without
  # touching the real repository's tags or Chart.yaml.
  WORK_DIR="$(mktemp -d)"
  export WORK_DIR
  git -C "${WORK_DIR}" init -q
  git -C "${WORK_DIR}" config user.email test@example.com
  git -C "${WORK_DIR}" config user.name test
  CHART_DIR="${WORK_DIR}/charts/stellar-operator"
  mkdir -p "${CHART_DIR}"
}

teardown() {
  rm -rf "${WORK_DIR}"
}

# Write a Chart.yaml at the given version inside the temp repo.
_make_chart() {
  local version="$1"
  local min_bump="${2:-}"
  cat > "${CHART_DIR}/Chart.yaml" <<EOF
apiVersion: v2
name: stellar-operator
description: test chart
type: application
version: ${version}
appVersion: "${version}"
annotations:${min_bump:+"
  versioning.min-bump: ${min_bump}"}
  oci.registry: ghcr.io
EOF
}

# Commit a conventional-commit message and return to a clean tree.
_commit() {
  local subject="$1"
  echo "change" >> "${CHART_DIR}/values.yaml"
  git -C "${WORK_DIR}" add -A
  git -C "${WORK_DIR}" commit -q -m "${subject}"
}

# ---------------------------------------------------------------------------
# Version bump rules (analyzed from the repo root, no chart tag present)
# ---------------------------------------------------------------------------

@test "a breaking change (feat!) bumps the MAJOR version" {
  _make_chart "1.0.0"
  _commit "feat!: remove deprecated API"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: major"* ]]
  [[ "$output" == *"new version: 2.0.0"* ]]
}

@test "a BREAKING CHANGE footer bumps the MAJOR version" {
  _make_chart "1.2.3"
  echo "change" >> "${CHART_DIR}/values.yaml"
  git -C "${WORK_DIR}" add -A
  git -C "${WORK_DIR}" commit -q -m "$(printf 'fix: tighten access rules\n\nBREAKING CHANGE: requires new CRD')"

  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: major"* ]]
  [[ "$output" == *"new version: 2.0.0"* ]]
}

@test "a feat commit bumps the MINOR version" {
  _make_chart "1.0.0"
  _commit "feat(webhook): add metrics endpoint"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: minor"* ]]
  [[ "$output" == *"new version: 1.1.0"* ]]
}

@test "a fix commit bumps the PATCH version" {
  _make_chart "1.2.0"
  _commit "fix(controller): nil deref on delete"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: patch"* ]]
  [[ "$output" == *"new version: 1.2.1"* ]]
}

@test "docs/chore/ci commits do not bump the version" {
  _make_chart "1.0.0"
  _commit "docs: clarify installation"
  _commit "chore: tidy deps"
  _commit "ci: speed up build"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: none"* ]]
}

@test "a feat outranks a fix (minor beats patch)" {
  _make_chart "1.0.0"
  _commit "fix: patch a bug"
  _commit "feat: add new field"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: minor"* ]]
  [[ "$output" == *"new version: 1.1.0"* ]]
}

# ---------------------------------------------------------------------------
# First commit is always picked up (no chart tag present)
# ---------------------------------------------------------------------------

@test "a single initial feat commit is released (root commit not excluded)" {
  _make_chart "0.1.0"
  _commit "feat: initial operator support"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: minor"* ]]
  [[ "$output" == *"new version: 0.2.0"* ]]
}

# ---------------------------------------------------------------------------
# versioning.min-bump floor
# ---------------------------------------------------------------------------

@test "min-bump: patch forces a patch release for docs-only commits" {
  _make_chart "1.0.0" "patch"
  _commit "docs: clarify installation"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: patch"* ]]
  [[ "$output" == *"new version: 1.0.1"* ]]
}

@test "min-bump: minor forces a minor release for fix-only commits" {
  _make_chart "1.0.0" "minor"
  _commit "fix: patch a bug"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: minor"* ]]
  [[ "$output" == *"new version: 1.1.0"* ]]
}

@test "min-bump does not override a higher bump type" {
  _make_chart "1.0.0" "patch"
  _commit "feat: add new feature"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: minor"* ]]
  [[ "$output" == *"new version: 1.1.0"* ]]
}

@test "min-bump none does not force a release for docs-only commits" {
  _make_chart "1.0.0" "none"
  _commit "docs: tweak wording"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: none"* ]]
  [[ "$output" == *"new version: 1.0.0"* ]]
}

# ---------------------------------------------------------------------------
# Changelog generation
# ---------------------------------------------------------------------------

@test "changelog entry lists conventional commit subjects" {
  _make_chart "1.0.0"
  _commit "feat: new alpha feature"
  _commit "fix: patch bug"
  run_bump --chart-path "${CHART_DIR}" --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"feat: new alpha feature"* ]]
  [[ "$output" == *"fix: patch bug"* ]]
}

# ---------------------------------------------------------------------------
# --bump-override
# ---------------------------------------------------------------------------

@test "--bump-override forces a bump type" {
  _make_chart "1.0.0"
  _commit "chore: no-op chore"
  run_bump --chart-path "${CHART_DIR}" --dry-run --bump-override minor
  [ "$status" -eq 0 ]
  [[ "$output" == *"Bump type: minor"* ]]
  [[ "$output" == *"new version: 1.1.0"* ]]
}

# ---------------------------------------------------------------------------
# --output-env (GitHub Actions mode) and real writes
# ---------------------------------------------------------------------------

@test "a bump writes the new version to Chart.yaml (non dry-run)" {
  _make_chart "1.4.0"
  _commit "feat: ship new feature"
  run_bump --chart-path "${CHART_DIR}"
  [ "$status" -eq 0 ]
  run grep -E '^version:' "${CHART_DIR}/Chart.yaml"
  [[ "$output" == *"1.5.0"* ]]
  # appVersion is kept in sync.
  run grep -E '^appVersion:' "${CHART_DIR}/Chart.yaml"
  [[ "$output" == *'"1.5.0"'* ]]
}

@test "--output-env emits GitHub Actions multiline outputs" {
  _make_chart "1.0.0"
  _commit "feat: new thing"
  run bash -c "cd '${WORK_DIR}' && GITHUB_OUTPUT='${WORK_DIR}/out.txt' bash '${BUMP}' --chart-path '${CHART_DIR}' --dry-run --output-env"
  [ "$status" -eq 0 ]
  run cat "${WORK_DIR}/out.txt"
  [[ "$output" == *"bump_type=minor"* ]]
  [[ "$output" == *"new_version=1.1.0"* ]]
  [[ "$output" == *"has_changes=true"* ]]
}
