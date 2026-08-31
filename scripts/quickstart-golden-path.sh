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
# Golden-path quickstart validation (issue #1067).
#
# Validates that the documented quickstart path is intact without needing a
# cluster: entry points exist, quickstart scripts parse, and the manifests a
# new user applies are valid YAML. Run locally or from CI:
#
#   ./scripts/quickstart-golden-path.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FAILURES=0

step() { echo; echo "==> $1"; }
fail() { echo "FAIL: $1" >&2; FAILURES=$((FAILURES + 1)); }

step "Quickstart entry points exist"
for f in scripts/quickstart-verify.sh README.md Makefile; do
  if [[ -e "${REPO_ROOT}/${f}" ]]; then
    echo "ok: ${f}"
  else
    fail "missing quickstart entry point: ${f}"
  fi
done

step "Quickstart scripts parse (bash -n)"
for f in scripts/quickstart-verify.sh scripts/preflight.sh scripts/secret-rotation-check.sh; do
  if [[ -e "${REPO_ROOT}/${f}" ]]; then
    if bash -n "${REPO_ROOT}/${f}"; then
      echo "ok: ${f}"
    else
      fail "syntax error in ${f}"
    fi
  fi
done

step "Golden-path manifests are valid YAML (config/crd, config/samples)"
if python3 -c "import yaml" 2>/dev/null; then
  while IFS= read -r manifest; do
    if python3 -c "import sys, yaml; list(yaml.safe_load_all(open(sys.argv[1])))" "${manifest}"; then
      echo "ok: ${manifest#"${REPO_ROOT}"/}"
    else
      fail "invalid YAML: ${manifest#"${REPO_ROOT}"/}"
    fi
  done < <(find "${REPO_ROOT}/config/crd" "${REPO_ROOT}/config/samples" -name '*.yaml' 2>/dev/null | sort)
else
  echo "warn: PyYAML not available, skipping manifest validation"
fi

step "README documents the quickstart"
if grep -qiE "quick ?start" "${REPO_ROOT}/README.md"; then
  echo "ok: README contains a quickstart section"
else
  fail "README.md has no quickstart section"
fi

echo
if [[ "${FAILURES}" -gt 0 ]]; then
  echo "Golden-path validation failed with ${FAILURES} error(s)." >&2
  exit 1
fi
echo "Golden-path validation passed."
