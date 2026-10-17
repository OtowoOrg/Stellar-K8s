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
# scripts/generate-release-notes.sh — Preview the GitHub Release notes.
#
# Generates the same changelog excerpt the `release` job publishes as the
# GitHub Release body (see `.github/workflows/release.yml`: git-cliff with
# `--latest --strip header`). Run it before tagging to review exactly what
# reviewers will see on the release page.
#
# Usage:
#   bash scripts/generate-release-notes.sh                  # print to stdout
#   bash scripts/generate-release-notes.sh --output NOTES.md # write to a file
#
# Requires: git-cliff (https://github.com/orhun/git-cliff)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}" || exit 1

OUTPUT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      OUTPUT="${2:?--output requires a value}"
      shift 2
      ;;
    -h|--help)
      sed -n '14,24p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1 (see --help)" >&2
      exit 1
      ;;
  esac
done

if ! command -v git-cliff >/dev/null 2>&1; then
  echo "error: git-cliff not found." >&2
  echo "Install it first: https://github.com/orhun/git-cliff (or: cargo install git-cliff)" >&2
  exit 1
fi

# Same excerpt the release workflow publishes: latest section only, no header.
if [[ -n "${OUTPUT}" ]]; then
  git cliff --latest --strip header --output "${OUTPUT}"
  echo "Release notes written to ${OUTPUT}"
else
  git cliff --latest --strip header
fi
