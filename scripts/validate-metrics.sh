#!/usr/bin/env bash
# Cross-check metric names registered in metrics.rs against metric-reference.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
METRICS_RS="${ROOT}/src/controller/metrics.rs"
METRIC_DOC="${ROOT}/docs/observability/metric-reference.md"

if [[ ! -f "$METRICS_RS" ]]; then
  echo "error: missing $METRICS_RS" >&2
  exit 1
fi
if [[ ! -f "$METRIC_DOC" ]]; then
  echo "error: missing $METRIC_DOC" >&2
  exit 1
fi

mapfile -t REGISTERED < <(
  grep -A1 'registry.register(' "$METRICS_RS" \
    | grep -E '^\s+"' \
    | sed -E 's/^[[:space:]]*"([^"]+)".*/\1/' \
    | sort -u
)

missing=0
for name in "${REGISTERED[@]}"; do
  if ! grep -qF "$name" "$METRIC_DOC"; then
    echo "missing in metric-reference.md: $name" >&2
    missing=1
  fi
done

extra=0
while IFS= read -r metric; do
  [[ -z "$metric" ]] && continue
  if ! printf '%s\n' "${REGISTERED[@]}" | grep -qxF "$metric"; then
    echo "documented but not registered: $metric" >&2
    extra=1
  fi
done < <(
  grep -E '^\| `[a-z0-9_]+`' "$METRIC_DOC" \
    | sed -n 's/^| `\([^`]*\)`.*/\1/p' \
    | sort -u
)

count="${#REGISTERED[@]}"
echo "Validated $count registered metrics against $METRIC_DOC"

if [[ "$missing" -ne 0 || "$extra" -ne 0 ]]; then
  exit 1
fi

echo "OK: metric reference is complete"
