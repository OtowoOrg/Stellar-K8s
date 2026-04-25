#!/bin/bash
# Stellar-K8s Root Cleanup Script
# This script moves temporary artifacts, logs, and one-off test files to their appropriate locations.

set -e

echo "🧹 Starting repository cleanup..."

# Create target directories if they don't exist
mkdir -p docs/completed_tasks
mkdir -p docs/reports
mkdir -p scripts/dev-utils
mkdir -p tests/scratch

# 1. Move Completion Reports
REPORTS=(
    "IMPLEMENTATION_COMPLETE.md"
    "SOROBAN_DASHBOARD_COMPLETE.md"
    "WASM_WEBHOOK_COMPLETE.md"
    "WEBHOOK_BENCHMARK_COMPLETE.md"
    "manifest_validation_report.md"
)

for f in "${REPORTS[@]}"; do
    if [ -f "$f" ]; then
        mv "$f" docs/completed_tasks/
        echo "✅ Moved $f to docs/completed_tasks/"
    fi
done

# 2. Cleanup Logs & Temporary Files
LOGS=(
    "build_errors.txt"
    "cargo_check.log"
    "check.log"
    "gh_log.txt"
    "log.txt"
    "rendered-output.yaml"
)

for f in "${LOGS[@]}"; do
    if [ -f "$f" ]; then
        rm "$f"
        echo "🗑️  Removed $f"
    fi
done

# 3. Move Test/Dev Utilities
UTILS=(
    "test_encap.rs"
    "test_pqc.rs"
    "test-precommit.sh"
    "get_helm.sh"
    "starfield.html"
)

for f in "${UTILS[@]}"; do
    if [ -f "$f" ]; then
        mv "$f" scripts/dev-utils/
        echo "📦 Moved $f to scripts/dev-utils/"
    fi
done

echo "✨ Cleanup complete. Your root directory is now focused on core project files."
