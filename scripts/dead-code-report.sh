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
# Generates a periodic dead-code and unused-config report (issue #1064).
#
# The report is informational: it always exits 0 and writes a Markdown
# summary to target/reports/dead-code-report.md so CI can upload it as an
# artifact. Set SKIP_CARGO=1 to skip the compiler pass (used for smoke tests).
# shell-safety: disable-file SH001 -- this report must survive a failing cargo pass
# and always exit 0, so `-e` is deliberately omitted from strict mode.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_DIR="${REPO_ROOT}/target/reports"
REPORT="${REPORT_DIR}/dead-code-report.md"
CONFIG_FILE="${REPO_ROOT}/config/operator-config.yaml"

mkdir -p "${REPORT_DIR}"

{
  echo "# Dead-code and unused-config report"
  echo
  echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
} > "${REPORT}"

echo "==> Collecting dead-code diagnostics"
{
  echo "## Dead code (rustc diagnostics)"
  echo
} >> "${REPORT}"

if [[ "${SKIP_CARGO:-0}" == "1" ]]; then
  echo "_Compiler pass skipped (SKIP_CARGO=1)._" >> "${REPORT}"
else
  DEAD_CODE="$(cd "${REPO_ROOT}" && cargo check --all-targets --message-format short 2>&1 \
    | grep -E "never used|never read|never constructed|dead_code" || true)"
  if [[ -n "${DEAD_CODE}" ]]; then
    {
      echo '```'
      echo "${DEAD_CODE}"
      echo '```'
    } >> "${REPORT}"
  else
    echo "_No dead-code diagnostics reported._" >> "${REPORT}"
  fi
fi

echo "==> Checking operator config keys for usage"
{
  echo
  echo "## Config keys not referenced in src/"
  echo
} >> "${REPORT}"

UNUSED_COUNT=0
if [[ -f "${CONFIG_FILE}" ]]; then
  # Top-level keys of the operator config, e.g. "reconcileInterval:".
  while IFS= read -r key; do
    if ! grep -rq -- "${key}" "${REPO_ROOT}/src" 2>/dev/null; then
      echo "- \`${key}\` (defined in config/operator-config.yaml, no reference found in src/)" >> "${REPORT}"
      UNUSED_COUNT=$((UNUSED_COUNT + 1))
    fi
  done < <(sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\):.*/\1/p' "${CONFIG_FILE}" | sort -u)
fi
if [[ "${UNUSED_COUNT}" -eq 0 ]]; then
  echo "_All top-level config keys are referenced._" >> "${REPORT}"
fi

echo "==> Report written to ${REPORT}"
exit 0
