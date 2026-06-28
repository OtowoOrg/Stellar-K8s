#!/usr/bin/env bash
# Generate changelog/release notes using a single git-cliff entry point.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

OUTPUT_FILE="CHANGELOG.md"
LATEST_ONLY=false
STRIP_MODE=""

usage() {
  cat <<'EOF'
Usage: bash scripts/generate-changelog.sh [options]

Options:
  --output <path>      Output file path (default: CHANGELOG.md)
  --latest             Generate notes for latest release range only
  --strip <mode>       Pass through to git-cliff --strip (header|footer|all)
  -h, --help           Show this help message

Examples:
  bash scripts/generate-changelog.sh
  bash scripts/generate-changelog.sh --latest --strip header --output release-notes.md
EOF
}

while (($#)); do
  case "$1" in
    --output)
      OUTPUT_FILE="${2:-}"
      shift 2
      ;;
    --latest)
      LATEST_ONLY=true
      shift
      ;;
    --strip)
      STRIP_MODE="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v git-cliff >/dev/null 2>&1; then
  echo "git-cliff is required. Install with: cargo install git-cliff"
  exit 1
fi

ARGS=(--config cliff.toml --output "${OUTPUT_FILE}")

if [[ "${LATEST_ONLY}" == "true" ]]; then
  ARGS+=(--latest)
fi

if [[ -n "${STRIP_MODE}" ]]; then
  ARGS+=(--strip "${STRIP_MODE}")
fi

echo "Generating changelog to ${OUTPUT_FILE}..."
git-cliff "${ARGS[@]}"
echo "Changelog generated: ${OUTPUT_FILE}"
