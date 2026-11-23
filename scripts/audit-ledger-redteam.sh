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
#
# Red-team drill for #1958: delete / modify / reorder one event in a generated
# ledger and assert that stellar-audit-verify pinpoints the exact sequence number.
#
# Usage: scripts/audit-ledger-redteam.sh [path-to-stellar-audit-verify] [entries]
set -euo pipefail

BIN="${1:-target/release/stellar-audit-verify}"
N="${2:-1000}"
VICTIM=$((N / 2))                      # sequence number the attacker targets
DIR="$(mktemp -d)"
trap 'rm -rf "$DIR"' EXIT

"$BIN" generate --out "$DIR/ledger.ndjson" --chain-id redteam --count "$N" >/dev/null
"$BIN" verify "$DIR/ledger.ndjson" >/dev/null || { echo "baseline ledger must verify"; exit 1; }

# Line 1 is the header, so seq S is on line S+2.
LINE=$((VICTIM + 2))
fail=0

expect() { # name, file, expected-kind, expected-seq
  local out rc
  set +e; out="$("$BIN" verify "$2" --json)"; rc=$?; set -e
  local kind seq
  kind="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["violation"]["kind"])')"
  seq="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["violation"]["seq"])')"
  if [[ $rc -eq 1 && "$kind" == "$3" && "$seq" == "$4" ]]; then
    echo "PASS  $1 -> $kind at seq $seq"
  else
    echo "FAIL  $1 -> rc=$rc kind=$kind seq=$seq (wanted $3 at $4)"; fail=1
  fi
}

sed "${LINE}d" "$DIR/ledger.ndjson" > "$DIR/deleted.ndjson"
expect "delete event $VICTIM" "$DIR/deleted.ndjson" missing_entries "$VICTIM"

sed "${LINE}s/\"verb\":\"[a-z]*\"/\"verb\":\"get\"/" "$DIR/ledger.ndjson" > "$DIR/modified.ndjson"
expect "modify event $VICTIM" "$DIR/modified.ndjson" modified_entry "$VICTIM"

awk -v a="$LINE" -v b="$((LINE + 1))" 'NR==a{held=$0; next} NR==b{print; print held; next} {print}' \
  "$DIR/ledger.ndjson" > "$DIR/reordered.ndjson"
expect "reorder events $VICTIM/$((VICTIM + 1))" "$DIR/reordered.ndjson" reordered_entry "$VICTIM"

exit "$fail"
