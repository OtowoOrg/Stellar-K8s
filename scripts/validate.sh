#!/usr/bin/env bash
# scripts/validate.sh — Fast local validation (delegates to repo-health.sh --fast).
exec bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/repo-health.sh" --fast "$@"
