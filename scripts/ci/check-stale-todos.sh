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
set -euo pipefail

echo "========================================"
echo "Checking for Stale TODO/FIXME References"
echo "========================================"

# Directories considered critical paths
CRITICAL_PATHS=(
  ".github/"
  "scripts/"
  "charts/"
  "src/"
)

# Paths that document the TODO policy itself (or generate issues) and would
# otherwise self-match on the words TODO/FIXME.
EXCLUDE_REGEX='(^scripts/ci/check-stale-todos\.sh$)'

ERRORS=0

echo "Scanning critical paths: ${CRITICAL_PATHS[*]}"

for dir in "${CRITICAL_PATHS[@]}"; do
  if [ ! -d "$dir" ]; then
    continue
  fi

  while IFS= read -r match; do
    if [ -z "$match" ]; then continue; fi
    # match format: file:line:content
    file=$(echo "$match" | cut -d':' -f1)
    line=$(echo "$match" | cut -d':' -f2)
    content=$(echo "$match" | cut -d':' -f3-)

    if echo "$file" | grep -E -q "$EXCLUDE_REGEX"; then
      continue
    fi

    # Valid scopes: TODO(#123), TODO(@user), TODO(exempt: reason)
    if ! echo "$content" | grep -E -q '\b(TODO|FIXME)\((#[0-9]+|@[a-zA-Z0-9_-]+|exempt:[^)]+)\)'; then
      echo "::error file=$file,line=$line::Stale or improperly formatted TODO/FIXME found. Use TODO(#[issue]), TODO(@[username]), or TODO(exempt: [reason])."
      echo "  Line: $content"
      ERRORS=$((ERRORS + 1))
    fi

  done < <(grep -rnE '\b(TODO|FIXME)\b' "$dir" \
             --exclude='check-stale-todos.sh' \
             --exclude-dir='.git' \
             --exclude-dir='archive' \
             || true)

done

if [ "$ERRORS" -gt 0 ]; then
  echo "Found $ERRORS stale TODO/FIXME references."
  exit 1
fi

echo "All TODO/FIXME references in critical paths are properly documented."
exit 0
