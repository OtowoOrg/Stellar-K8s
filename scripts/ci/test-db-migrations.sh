#!/usr/bin/env bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
# Automated database migration tests (issue #1317).
#
# Provisions nothing itself — CI (or the developer) must provide DATABASE_URL
# pointing at an isolated Postgres instance. Safe test credentials only.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "ERROR: DATABASE_URL is required."
  echo "Example: postgres://stellar:stellar_test@127.0.0.1:5432/stellar_migration_test"
  exit 1
fi

echo "→ Loading bundled SQL migrations from db/migrations"
mapfile -t UP_FILES < <(ls -1 db/migrations/*.up.sql)
if [[ ${#UP_FILES[@]} -lt 1 ]]; then
  echo "ERROR: no up migrations found"
  exit 1
fi

# ---------------------------------------------------------------------------
# Migration safety gates (issue #1986).
#
# Each gate writes a JUnit-style report so results surface in existing PR
# checks with no new UI. Any violation fails the pipeline before the harness
# runs. Gates reuse cached production schema snapshots to stay under 60s.
# ---------------------------------------------------------------------------
GATE_REPORT_DIR="${MIGRATION_GATE_REPORT_DIR:-${ROOT}/target/migration-gates}"
SCHEMA_SNAPSHOT_DIR="${SCHEMA_SNAPSHOT_DIR:-${ROOT}/target/schema-snapshots}"
mkdir -p "$GATE_REPORT_DIR" "$SCHEMA_SNAPSHOT_DIR"

GATE_FAILED=0

# write_junit_report <report-file> <suite-name> <case-name> <status> <message>
# status is one of: pass | fail | skip
write_junit_report() {
  local report_file="$1" suite="$2" case_name="$3" status="$4" message="${5:-}"
  local failures=0 errors=0 skipped=0
  case "$status" in
    fail) failures=1 ;;
    skip) skipped=1 ;;
  esac
  {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<testsuites name="migration-safety-gates" tests="1" failures="%d" errors="%d" skipped="%d">\n' \
      "$failures" "$errors" "$skipped"
    printf '  <testsuite name="%s" tests="1" failures="%d" errors="%d" skipped="%d">\n' \
      "$suite" "$failures" "$errors" "$skipped"
    printf '    <testcase classname="%s" name="%s">\n' "$suite" "$case_name"
    if [[ "$status" == "fail" ]]; then
      printf '      <failure message="%s">%s</failure>\n' "$message" "$message"
    elif [[ "$status" == "skip" ]]; then
      printf '      <skipped message="%s"/>\n' "$message"
    fi
    printf '    </testcase>\n'
    printf '  </testsuite>\n'
    printf '</testsuites>\n'
  } >"$report_file"
}

# run_gate <gate-name> <report-file> <command...>
run_gate() {
  local gate="$1" report_file="$2"
  shift 2
  echo "→ Migration safety gate: ${gate}"
  local output
  if output="$("$@" 2>&1)"; then
    echo "  ✓ ${gate} passed"
    write_junit_report "$report_file" "migration-gate.${gate}" "$gate" pass
  else
    local rc=$?
    echo "  ✗ ${gate} FAILED (exit ${rc})"
    echo "$output" | sed 's/^/    /'
    write_junit_report "$report_file" "migration-gate.${gate}" "$gate" fail \
      "Migration safety gate '${gate}' failed (exit ${rc})"
    GATE_FAILED=1
  fi
}

# Locate the gate runner. Prefer the compiled binary, fall back to cargo run.
GATE_BIN="${MIGRATION_GATE_BIN:-}"
if [[ -z "$GATE_BIN" ]]; then
  for candidate in \
    "${ROOT}/target/release/migration-safety-gates" \
    "${ROOT}/target/debug/migration-safety-gates"; do
    if [[ -x "$candidate" ]]; then
      GATE_BIN="$candidate"
      break
    fi
  done
fi

gate_cmd() {
  if [[ -n "$GATE_BIN" ]]; then
    echo "$GATE_BIN"
  else
    echo "cargo run --quiet --bin migration-safety-gates --"
  fi
}

# Reuse cached production schema snapshot when available; refresh otherwise.
SNAPSHOT_FILE="${SCHEMA_SNAPSHOT_DIR}/production-schema.sql"
if [[ ! -s "$SNAPSHOT_FILE" ]]; then
  echo "→ No cached production schema snapshot; capturing from live schema"
  if ! pg_dump --schema-only --no-owner --no-privileges "$DATABASE_URL" >"$SNAPSHOT_FILE" 2>/dev/null; then
    echo "  ! Unable to capture live schema; compatibility gate will use repo baseline"
    rm -f "$SNAPSHOT_FILE"
  fi
else
  echo "→ Reusing cached production schema snapshot: $SNAPSHOT_FILE"
fi

GATE_COMMON_ARGS=(--migrations-dir db/migrations --report-dir "$GATE_REPORT_DIR")
if [[ -s "$SNAPSHOT_FILE" ]]; then
  GATE_COMMON_ARGS+=(--production-schema "$SNAPSHOT_FILE")
fi

# shellcheck disable=SC2206
GATE_CMD_ARR=($(gate_cmd))

run_gate "lock-analysis" \
  "${GATE_REPORT_DIR}/lock-analysis.xml" \
  "${GATE_CMD_ARR[@]}" lock-analysis "${GATE_COMMON_ARGS[@]}"

run_gate "backward-compatibility" \
  "${GATE_REPORT_DIR}/backward-compatibility.xml" \
  "${GATE_CMD_ARR[@]}" compatibility "${GATE_COMMON_ARGS[@]}"

run_gate "rollback-plan" \
  "${GATE_REPORT_DIR}/rollback-plan.xml" \
  "${GATE_CMD_ARR[@]}" rollback "${GATE_COMMON_ARGS[@]}"

if [[ "$GATE_FAILED" -ne 0 ]]; then
  echo "ERROR: one or more migration safety gates failed; blocking deploy."
  echo "Reports written to: $GATE_REPORT_DIR"
  exit 1
fi

echo "✓ Migration safety gates passed (reports: $GATE_REPORT_DIR)"

echo "→ Running Rust migration harness (fresh + existing-data + rollback)"
export DATABASE_URL
export STELLAR_MIGRATION_TEST=1
cargo test --test db_migration_harness --features rest-api,metrics,admission-webhook,k8s-v1-30 -- --nocapture

echo "✓ Database migration tests passed"
