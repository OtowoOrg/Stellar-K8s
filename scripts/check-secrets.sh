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
# check-secrets.sh
#
# Secure secret-handling checks for all CI and runtime pipeline command paths.
# Closes Issue #1116.
#
# Checks performed
# ────────────────
#   1. Hard-coded secrets scan — grep source for patterns that look like real
#      credentials (Stellar seeds, bearer tokens, AWS keys, PEM blocks, etc.)
#      that were accidentally committed.
#   2. Env-var hygiene — confirm that secret values are never echoed via
#      `echo`, `printf`, or `set -x` in shell scripts.
#   3. GitHub Actions secret safety — ensure workflow files do not print
#      secret context values and use `mask` where required.
#   4. Dockerfile secret hygiene — ensure no ENV / ARG directives carry
#      secret names that would bake values into image layers.
#   5. Rust source secret hygiene — ensure no literal secret strings appear
#      in .rs files outside of test fixtures (which are clearly labelled).
#
# Exit codes
# ──────────
#   0  — No findings
#   1  — One or more findings (fails CI)
#
# Usage:
#   ./scripts/check-secrets.sh
#   ./scripts/check-secrets.sh --report   # report-only, always exit 0

set -euo pipefail

REPORT_ONLY=false
for arg in "$@"; do
    [[ "$arg" == "--report" ]] && REPORT_ONLY=true
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BOLD='\033[1m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
RESET='\033[0m'

separator() { echo -e "${BOLD}────────────────────────────────────────────────────────────────${RESET}"; }

findings=0
finding() {
    local file="$1" line="$2" desc="$3"
    echo -e "  ${RED}✗${RESET}  ${file}:${line} — ${desc}"
    findings=$((findings + 1))
}

# ── Check 1: Hard-coded credential patterns in source ─────────────────────────
separator
echo -e "${BOLD}Check 1 — Hard-coded credential patterns${RESET}"
separator

# Stellar seed: 'S' + 55 base58 chars (56 total).
# Exclude known test fixtures that carry safe-looking placeholder seeds.
while IFS=: read -r file lineno content; do
    # Skip files that are deliberately testing the scrubber / pipeline redaction.
    [[ "$file" == *log_scrub* || "$file" == *pipeline_log_redaction* || "$file" == *pipeline_logs* || "$file" == *tests* || "$file" == *test_* ]] && continue
    finding "$file" "$lineno" "Possible Stellar seed (S...56 chars)"
done < <(grep -rn --include="*.rs" --include="*.sh" --include="*.yaml" --include="*.yml" \
    -E "S[A-Za-z0-9]{54}[^A-Za-z0-9]" . \
    --exclude-dir=.git \
    --exclude-dir=target \
    | grep -v "# test\|// test\|FIXTURE\|placeholder\|EXAMPLE" \
    || true)

# AWS access key pattern: AKIA[A-Z0-9]{16}
while IFS=: read -r file lineno content; do
    [[ "$file" == *test* || "$file" == *example* || "$file" == *fixture* || "$file" == *sample* ]] && continue
    # Allow AWS documentation placeholders (…EXAMPLE).
    echo "$content" | grep -qiE 'EXAMPLE|PLACEHOLDER|YOUR[_-]?KEY' && continue
    finding "$file" "$lineno" "Possible AWS Access Key ID (AKIA...)"
done < <(grep -rn --include="*.rs" --include="*.sh" --include="*.yaml" --include="*.yml" \
    -E "AKIA[A-Z0-9]{16}" . \
    --exclude-dir=.git --exclude-dir=target \
    || true)

# PEM private key block
while IFS=: read -r file lineno _; do
    [[ "$file" == *test* || "$file" == *example* || "$file" == *fixture* || "$file" == *log_scrub* || "$file" == *pipeline_log_redaction* ]] && continue
    finding "$file" "$lineno" "PEM private key block committed to repository"
done < <(grep -rn --include="*.rs" --include="*.pem" --include="*.key" \
    "BEGIN.*PRIVATE KEY" . \
    --exclude-dir=.git --exclude-dir=target \
    || true)

# Generic password= / secret= / token= with non-placeholder values
while IFS=: read -r file lineno content; do
    [[ "$file" == *test* || "$file" == *example* || "$file" == *fixture* || "$file" == *sample* || "$file" == *secret_rotation* || "$file" == *secret-rotation* || "$file" == *check-secrets* ]] && continue
    # Allow obvious placeholders and Secret *resource names* (not credential values).
    echo "$content" | grep -qiE "(placeholder|example|changeme|your[-_]|<[^>]+>|\\\$\{|test[_-]?password|stellar-core-secret)" && continue
    finding "$file" "$lineno" "Possible inline secret assignment (password=/secret=/token=)"
done < <(grep -rni --include="*.rs" --include="*.sh" --include="*.yaml" --include="*.yml" \
    -E "(password|secret|token)\s*=\s*['\"][^'\"]{8,}" . \
    --exclude-dir=.git --exclude-dir=target \
    || true)

if [[ $findings -eq 0 ]]; then
    echo -e "${GREEN}✓  No hard-coded credential patterns found${RESET}"
fi

# ── Check 2: Shell script secret-echo hygiene ─────────────────────────────────
separator
echo -e "${BOLD}Check 2 — Shell script secret-echo hygiene${RESET}"
separator

shell_findings_before=$findings

# Look for echo / printf of environment variables whose names suggest secrets.
SECRET_VAR_PATTERN='(SECRET|PASSWORD|TOKEN|KEY|SEED|CERT|PRIVATE)'

while IFS=: read -r file lineno content; do
    # Allow lines that are clearly in comments.
    echo "$content" | grep -q '^\s*#' && continue
    finding "$file" "$lineno" "Possible echo of a secret env-var"
done < <(grep -rn --include="*.sh" \
    -E "(echo|printf)\s+[\"']?\\\$\{?${SECRET_VAR_PATTERN}" scripts/ .github/ \
    --exclude-dir=.git \
    || true)

# Detect live `set -x` / `set -o xtrace` (would leak secrets in CI logs).
# Skip this checker script itself (documents the pattern in comments) and
# comment-only mentions elsewhere.
while IFS=: read -r file lineno content; do
    [[ "$file" == *check-secrets.sh ]] && continue
    echo "$content" | grep -qE '^\s*#' && continue
    finding "$file" "$lineno" "'set -x' found in script — may leak secrets to CI logs"
done < <(grep -rn --include="*.sh" 'set -x\|set -o xtrace' scripts/ .github/ \
    --exclude-dir=.git \
    || true)

if [[ $findings -eq $shell_findings_before ]]; then
    echo -e "${GREEN}✓  No secret-echo issues in shell scripts${RESET}"
fi

# ── Check 3: GitHub Actions workflow secret safety ────────────────────────────
separator
echo -e "${BOLD}Check 3 — GitHub Actions secret hygiene${RESET}"
separator

workflow_findings_before=$findings

# Ensure secrets are accessed via ${{ secrets.X }} not injected into env directly
# and echoed unmasked.
while IFS=: read -r file lineno content; do
    # Ignore lines that are just comments or masked properly.
    echo "$content" | grep -q '::add-mask\|# safe\|# masked' && continue
    echo "$content" | grep -q 'echo.*secrets\.' || continue
    finding "$file" "$lineno" "Possible unmasked GitHub secret echoed in workflow"
done < <(grep -rn --include="*.yml" --include="*.yaml" \
    'echo.*\${{.*secrets\.' .github/workflows/ \
    || true)

# Check that workflow env blocks don't reference secrets as plain env vars
# in a way that could expose them in runner logs.
while IFS=: read -r file lineno content; do
    echo "$content" | grep -q '#.*safe\|# ok\|# intentional' && continue
    finding "$file" "$lineno" "Workflow env block injects a secret into a plain env var — consider masking"
done < <(grep -rn --include="*.yml" --include="*.yaml" \
    -A1 'env:' .github/workflows/ \
    | grep '\${{.*secrets\.' \
    | grep -v 'GITHUB_TOKEN\|CODECOV_TOKEN' \
    || true)

if [[ $findings -eq $workflow_findings_before ]]; then
    echo -e "${GREEN}✓  No workflow secret hygiene issues found${RESET}"
fi

# ── Check 4: Dockerfile secret hygiene ────────────────────────────────────────
separator
echo -e "${BOLD}Check 4 — Dockerfile secret hygiene${RESET}"
separator

dockerfile_findings_before=$findings

while IFS=: read -r file lineno content; do
    # RUN --mount=type=secret is the safe pattern — skip it.
    echo "$content" | grep -q 'mount=type=secret' && continue
    finding "$file" "$lineno" "Dockerfile ENV/ARG with secret-like name bakes value into image layer"
done < <(grep -rn --include="Dockerfile" --include="*.Dockerfile" \
    -E "^(ENV|ARG)\s+(SECRET|PASSWORD|TOKEN|KEY|SEED|PRIVATE)" . \
    --exclude-dir=.git \
    || true)

if [[ $findings -eq $dockerfile_findings_before ]]; then
    echo -e "${GREEN}✓  No Dockerfile secret hygiene issues found${RESET}"
fi

# ── Check 5: Rust source — no literal secret strings outside tests ─────────────
separator
echo -e "${BOLD}Check 5 — Rust source secret literals${RESET}"
separator

rust_findings_before=$findings

# Look for string literals that look like real Stellar seeds in non-test files.
while IFS=: read -r file lineno content; do
    # Allow test/fixture files and the scrubber / pipeline redaction checker itself.
    [[ "$file" == *test* || "$file" == *scrub* || "$file" == *fixture* || "$file" == *pipeline_log_redaction* ]] && continue
    echo "$content" | grep -q 'FIXTURE\|placeholder\|example' && continue
    finding "$file" "$lineno" "Possible Stellar seed literal in non-test Rust source"
done < <(grep -rn --include="*.rs" \
    -E '"S[A-Za-z0-9]{54}"' src/ \
    | grep -v '#\[cfg(test)\]\|mod tests\|test_' \
    || true)

if [[ $findings -eq $rust_findings_before ]]; then
    echo -e "${GREEN}✓  No secret literals in non-test Rust source${RESET}"
fi

# ── Summary ────────────────────────────────────────────────────────────────────
separator
echo -e "${BOLD}Secret-handling audit summary${RESET}"
separator

if [[ $findings -eq 0 ]]; then
    echo -e "${GREEN}✓  All secret-handling checks passed${RESET}"
    exit 0
else
    echo -e "${RED}✗  Total findings: ${findings}${RESET}"
    echo ""
    echo "  Remediation guidance:"
    echo "  • Move secrets to Kubernetes Secrets or a KMS backend."
    echo "  • Use \${{ secrets.MY_SECRET }} in GitHub Actions (never echo them)."
    echo "  • Add ::add-mask::<value> for dynamic secrets in CI."
    echo "  • Replace hard-coded seeds with environment-variable injection."
    echo "  • Use RUN --mount=type=secret in Dockerfiles instead of ENV/ARG."
    if $REPORT_ONLY; then
        echo ""
        echo "(--report mode: exiting 0)"
        exit 0
    fi
    exit 1
fi
