#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: generate-release-notes.sh [--output FILE] [--config FILE] [--latest] [--strip-header]

Generates release notes using git-cliff and writes them to the requested output file.
EOF
}

output_file=""
config_file="cliff.toml"
latest=false
strip_header=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      output_file="$2"
      shift 2
      ;;
    --config)
      config_file="$2"
      shift 2
      ;;
    --latest)
      latest=true
      shift
      ;;
    --strip-header)
      strip_header=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$output_file" ]]; then
  output_file="changelog.md"
fi

if ! command -v git-cliff >/dev/null 2>&1; then
  if command -v cargo >/dev/null 2>&1; then
    echo "Installing git-cliff via cargo..."
    cargo install git-cliff --locked
  else
    echo "git-cliff is required. Install it or ensure it is available on PATH." >&2
    exit 1
  fi
fi

args=(--config "$config_file" --output "$output_file")
if [[ "$latest" == true ]]; then
  args+=(--latest)
fi
if [[ "$strip_header" == true ]]; then
  args+=(--strip header)
fi

git-cliff "${args[@]}"
